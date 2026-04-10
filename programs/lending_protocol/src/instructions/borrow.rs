use crate::constants::PRICE_SCALE;
use crate::errors::LendingError;
use crate::helpers::{accrue_interest, require_fresh_oracle};
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

    // C-3: collateral_mint is now required to obtain its decimals for
    // correct USD normalisation when computing max_borrow
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

impl<'info> Borrow<'info> {
    fn borrow(&mut self, amount: u64) -> Result<()> {
        require!(!self.pool.is_paused, LendingError::Paused);
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

        let clock = Clock::get()?;

        // L-2: reject stale oracle prices
        require_fresh_oracle(&self.oracle, clock.unix_timestamp)?;

        // 1. Settle interest before any balance math
        accrue_interest(
            &mut self.user_position,
            clock.unix_timestamp,
            self.pool.interest_rate,
        )?;

        // 2. C-3: normalise collateral to USD using actual token decimals.
        //    collateral_value is in PRICE_SCALE (1e6) USD units.
        let collateral_scale = 10u128
            .checked_pow(self.collateral_mint.decimals as u32)
            .ok_or(LendingError::Overflow)?;

        let collateral_value_usd = (self.user_position.collateral_deposited as u128)
            .checked_mul(self.oracle.price as u128)
            .ok_or(LendingError::Overflow)?
            .checked_div(collateral_scale)
            .ok_or(LendingError::Overflow)?;

        // 3. Apply LTV to get max borrowable in USD (still PRICE_SCALE units)
        let max_borrow_usd = collateral_value_usd
            .checked_mul(self.pool.max_ltv as u128)
            .ok_or(LendingError::Overflow)?
            .checked_div(100)
            .ok_or(LendingError::Overflow)?;

        // 4. Convert max_borrow from USD to raw borrow-token units.
        //    Assumes borrow token is a USD stablecoin (1 whole unit = $1).
        let borrow_scale = 10u128
            .checked_pow(self.borrow_mint.decimals as u32)
            .ok_or(LendingError::Overflow)?;

        let max_borrow_raw_u128 = max_borrow_usd
            .checked_mul(borrow_scale)
            .ok_or(LendingError::Overflow)?
            .checked_div(PRICE_SCALE)
            .ok_or(LendingError::Overflow)?;

        // M-1: checked cast instead of bare `as u64`
        let max_borrow =
            u64::try_from(max_borrow_raw_u128).map_err(|_| error!(LendingError::Overflow))?;

        // 5. New total debt must not exceed max borrow
        let new_total_debt = self
            .user_position
            .borrowed_amount
            .checked_add(self.user_position.interest_accrued)
            .ok_or(LendingError::Overflow)?
            .checked_add(amount)
            .ok_or(LendingError::Overflow)?;

        require!(new_total_debt <= max_borrow, LendingError::ExceedsMaxLtv);

        // 6. Update state before CPI (CEI — H-3)
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

        // 7. Pool PDA signs to release tokens from borrow vault
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

        Ok(())
    }
}

pub fn borrow_handler(ctx: Context<Borrow>, amount: u64) -> Result<()> {
    ctx.accounts.borrow(amount)
}
