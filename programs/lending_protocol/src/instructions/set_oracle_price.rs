use anchor_lang::prelude::*;
use pyth_solana_receiver_sdk::price_update::{get_feed_id_from_hex, PriceUpdateV2};

use crate::{errors::LendingError, helpers::convert_pyth_price, state::StubOracle};

const SOL_USD_FEED_ID: &str = "0xef0d8b6fda2ceba41da15d4095d1da392a0d2f8ed0c6c7bc0f4cfac8c280b56d";

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

    /// CHECK: deserialized and validated manually using Pyth SDK
    pub price_update: UncheckedAccount<'info>,

    pub system_program: Program<'info, System>,
}

impl<'info> SetOraclePrice<'info> {
    fn set_oracle_price(&mut self) -> Result<()> {
        // manually deserialize using Pyth SDK's own method
        let data = self
            .price_update
            .try_borrow_data()
            .map_err(|_| error!(LendingError::InvalidOraclePrice))?;

        let price_update = PriceUpdateV2::deserialize(&mut &data[8..])
            .map_err(|_| error!(LendingError::InvalidOraclePrice))?;

        let feed_id = get_feed_id_from_hex(SOL_USD_FEED_ID)
            .map_err(|_| error!(LendingError::InvalidOraclePrice))?;

        let price = price_update
            .get_price_no_older_than(&Clock::get()?, 60, &feed_id)
            .map_err(|_| error!(LendingError::InvalidOraclePrice))?;

        let converted_price = convert_pyth_price(price.price, price.exponent)?;

        self.oracle.price = converted_price;

        Ok(())
    }
}

pub fn set_oracle_price_handler(ctx: Context<SetOraclePrice>) -> Result<()> {
    ctx.accounts.set_oracle_price()
}
