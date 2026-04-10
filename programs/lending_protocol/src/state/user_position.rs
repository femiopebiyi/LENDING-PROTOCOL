use anchor_lang::prelude::*;

#[account]
#[derive(InitSpace)]
pub struct UserPosition {
    pub owner: Pubkey,             // the borrower
    pub pool: Pubkey,              // which pool this position belongs to
    pub collateral_deposited: u64, // raw token amount deposited
    pub borrowed_amount: u64,      // raw token amount borrowed
    pub interest_accrued: u64,     // interest accumulated but not yet repaid
    pub last_update_time: i64,     // unix timestamp of last interest settlement
    pub bump: u8,
    pub is_open: bool,
    pub is_liquidating: bool,
}
