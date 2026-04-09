use anchor_lang::prelude::*;

#[account]
#[derive(InitSpace)]
pub struct LendingPool {
    pub owner: Pubkey,              // protocol admin
    pub collateral_mint: Pubkey,    // token users deposit as collateral e.g. SOL
    pub borrow_mint: Pubkey,        // token users borrow e.g. USDC
    pub collateral_vault: Pubkey,   // holds collateral deposits
    pub borrow_vault: Pubkey,       // holds lender deposits available to borrow
    pub oracle: Pubkey,             // stub oracle for collateral price
    pub liquidation_threshold: u64, // e.g. 80 = 80% — health drops below 1 at this point
    pub liquidation_bonus: u64,     // e.g. 5 = 5% bonus for liquidators
    pub interest_rate: u64,         // annual interest rate in bps e.g. 500 = 5%
    pub max_ltv: u64,               // max loan to value ratio e.g. 75 = 75%
    pub total_collateral: u64,      // total collateral deposited across all users
    pub total_borrowed: u64,        // total tokens currently borrowed
    pub bump: u8,
    pub seed: u64,
}
