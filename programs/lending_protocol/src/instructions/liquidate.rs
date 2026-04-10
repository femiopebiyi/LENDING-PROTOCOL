use anchor_lang::prelude::*;
use anchor_spl::{
    associated_token::AssociatedToken,
    token_interface::{transfer_checked, Mint, TokenAccount, TokenInterface, TransferChecked},
};

use crate::{
    constants::PRICE_SCALE,
    errors::LendingError,
    helpers::{accrue_interest, health_factor},
    state::{LendingPool, StubOracle, UserPosition},
};

#[derive(Accounts)]
pub struct Liquidate<'info> {
    #[account(mut)]
    pub liquidator: Signer<'info>,

    #[account(
        constraint = borrower.key() != liquidator.key() @ LendingError::CannotLiquidateSelf
    )]
    pub borrower: SystemAccount<'info>,

    #[account(
        mut,
        seeds = [b"lendingpool", pool.owner.as_ref(), pool.seed.to_le_bytes().as_ref()],
        bump = pool.bump
    )]
    pub pool: Account<'info, LendingPool>,

    #[account(
        mint::token_program = token_program,
    )]
    pub collateral_mint: InterfaceAccount<'info, Mint>,

    #[account(
        mint::token_program = token_program,
        address = pool.borrow_mint @LendingError::InvalidMint
    )]
    pub borrow_mint: InterfaceAccount<'info, Mint>,

    #[account(
        mut,
        associated_token::mint = collateral_mint,
        associated_token::authority = pool,
        associated_token::token_program = token_program,
    )]
    pub collateral_vault: InterfaceAccount<'info, TokenAccount>,

    #[account(
        mut,
        associated_token::mint = borrow_mint,
        associated_token::authority = pool,
        associated_token::token_program = token_program,
    )]
    pub borrow_vault: InterfaceAccount<'info, TokenAccount>,

    #[account(
        init_if_needed,
        payer = liquidator,
        associated_token::mint = collateral_mint,
        associated_token::authority = liquidator,
        associated_token::token_program = token_program,
    )]
    pub liquidator_ata_collateral: InterfaceAccount<'info, TokenAccount>,

    #[account(
        mut,
        associated_token::mint = borrow_mint,
        associated_token::authority = liquidator,
        associated_token::token_program = token_program,
    )]
    pub liquidator_ata_borrow: InterfaceAccount<'info, TokenAccount>,

    #[account(
        mut,
        seeds = [b"userposition", pool.key().as_ref(), borrower.key().as_ref()],
        bump = user_position.bump,
        constraint = user_position.owner.key() == borrower.key() @LendingError::CredentialMismatch
    )]
    pub user_position: Account<'info, UserPosition>,

    #[account(
        seeds = [b"stuboracle", oracle.authority.as_ref(), oracle.seed.to_le_bytes().as_ref()],
        bump = oracle.bump,
        constraint = oracle.key() == pool.oracle @ LendingError::InvalidOracle
    )]
    pub oracle: Account<'info, StubOracle>,

    pub associated_token_program: Program<'info, AssociatedToken>,
    pub token_program: Interface<'info, TokenInterface>,
    pub system_program: Program<'info, System>,
}

impl<'info> Liquidate<'info> {
    fn liquidate(&mut self, repay_amount: u64) -> Result<()> {
        require!(!self.pool.is_paused, LendingError::Paused);
        // 1. Validate
        require!(repay_amount > 0, LendingError::InvalidAmount);

        // 2. Settle interest on borrower's position
        let clock = Clock::get()?;
        accrue_interest(
            &mut self.user_position,
            clock.unix_timestamp,
            self.pool.interest_rate,
        )?;

        // 3. Verify position is unhealthy — reject if health >= 1.0
        let health = health_factor(&self.user_position, &self.pool, &self.oracle)?;
        require!(health < PRICE_SCALE, LendingError::PositionHealthy);

        // 4. Cap repay amount at total debt
        let total_debt = self
            .user_position
            .borrowed_amount
            .checked_add(self.user_position.interest_accrued)
            .ok_or(LendingError::Overflow)?;

        let repay_amount = repay_amount.min(total_debt);

        // 5. Calculate collateral to seize
        // convert repay amount to USD value then to collateral token amount
        let repay_value_usd = (repay_amount as u128)
            .checked_mul(PRICE_SCALE)
            .ok_or(LendingError::Overflow)?;

        let collateral_amount = repay_value_usd
            .checked_div(self.oracle.price as u128)
            .ok_or(LendingError::Overflow)?;

        // apply liquidation bonus — liquidator receives extra collateral as incentive
        let collateral_to_seize = collateral_amount
            .checked_mul(100 + self.pool.liquidation_bonus as u128)
            .ok_or(LendingError::Overflow)?
            .checked_div(100)
            .ok_or(LendingError::Overflow)? as u64;

        // cap at available collateral — can never seize more than exists
        let collateral_to_seize = collateral_to_seize.min(self.user_position.collateral_deposited);

        // 6. Split repayment — interest cleared first, remainder reduces principal
        let interest_payment = repay_amount.min(self.user_position.interest_accrued);
        let principal_payment = repay_amount
            .checked_sub(interest_payment)
            .ok_or(LendingError::Overflow)?;

        // 7. Build pool PDA signer seeds
        let seeds = &[
            b"lendingpool",
            self.pool.owner.as_ref(),
            &self.pool.seed.to_le_bytes(),
            &[self.pool.bump],
        ];
        let signer_seeds = &[&seeds[..]];

        // 8. Liquidator pays debt into borrow vault — liquidator signs
        transfer_checked(
            CpiContext::new(
                self.token_program.to_account_info(),
                TransferChecked {
                    from: self.liquidator_ata_borrow.to_account_info(),
                    to: self.borrow_vault.to_account_info(),
                    authority: self.liquidator.to_account_info(),
                    mint: self.borrow_mint.to_account_info(),
                },
            ),
            repay_amount,
            self.borrow_mint.decimals,
        )?;

        // 9. Collateral + bonus goes from vault to liquidator — pool PDA signs
        transfer_checked(
            CpiContext::new_with_signer(
                self.token_program.to_account_info(),
                TransferChecked {
                    from: self.collateral_vault.to_account_info(),
                    to: self.liquidator_ata_collateral.to_account_info(),
                    authority: self.pool.to_account_info(),
                    mint: self.collateral_mint.to_account_info(),
                },
                signer_seeds,
            ),
            collateral_to_seize,
            self.collateral_mint.decimals,
        )?;

        // 10. Update borrower position
        self.user_position.interest_accrued = self
            .user_position
            .interest_accrued
            .checked_sub(interest_payment)
            .ok_or(LendingError::Overflow)?;

        self.user_position.borrowed_amount = self
            .user_position
            .borrowed_amount
            .checked_sub(principal_payment)
            .ok_or(LendingError::Overflow)?;

        self.user_position.collateral_deposited = self
            .user_position
            .collateral_deposited
            .checked_sub(collateral_to_seize)
            .ok_or(LendingError::Overflow)?;

        // 11. Update pool totals
        self.pool.total_collateral = self
            .pool
            .total_collateral
            .checked_sub(collateral_to_seize)
            .ok_or(LendingError::Overflow)?;

        self.pool.total_borrowed = self
            .pool
            .total_borrowed
            .checked_sub(principal_payment)
            .ok_or(LendingError::Overflow)?;

        // 12. Close position if fully cleared
        if self.user_position.collateral_deposited == 0 && self.user_position.borrowed_amount == 0 {
            self.user_position.is_open = false;
        }

        Ok(())
    }
}

pub fn liquidate_handler(ctx: Context<Liquidate>, repay_amount: u64) -> Result<()> {
    ctx.accounts.liquidate(repay_amount)
}
