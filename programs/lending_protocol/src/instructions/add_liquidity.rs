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
pub struct AddLiquidity<'info> {
    #[account(mut)]
    pub provider: Signer<'info>,

    // H-1: pool must be mut so total_liquidity_deposited is serialized back
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
        associated_token::authority = provider,
        associated_token::mint = borrow_mint,
    )]
    pub provider_ata_borrow: InterfaceAccount<'info, TokenAccount>,

    #[account(
        init_if_needed,
        payer = provider,
        space = 8 + LiquidityProvider::INIT_SPACE,
        seeds = [b"liquidityprovider", pool.key().as_ref(), provider.key().as_ref()],
        bump
    )]
    pub provider_info: Account<'info, LiquidityProvider>,

    pub associated_token_program: Program<'info, AssociatedToken>,
    pub token_program: Interface<'info, TokenInterface>,
    pub system_program: Program<'info, System>,
}

impl<'info> AddLiquidity<'info> {
    fn add_liquidity(&mut self, amount: u64, bumps: &AddLiquidityBumps) -> Result<()> {
        require!(!self.pool.is_paused, LendingError::Paused);
        require!(amount > 0, LendingError::InvalidAmount);

        if self.provider_info.provider == Pubkey::default() {
            self.provider_info.provider = self.provider.key();
            self.provider_info.provided_mint = self.borrow_mint.key();
            self.provider_info.bump = bumps.provider_info;
            self.provider_info.amount_provided = 0;
        }

        // Update state before CPI (CEI — H-3)
        self.provider_info.amount_provided = self
            .provider_info
            .amount_provided
            .checked_add(amount)
            .ok_or(LendingError::Overflow)?;

        self.pool.total_liquidity_deposited = self
            .pool
            .total_liquidity_deposited
            .checked_add(amount)
            .ok_or(LendingError::Overflow)?;

        transfer_checked(
            CpiContext::new(
                self.token_program.to_account_info(),
                TransferChecked {
                    authority: self.provider.to_account_info(),
                    from: self.provider_ata_borrow.to_account_info(),
                    to: self.borrow_vault.to_account_info(),
                    mint: self.borrow_mint.to_account_info(),
                },
            ),
            amount,
            self.borrow_mint.decimals,
        )?;

        Ok(())
    }
}

pub fn add_liquidity_handler(ctx: Context<AddLiquidity>, amount: u64) -> Result<()> {
    ctx.accounts.add_liquidity(amount, &ctx.bumps)
}
