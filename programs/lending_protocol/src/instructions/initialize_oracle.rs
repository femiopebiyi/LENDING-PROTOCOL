use anchor_lang::prelude::*;
use pyth_solana_receiver_sdk::price_update::get_feed_id_from_hex;

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
    fn initialize(
        &mut self,
        price: u64,
        feed_id_hex: &str,
        bumps: &InitializeOracleBumps,
        seed: u64,
    ) -> Result<()> {
        require_gt!(price, 0, LendingError::InvalidAmount);

        // parse and store the feed ID at creation time so set_oracle_price
        // uses the correct feed for this oracle regardless of the asset
        let feed_id = get_feed_id_from_hex(feed_id_hex)
            .map_err(|_| error!(LendingError::InvalidOraclePrice))?;

        let clock = Clock::get()?;

        self.oracle.set_inner(StubOracle {
            authority: self.owner.key(),
            price,
            bump: bumps.oracle,
            seed,
            feed_id,
            last_updated_at: clock.unix_timestamp,
        });

        Ok(())
    }
}

pub fn initialize_oracle_handler(
    ctx: Context<InitializeOracle>,
    seed: u64,
    price: u64,
    feed_id_hex: String,
) -> Result<()> {
    ctx.accounts
        .initialize(price, &feed_id_hex, &ctx.bumps, seed)
}
