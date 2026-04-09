use crate::constants::PRICE_SCALE;
use crate::errors::LendingError;
use crate::helpers::accrue_interest;
use crate::state::{LendingPool, StubOracle, UserPosition};
use anchor_lang::prelude::*;
use anchor_spl::{
    associated_token::AssociatedToken,
    token_interface::{transfer_checked, Mint, TokenAccount, TokenInterface, TransferChecked},
};

#[derive(Accounts)]
pub struct Borrow<'info> {
    #[account(mut)]
    pub borrower: Signer<'info>,

    #[account(
        mut,
        seeds = [b"lendingpool", pool.owner.as_ref(), pool.seed.to_le_bytes().as_ref()],
        bump = pool.bump
    )]
    pub pool: Account<'info, LendingPool>,

    #[account(
        mint::token_program = token_program,
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
        init_if_needed,
        payer = borrower,
        associated_token::token_program = token_program,
        associated_token::authority = borrower,
        associated_token::mint = borrow_mint,
    )]
    pub user_ata_borrow: InterfaceAccount<'info, TokenAccount>,

    #[account(
        mut,
        seeds = [b"userposition", pool.key().as_ref(), borrower.key().as_ref()],
        bump = user_position.bump
    )]
    pub user_position: Account<'info, UserPosition>,

    #[account(
        seeds = [b"stuboracle", oracle.authority.as_ref(), oracle.seed.to_le_bytes().as_ref()],
        bump = oracle.bump,
    )]
    pub oracle: Account<'info, StubOracle>,

    pub associated_token_program: Program<'info, AssociatedToken>,
    pub token_program: Interface<'info, TokenInterface>,
    pub system_program: Program<'info, System>,
}

impl<'info> Borrow<'info> {
    fn borrow(&mut self, amount: u64) -> Result<()> {
        // 1. Validate — collateral check first before any math runs
        require!(amount > 0, LendingError::InvalidAmount);
        require!(
            self.user_position.collateral_deposited > 0,
            LendingError::InsufficientCollateral
        );
        require!(
            self.borrow_vault.amount >= amount,
            LendingError::InsufficientLiquidity
        );

        require!(self.user_position.is_open, LendingError::PositionNotOpened);

        // 2. Settle interest before changing balances
        let clock = Clock::get()?;
        accrue_interest(
            &mut self.user_position,
            clock.unix_timestamp,
            self.pool.interest_rate,
        )?;

        // 3. Calculate USD value of deposited collateral
        let collateral_value = (self.user_position.collateral_deposited as u128)
            .checked_mul(self.oracle.price as u128)
            .ok_or(LendingError::Overflow)?
            .checked_div(PRICE_SCALE)
            .ok_or(LendingError::Overflow)?;

        // 4. Calculate maximum the user is allowed to borrow
        let max_borrow = collateral_value
            .checked_mul(self.pool.max_ltv as u128)
            .ok_or(LendingError::Overflow)?
            .checked_div(100)
            .ok_or(LendingError::Overflow)? as u64;

        // 5. New total debt must not exceed max borrow
        let new_total_debt = self
            .user_position
            .borrowed_amount
            .checked_add(self.user_position.interest_accrued)
            .ok_or(LendingError::Overflow)?
            .checked_add(amount)
            .ok_or(LendingError::Overflow)?;

        require!(new_total_debt <= max_borrow, LendingError::ExceedsMaxLtv);

        // 6. Pool PDA signs to release tokens from borrow vault
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
                    from: self.borrow_vault.to_account_info(),
                    to: self.user_ata_borrow.to_account_info(),
                    mint: self.borrow_mint.to_account_info(),
                    authority: self.pool.to_account_info(),
                },
                signer_seeds,
            ),
            amount,
            self.borrow_mint.decimals,
        )?;

        // 7. Update balances
        self.user_position.borrowed_amount = self
            .user_position
            .borrowed_amount
            .checked_add(amount)
            .ok_or(LendingError::Overflow)?;

        self.pool.total_borrowed = self
            .pool
            .total_borrowed
            .checked_add(amount)
            .ok_or(LendingError::Overflow)?;

        self.user_position.last_update_time = clock.unix_timestamp;

        Ok(())
    }
}

pub fn borrow_handler(ctx: Context<Borrow>, amount: u64) -> Result<()> {
    ctx.accounts.borrow(amount)
}
