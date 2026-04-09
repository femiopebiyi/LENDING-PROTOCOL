use anchor_lang::prelude::*;

use crate::{errors::LendingError, state::StubOracle};

#[derive(Accounts)]
pub struct SetOraclePrice<'info> {
    #[account(mut)]
    pub setter: Signer<'info>,

    #[account(
        mut,
        seeds = [b"stuboracle", oracle.authority.as_ref(), oracle.seed.to_le_bytes().as_ref()],
        bump = oracle.bump,
         constraint = oracle.authority == setter.key() @ LendingError::CredentialMismatch
    )]
    pub oracle: Account<'info, StubOracle>,
}

impl<'info> SetOraclePrice<'info> {
    fn set_oracle_price(&mut self, price: u64) -> Result<()> {
        require_gt!(price, 0, LendingError::InvalidAmount);

        self.oracle.price = price;

        Ok(())
    }
}

pub fn set_oracle_price_handler(ctx: Context<SetOraclePrice>, price: u64) -> Result<()> {
    ctx.accounts.set_oracle_price(price)
}
