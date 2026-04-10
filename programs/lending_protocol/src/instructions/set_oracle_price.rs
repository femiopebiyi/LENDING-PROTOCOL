use anchor_lang::prelude::*;

use crate::{errors::LendingError, helpers::convert_pyth_price, state::StubOracle};

// Maximum age in seconds a Pyth price is trusted for.
pub const MAX_PRICE_AGE_SECS: u64 = 60;

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

    // C-1: require this account to be owned by the Pyth receiver program.
    // Using `owner =` forces Anchor to verify the account's program owner before
    // we touch any data, preventing a fake account with crafted bytes from passing.
    #[account(
        owner = pyth_solana_receiver_sdk::ID @ LendingError::InvalidOracleProgram
    )]
    /// CHECK: owner verified by constraint above; data deserialized via Pyth SDK below
    pub price_update: UncheckedAccount<'info>,

    pub system_program: Program<'info, System>,
}

impl<'info> SetOraclePrice<'info> {
    fn set_oracle_price(&mut self) -> Result<()> {
        use pyth_solana_receiver_sdk::price_update::PriceUpdateV2;

        let data = self
            .price_update
            .try_borrow_data()
            .map_err(|_| error!(LendingError::InvalidOraclePrice))?;

        let price_update = PriceUpdateV2::deserialize(&mut &data[8..])
            .map_err(|_| error!(LendingError::InvalidOraclePrice))?;

        // C-2: use the feed ID stored on the oracle account rather than a
        // compile-time constant, so each pool's oracle uses the right price feed
        let feed_id = self.oracle.feed_id;

        let clock = Clock::get()?;
        let price = price_update
            .get_price_no_older_than(&clock, MAX_PRICE_AGE_SECS, &feed_id)
            .map_err(|_| error!(LendingError::InvalidOraclePrice))?;

        let converted_price = convert_pyth_price(price.price, price.exponent)?;

        self.oracle.price = converted_price;
        // L-2: record when this price was written so borrow/liquidate can
        // reject it if it grows stale between oracle updates
        self.oracle.last_updated_at = clock.unix_timestamp;

        Ok(())
    }
}

pub fn set_oracle_price_handler(ctx: Context<SetOraclePrice>) -> Result<()> {
    ctx.accounts.set_oracle_price()
}
