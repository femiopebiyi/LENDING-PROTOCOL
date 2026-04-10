use anchor_lang::prelude::*;
use anchor_spl::{
    associated_token::AssociatedToken,
    token_interface::{transfer_checked, Mint, TokenAccount, TokenInterface, TransferChecked},
};

use crate::state::{LendingPool, UserPosition};
use crate::{errors::LendingError, helpers::accrue_interest};

#[derive(Accounts)]
pub struct Repay<'info> {
    #[account(mut)]
    pub repayer: Signer<'info>,

    #[account(
        mut,
        seeds = [b"lendingpool", pool.owner.as_ref(), pool.seed.to_le_bytes().as_ref()],
        bump = pool.bump
    )]
    pub pool: Account<'info, LendingPool>,

    #[account(
        mint::token_program = token_program,
        address = pool.borrow_mint @ LendingError::InvalidMint
    )]
    pub borrow_mint: InterfaceAccount<'info, Mint>,

    #[account(
        mut,
        associated_token::mint = borrow_mint,
        associated_token::authority = pool,
        associated_token::token_program = token_program,
    )]
    pub borrow_vault: InterfaceAccount<'info, TokenAccount>,

    #[account(
        mut,
        associated_token::token_program = token_program,
        associated_token::authority = repayer,
        associated_token::mint = borrow_mint,
    )]
    pub user_ata_borrow: InterfaceAccount<'info, TokenAccount>,

    #[account(
        mut,
        seeds = [b"userposition", pool.key().as_ref(), repayer.key().as_ref()],
        bump = user_position.bump,
        constraint = user_position.owner.key() == repayer.key() @ LendingError::CredentialMismatch
    )]
    pub user_position: Account<'info, UserPosition>,

    pub associated_token_program: Program<'info, AssociatedToken>,
    pub token_program: Interface<'info, TokenInterface>,
    pub system_program: Program<'info, System>,
}

impl<'info> Repay<'info> {
    fn repay(&mut self, amount: u64) -> Result<()> {
        // 1. Validate
        require!(amount > 0, LendingError::InvalidAmount);

        // 2. Settle interest before computing debt
        let clock = Clock::get()?;
        accrue_interest(
            &mut self.user_position,
            clock.unix_timestamp,
            self.pool.interest_rate,
        )?;

        // 3. Calculate total debt
        let total_debt = self
            .user_position
            .borrowed_amount
            .checked_add(self.user_position.interest_accrued)
            .ok_or(LendingError::Overflow)?;

        require!(total_debt > 0, LendingError::InsufficientBorrow);

        // 4. Cap repay at total debt — cannot overpay
        let repay_amount = amount.min(total_debt);

        // 5. Split: clear interest first, remainder reduces principal
        let interest_payment = repay_amount.min(self.user_position.interest_accrued);
        let principal_payment = repay_amount
            .checked_sub(interest_payment)
            .ok_or(LendingError::Overflow)?;

        // 6. Update state before CPI (CEI pattern — H-3)
        // If the borrow token has a Token-2022 transfer hook that calls back
        // into the protocol, the updated (reduced) balances are already visible.
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

        self.pool.total_borrowed = self
            .pool
            .total_borrowed
            .checked_sub(principal_payment)
            .ok_or(LendingError::Overflow)?;

        self.pool.total_interest_accrued = self
            .pool
            .total_interest_accrued
            .checked_add(interest_payment)
            .ok_or(LendingError::Overflow)?;

        // M-6: use checked addition for the closure test
        let remaining_debt = self
            .user_position
            .borrowed_amount
            .checked_add(self.user_position.interest_accrued)
            .ok_or(LendingError::Overflow)?;

        if remaining_debt == 0 {
            self.user_position.is_open = false;
        }

        self.user_position.last_update_time = clock.unix_timestamp;

        // 7. CPI: transfer repayment from user ATA to borrow vault
        transfer_checked(
            CpiContext::new(
                self.token_program.to_account_info(),
                TransferChecked {
                    from: self.user_ata_borrow.to_account_info(),
                    to: self.borrow_vault.to_account_info(),
                    mint: self.borrow_mint.to_account_info(),
                    authority: self.repayer.to_account_info(),
                },
            ),
            repay_amount,
            self.borrow_mint.decimals,
        )?;

        Ok(())
    }
}

pub fn repay_handler(ctx: Context<Repay>, amount: u64) -> Result<()> {
    ctx.accounts.repay(amount)
}
