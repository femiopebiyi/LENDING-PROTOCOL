use anchor_lang::prelude::*;
use anchor_spl::{
    associated_token::AssociatedToken,
    token_interface::{transfer_checked, Mint, TokenAccount, TokenInterface, TransferChecked},
};

use crate::{
    constants::PRICE_SCALE,
    errors::LendingError,
    helpers::{accrue_interest, health_factor, require_fresh_oracle},
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

    // C-4: collateral_mint is now constrained to pool.collateral_mint.
    // Without this an attacker could pass any mint, the derived ATA would
    // not match the real vault, and pool state would be corrupted with no
    // actual collateral transfer.
    #[account(
        mint::token_program = token_program,
        address = pool.collateral_mint @ LendingError::InvalidMint
    )]
    pub collateral_mint: InterfaceAccount<'info, Mint>,

    #[account(
        mint::token_program = token_program,
        address = pool.borrow_mint @ LendingError::InvalidMint
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
        constraint = user_position.owner.key() == borrower.key() @ LendingError::CredentialMismatch
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
        // M-5: removed is_liquidating flag — Solana serialises account access
        // per transaction so two concurrent txs cannot both pass this check on
        // the same UserPosition account simultaneously.

        require!(repay_amount > 0, LendingError::InvalidAmount);

        let clock = Clock::get()?;

        // L-2: reject stale oracle
        require_fresh_oracle(&self.oracle, clock.unix_timestamp)?;

        // 1. Settle interest on borrower's position
        accrue_interest(
            &mut self.user_position,
            clock.unix_timestamp,
            self.pool.interest_rate,
        )?;

        // 2. Verify position is unhealthy
        let health = health_factor(
            &self.user_position,
            &self.pool,
            &self.oracle,
            self.collateral_mint.decimals,
            self.borrow_mint.decimals,
        )?;
        require!(health < PRICE_SCALE, LendingError::PositionHealthy);

        // 3. Cap repay at total debt
        let total_debt = self
            .user_position
            .borrowed_amount
            .checked_add(self.user_position.interest_accrued)
            .ok_or(LendingError::Overflow)?;

        let repay_amount = repay_amount.min(total_debt);

        // 4. C-3: convert repay amount (raw borrow token units) to USD,
        //    then convert USD to raw collateral token units.
        let borrow_scale = 10u128
            .checked_pow(self.borrow_mint.decimals as u32)
            .ok_or(LendingError::Overflow)?;

        let collateral_scale = 10u128
            .checked_pow(self.collateral_mint.decimals as u32)
            .ok_or(LendingError::Overflow)?;

        // repay_value_usd is in PRICE_SCALE (1e6) USD units
        let repay_value_usd = (repay_amount as u128)
            .checked_mul(PRICE_SCALE)
            .ok_or(LendingError::Overflow)?
            .checked_div(borrow_scale)
            .ok_or(LendingError::Overflow)?;

        // convert USD value to raw collateral units using oracle price
        let collateral_amount_u128 = repay_value_usd
            .checked_mul(collateral_scale)
            .ok_or(LendingError::Overflow)?
            .checked_div(self.oracle.price as u128)
            .ok_or(LendingError::Overflow)?;

        // apply liquidation bonus
        let collateral_with_bonus_u128 = collateral_amount_u128
            .checked_mul(
                (100u128)
                    .checked_add(self.pool.liquidation_bonus as u128)
                    .ok_or(LendingError::Overflow)?,
            )
            .ok_or(LendingError::Overflow)?
            .checked_div(100)
            .ok_or(LendingError::Overflow)?;

        // M-1: checked cast
        let collateral_with_bonus = u64::try_from(collateral_with_bonus_u128)
            .map_err(|_| error!(LendingError::Overflow))?;

        // cap at available collateral
        let collateral_to_seize =
            collateral_with_bonus.min(self.user_position.collateral_deposited);

        // 5. Split repayment: interest first, remainder reduces principal
        let interest_payment = repay_amount.min(self.user_position.interest_accrued);
        let principal_payment = repay_amount
            .checked_sub(interest_payment)
            .ok_or(LendingError::Overflow)?;

        // 6. Update all state before CPIs (CEI pattern — H-3)
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

        if self.user_position.collateral_deposited == 0 && self.user_position.borrowed_amount == 0 {
            self.user_position.is_open = false;
        }

        // 7. Build pool PDA signer seeds
        let seeds = &[
            b"lendingpool",
            self.pool.owner.as_ref(),
            &self.pool.seed.to_le_bytes(),
            &[self.pool.bump],
        ];
        let signer_seeds = &[&seeds[..]];

        // 8. Liquidator pays debt into borrow vault
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

        // 9. Collateral + bonus released to liquidator
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

        Ok(())
    }
}

pub fn liquidate_handler(ctx: Context<Liquidate>, repay_amount: u64) -> Result<()> {
    ctx.accounts.liquidate(repay_amount)
}
