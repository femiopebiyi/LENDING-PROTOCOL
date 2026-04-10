// state/liquidity_provider.rs
use anchor_lang::prelude::*;

#[account]
#[derive(InitSpace)]
pub struct LiquidityProvider {
    pub provider: Pubkey,
    pub provided_mint: Pubkey,
    pub amount_provided: u64,
    pub bump: u8,
}
