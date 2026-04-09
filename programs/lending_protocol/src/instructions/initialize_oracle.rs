use anchor_lang::prelude::*;

use crate::{errors::LendingError, state::StubOracle};

#[derive(Accounts)]
#[instruction(seed: u64)]
pub struct InitializeOracle<'info> {
    #[account(mut)]
    pub owner: Signer<'info>,

    #[account(
        init,
        payer = owner,
        space = 8 + StubOracle::INIT_SPACE,
        seeds = [b"stuboracle", owner.key().as_ref(), seed.to_le_bytes().as_ref()],
        bump
    )]
    pub oracle: Account<'info, StubOracle>,

    pub system_program: Program<'info, System>,
}

impl<'info> InitializeOracle<'info> {
    fn initialize(&mut self, price: u64, bumps: &InitializeOracleBumps, seed: u64) -> Result<()> {
        require_gt!(price, 0, LendingError::InvalidAmount);

        self.oracle.set_inner(StubOracle {
            authority: self.owner.key(),
            price,
            bump: bumps.oracle,
            seed,
        });

        Ok(())
    }
}

pub fn initialize_oracle_handler(
    ctx: Context<InitializeOracle>,
    seed: u64,
    price: u64,
) -> Result<()> {
    ctx.accounts.initialize(price, &ctx.bumps, seed)
}
