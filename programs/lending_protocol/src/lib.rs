use anchor_lang::prelude::*;

declare_id!("8Yb4i9zTPzrXH1fqqvSSbYASnyxSuy2hMJnTfuLM4iA5");

mod state;

mod instructions;
use instructions::*;

mod constants;
mod errors;
mod helpers;

#[program]
pub mod lending_protocol {

    use super::*;

    pub fn initialize_oracle(ctx: Context<InitializeOracle>, seed: u64, price: u64) -> Result<()> {
        instructions::initialize_oracle::initialize_oracle_handler(ctx, seed, price)
    }

    pub fn set_oracle_price(ctx: Context<SetOraclePrice>) -> Result<()> {
        instructions::set_oracle_price::set_oracle_price_handler(ctx)
    }

    pub fn initialize_pool(
        ctx: Context<InitializePool>,
        seed: u64,
        liquidation_threshold: u64,
        liquidation_bonus: u64,
        interest_rate: u64,
        max_ltv: u64,
    ) -> Result<()> {
        instructions::initialize_pool::initialize_pool_handler(
            ctx,
            seed,
            liquidation_threshold,
            liquidation_bonus,
            interest_rate,
            max_ltv,
        )
    }

    pub fn deposit_collateral(ctx: Context<DepositCollateral>, amount: u64) -> Result<()> {
        instructions::deposit_collateral::deposit_collateral_handler(ctx, amount)
    }

    pub fn borrow(ctx: Context<Borrow>, amount: u64) -> Result<()> {
        instructions::borrow::borrow_handler(ctx, amount)
    }

    pub fn repay(ctx: Context<Repay>, amount: u64) -> Result<()> {
        instructions::repay::repay_handler(ctx, amount)
    }

    pub fn withdraw_collateral(ctx: Context<WithdrawCollateral>, amount: u64) -> Result<()> {
        instructions::withdraw_collateral::withdraw_collateral_handler(ctx, amount)
    }

    pub fn liquidate(ctx: Context<Liquidate>, repay_amount: u64) -> Result<()> {
        instructions::liquidate::liquidate_handler(ctx, repay_amount)
    }

    pub fn add_liquidity(ctx: Context<AddLiquidity>, amount: u64) -> Result<()> {
        instructions::add_liquidity::add_liquidity_handler(ctx, amount)
    }

    pub fn withdraw_liquidity(ctx: Context<WithdrawLiquidity>, amount: u64) -> Result<()> {
        instructions::withdraw_liquidity::withdraw_liquidity_handler(ctx, amount)
    }

    pub fn pause_pool(ctx: Context<PausePool>) -> Result<()> {
        instructions::pause_pool::pause_pool_handler(ctx)
    }

    pub fn unpause_pool(ctx: Context<UnpausePool>) -> Result<()> {
        instructions::unpause_pool::unpause_pool_handler(ctx)
    }
}
