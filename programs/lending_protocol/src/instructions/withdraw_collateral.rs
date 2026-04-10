use anchor_lang::prelude::*;
use anchor_spl::{
    associated_token::AssociatedToken,
    token_interface::{transfer_checked, Mint, TokenAccount, TokenInterface, TransferChecked},
};

use crate::state::{LendingPool, StubOracle, UserPosition};
use crate::{constants::PRICE_SCALE, helpers::health_factor};
use crate::{errors::LendingError, helpers::accrue_interest};

#[derive(Accounts)]
pub struct WithdrawCollateral<'info> {
    #[account(mut)]
    pub withdrawer: Signer<'info>,

    #[account(
        mut,
        seeds = [b"lendingpool", pool.owner.as_ref(), pool.seed.to_le_bytes().as_ref()],
        bump = pool.bump
    )]
    pub pool: Account<'info, LendingPool>,

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
        init_if_needed,
        payer = withdrawer,
        associated_token::token_program = token_program,
        associated_token::authority = withdrawer,
        associated_token::mint = collateral_mint,
    )]
    pub user_ata_collateral: InterfaceAccount<'info, TokenAccount>,

    #[account(
        mut,
        seeds = [b"userposition", pool.key().as_ref(), withdrawer.key().as_ref()],
        bump = user_position.bump,
        constraint = user_position.owner.key() == withdrawer.key() @ LendingError::CredentialMismatch
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

impl<'info> WithdrawCollateral<'info> {
    fn withdraw_collateral(&mut self, amount: u64) -> Result<()> {
        require!(!self.pool.is_paused, LendingError::Paused);
        require!(amount > 0, LendingError::InvalidAmount);
        require!(
            amount <= self.user_position.collateral_deposited,
            LendingError::InsufficientCollateralBalance
        );

        let clock = Clock::get()?;

        // 1. Settle interest before any balance changes
        accrue_interest(
            &mut self.user_position,
            clock.unix_timestamp,
            self.pool.interest_rate,
        )?;

        let total_debt = self
            .user_position
            .borrowed_amount
            .checked_add(self.user_position.interest_accrued)
            .ok_or(LendingError::Overflow)?;

        // 2. H-4: simulate the post-withdrawal collateral balance and run the
        //    health check BEFORE mutating any state. This avoids the pattern of
        //    writing state and then panicking — keeping the code safe to extend.
        let simulated_collateral = self
            .user_position
            .collateral_deposited
            .checked_sub(amount)
            .ok_or(LendingError::Overflow)?;

        if total_debt > 0 {
            // Temporarily borrow a copy to pass to health_factor without
            // mutating the real position yet.
            let simulated_position = UserPosition {
                collateral_deposited: simulated_collateral,
                ..*self.user_position
            };
            let health = health_factor(
                &simulated_position,
                &self.pool,
                &self.oracle,
                self.collateral_mint.decimals,
                self.borrow_mint.decimals,
            )?;
            require!(health >= PRICE_SCALE, LendingError::InsufficientCollateral);
        }

        // 3. State is now known-safe to update
        self.user_position.collateral_deposited = simulated_collateral;

        // 4. CPI: pool PDA releases collateral to user
        let seeds = &[
            b"lendingpool",
            self.pool.owner.as_ref(),
            &self.pool.seed.to_le_bytes(),
            &[self.pool.bump],
        ];
        let signer_seeds = &[&seeds[..]];

        transfer_checked(
            CpiContext::new_with_signer(
                self.token_program.to_account_info(),
                TransferChecked {
                    from: self.collateral_vault.to_account_info(),
                    to: self.user_ata_collateral.to_account_info(),
                    mint: self.collateral_mint.to_account_info(),
                    authority: self.pool.to_account_info(),
                },
                signer_seeds,
            ),
            amount,
            self.collateral_mint.decimals,
        )?;

        // 5. Update pool total
        self.pool.total_collateral = self
            .pool
            .total_collateral
            .checked_sub(amount)
            .ok_or(LendingError::Overflow)?;

        // 6. Close position if fully withdrawn with no outstanding debt
        if self.user_position.collateral_deposited == 0 && self.user_position.borrowed_amount == 0 {
            self.user_position.is_open = false;
        }

        Ok(())
    }
}

pub fn withdraw_collateral_handler(ctx: Context<WithdrawCollateral>, amount: u64) -> Result<()> {
    ctx.accounts.withdraw_collateral(amount)
}
