use anchor_lang::prelude::*;

use crate::constants::{PRICE_SCALE, RATE_SCALE, SECONDS_PER_YEAR};
use crate::errors::LendingError;
use crate::state::{LendingPool, StubOracle, UserPosition};

// L-2: maximum seconds a cached oracle price is trusted for use in
// borrow / liquidate. Callers pass the current clock and compare against
// oracle.last_updated_at.
pub const MAX_ORACLE_AGE_SECS: i64 = 120;

/// Reject oracle prices that are older than MAX_ORACLE_AGE_SECS.
pub fn require_fresh_oracle(oracle: &StubOracle, current_time: i64) -> Result<()> {
    let age = current_time.saturating_sub(oracle.last_updated_at);
    require!(age <= MAX_ORACLE_AGE_SECS, LendingError::StaleOracle);
    Ok(())
}

/// Settle interest accrued since last_update_time.
pub fn accrue_interest(
    position: &mut UserPosition,
    current_time: i64,
    interest_rate: u64,
) -> Result<()> {
    let time_elapsed = current_time.saturating_sub(position.last_update_time) as u128;

    if time_elapsed == 0 {
        return Ok(());
    }

    let total_debt = position
        .borrowed_amount
        .checked_add(position.interest_accrued)
        .ok_or(LendingError::Overflow)?;

    if total_debt == 0 {
        position.last_update_time = current_time;
        return Ok(());
    }

    let interest_u128 = (total_debt as u128)
        .checked_mul(interest_rate as u128)
        .ok_or(LendingError::Overflow)?
        .checked_mul(time_elapsed)
        .ok_or(LendingError::Overflow)?
        .checked_div(RATE_SCALE)
        .ok_or(LendingError::Overflow)?
        .checked_div(SECONDS_PER_YEAR)
        .ok_or(LendingError::Overflow)?;

    // M-1: use checked cast instead of bare `as u64` which silently truncates
    let interest = u64::try_from(interest_u128).map_err(|_| error!(LendingError::Overflow))?;

    position.interest_accrued = position
        .interest_accrued
        .checked_add(interest)
        .ok_or(LendingError::Overflow)?;

    position.last_update_time = current_time;
    Ok(())
}

/// Calculate health factor scaled by PRICE_SCALE.
///
/// health = (collateral_value_usd * liquidation_threshold)
///        / total_debt_usd
///
/// Both sides are expressed in the same USD unit (PRICE_SCALE = 1_000_000).
/// A value below PRICE_SCALE (1.0) means the position is liquidatable.
///
/// C-3: `collateral_decimals` and `borrow_decimals` normalise raw token
/// amounts to whole-unit USD values before comparing them, making the
/// function correct for any mint pair rather than only USDC/SOL with a
/// coincidental decimal match.
pub fn health_factor(
    position: &UserPosition,
    pool: &LendingPool,
    oracle: &StubOracle,
    collateral_decimals: u8,
    borrow_decimals: u8,
) -> Result<u128> {
    let total_debt_raw = position
        .borrowed_amount
        .checked_add(position.interest_accrued)
        .ok_or(LendingError::Overflow)? as u128;

    if total_debt_raw == 0 {
        return Ok(u128::MAX);
    }

    // Normalise collateral to USD (PRICE_SCALE units)
    let collateral_scale = 10u128
        .checked_pow(collateral_decimals as u32)
        .ok_or(LendingError::Overflow)?;

    let collateral_value_usd = (position.collateral_deposited as u128)
        .checked_mul(oracle.price as u128)
        .ok_or(LendingError::Overflow)?
        .checked_div(collateral_scale)
        .ok_or(LendingError::Overflow)?;

    // Apply liquidation threshold to collateral value
    let effective_collateral = collateral_value_usd
        .checked_mul(pool.liquidation_threshold as u128)
        .ok_or(LendingError::Overflow)?
        .checked_div(100)
        .ok_or(LendingError::Overflow)?;

    // Normalise debt to USD (PRICE_SCALE units).
    // Assumes the borrow token is a USD stablecoin (1 whole unit = $1).
    let borrow_scale = 10u128
        .checked_pow(borrow_decimals as u32)
        .ok_or(LendingError::Overflow)?;

    let total_debt_usd = total_debt_raw
        .checked_mul(PRICE_SCALE)
        .ok_or(LendingError::Overflow)?
        .checked_div(borrow_scale)
        .ok_or(LendingError::Overflow)?;

    let health = effective_collateral
        .checked_mul(PRICE_SCALE)
        .ok_or(LendingError::Overflow)?
        .checked_div(total_debt_usd)
        .ok_or(LendingError::Overflow)?;

    Ok(health)
}

/// Convert a raw Pyth price to our internal PRICE_SCALE (6 decimals).
pub fn convert_pyth_price(price: i64, exponent: i32) -> Result<u64> {
    require!(price > 0, LendingError::InvalidOraclePrice);

    let price_u128 = price as u128;

    let adjusted = if exponent < 0 {
        let exp = (-exponent) as u32;
        // M-4: guard against exponents large enough to overflow u128
        require!(exp <= 38, LendingError::InvalidOraclePrice);
        let pyth_scale = 10u128.checked_pow(exp).ok_or(LendingError::Overflow)?;

        price_u128
            .checked_mul(PRICE_SCALE)
            .ok_or(LendingError::Overflow)?
            .checked_div(pyth_scale)
            .ok_or(LendingError::Overflow)?
    } else {
        let exp = exponent as u32;
        require!(exp <= 38, LendingError::InvalidOraclePrice);
        price_u128
            .checked_mul(PRICE_SCALE)
            .ok_or(LendingError::Overflow)?
            .checked_mul(10u128.checked_pow(exp).ok_or(LendingError::Overflow)?)
            .ok_or(LendingError::Overflow)?
    };

    // M-1: checked cast
    u64::try_from(adjusted).map_err(|_| error!(LendingError::Overflow))
}
