// withdraw_liquidity.rs
use anchor_lang::prelude::*;
use anchor_spl::{
    associated_token::AssociatedToken,
    token_interface::{transfer_checked, Mint, TokenAccount, TokenInterface, TransferChecked},
};

use crate::{
    errors::LendingError,
    state::{LendingPool, LiquidityProvider},
};

#[derive(Accounts)]
pub struct WithdrawLiquidity<'info> {
    #[account(mut)]
    pub withdrawer: Signer<'info>,

    #[account(
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
        init_if_needed,
        payer = withdrawer,
        associated_token::token_program = token_program,
        associated_token::authority = withdrawer,
        associated_token::mint = borrow_mint,
    )]
    pub provider_ata_borrow: InterfaceAccount<'info, TokenAccount>,

    #[account(
        mut,
        seeds = [b"liquidityprovider", pool.key().as_ref(), withdrawer.key().as_ref()],
        bump = provider_info.bump,
    )]
    pub provider_info: Account<'info, LiquidityProvider>,

    pub associated_token_program: Program<'info, AssociatedToken>,
    pub token_program: Interface<'info, TokenInterface>,
    pub system_program: Program<'info, System>,
}

impl<'info> WithdrawLiquidity<'info> {
    fn withdraw_liquidity(&mut self, amount: u64) -> Result<()> {
        require!(!self.pool.is_paused, LendingError::Paused);
        require!(amount > 0, LendingError::InvalidAmount);
        require!(
            amount <= self.provider_info.amount_provided,
            LendingError::InsufficientLiquidity
        );
        require!(
            self.borrow_vault.amount >= amount,
            LendingError::InsufficientLiquidity
        );

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
                    to: self.provider_ata_borrow.to_account_info(),
                    authority: self.pool.to_account_info(),
                    mint: self.borrow_mint.to_account_info(),
                },
                signer_seeds,
            ),
            amount,
            self.borrow_mint.decimals,
        )?;

        self.provider_info.amount_provided = self
            .provider_info
            .amount_provided
            .checked_sub(amount)
            .ok_or(LendingError::Overflow)?;

        self.pool.total_liquidity_deposited = self
            .pool
            .total_liquidity_deposited
            .checked_sub(amount)
            .ok_or(LendingError::Overflow)?;

        Ok(())
    }
}

pub fn withdraw_liquidity_handler(ctx: Context<WithdrawLiquidity>, amount: u64) -> Result<()> {
    ctx.accounts.withdraw_liquidity(amount)
}
