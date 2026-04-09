use anchor_lang::prelude::*;
use anchor_spl::{
    associated_token::AssociatedToken,
    token_interface::{Mint, TokenAccount, TokenInterface},
};

use crate::{
    errors::LendingError,
    state::{LendingPool, StubOracle},
};

#[derive(Accounts)]
#[instruction(seed: u64)]
pub struct InitializePool<'info> {
    #[account(mut)]
    pub owner: Signer<'info>,

    #[account(
        init,
        payer = owner,
        space = 8 + LendingPool::INIT_SPACE,
        seeds = [b"lendingpool", owner.key().as_ref(), seed.to_le_bytes().as_ref()],
        bump
    )]
    pub pool: Account<'info, LendingPool>,

    #[account(
        mint::token_program = token_program,
        address = pool.collateral_mint @ LendingError::InvalidMint
    )]
    pub collateral_mint: InterfaceAccount<'info, Mint>,

    #[account(
        mint::token_program = token_program,
        constraint = borrow_mint.key() != collateral_mint.key() @ LendingError::InvalidParameter,
        address = pool.borrow_mint @ LendingError::InvalidMint
    )]
    pub borrow_mint: InterfaceAccount<'info, Mint>,

    #[account(
        init,
        payer = owner,
        associated_token::mint = collateral_mint,
        associated_token::authority = pool,
        associated_token::token_program = token_program,
    )]
    pub collateral_vault: InterfaceAccount<'info, TokenAccount>,

    #[account(
        init,
        payer = owner,
        associated_token::mint = borrow_mint,
        associated_token::authority = pool,
        associated_token::token_program = token_program,
    )]
    pub borrow_vault: InterfaceAccount<'info, TokenAccount>,

    #[account(
        seeds = [b"stuboracle", oracle.authority.as_ref(), oracle.seed.to_le_bytes().as_ref()],
        bump = oracle.bump,
    )]
    pub oracle: Account<'info, StubOracle>,

    pub associated_token_program: Program<'info, AssociatedToken>,
    pub token_program: Interface<'info, TokenInterface>,
    pub system_program: Program<'info, System>,
}

impl<'info> InitializePool<'info> {
    fn initialize_pool(
        &mut self,
        seed: u64,
        liquidation_threshold: u64,
        liquidation_bonus: u64,
        interest_rate: u64,
        max_ltv: u64,
        bumps: &InitializePoolBumps,
    ) -> Result<()> {
        require!(
            liquidation_threshold > 0 && liquidation_threshold <= 100,
            LendingError::InvalidParameter
        );
        require!(
            liquidation_bonus > 0 && liquidation_bonus <= 20,
            LendingError::InvalidParameter
        );
        require!(
            max_ltv > 0 && max_ltv < liquidation_threshold,
            LendingError::InvalidParameter
        );
        require!(interest_rate > 0, LendingError::InvalidParameter);

        self.pool.set_inner(LendingPool {
            owner: self.owner.key(),
            collateral_mint: self.collateral_mint.key(),
            borrow_mint: self.borrow_mint.key(),
            collateral_vault: self.collateral_vault.key(),
            borrow_vault: self.borrow_vault.key(),
            oracle: self.oracle.key(),
            liquidation_threshold,
            liquidation_bonus,
            interest_rate,
            max_ltv,
            total_collateral: 0,
            total_borrowed: 0,
            bump: bumps.pool,
            seed,
        });

        Ok(())
    }
}

pub fn initialize_pool_handler(
    ctx: Context<InitializePool>,
    seed: u64,
    liquidation_threshold: u64,
    liquidation_bonus: u64,
    interest_rate: u64,
    max_ltv: u64,
) -> Result<()> {
    ctx.accounts.initialize_pool(
        seed,
        liquidation_threshold,
        liquidation_bonus,
        interest_rate,
        max_ltv,
        &ctx.bumps,
    )
}
