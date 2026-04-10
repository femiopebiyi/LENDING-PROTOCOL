use anchor_lang::prelude::*;

#[account]
#[derive(InitSpace)]
pub struct StubOracle {
    pub seed: u64,
    pub authority: Pubkey, // who can update the price
    pub price: u64,        // price in USD scaled by PRICE_SCALE
    pub bump: u8,
    pub feed_id: [u8; 32], // C-2: stored at init, not hardcoded at usage
    pub last_updated_at: i64,
}
