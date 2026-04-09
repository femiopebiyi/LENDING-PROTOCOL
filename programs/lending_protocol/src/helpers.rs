use anchor_lang::prelude::*;

use crate::constants::{PRICE_SCALE, RATE_SCALE, SECONDS_PER_YEAR};
use crate::errors::LendingError;
use crate::state::{LendingPool, StubOracle, UserPosition};

/// Settle interest accrued since last_update_time
pub fn accrue_interest(
    position: &mut UserPosition,
    current_time: i64,
    interest_rate: u64,
) -> Result<()> {
    let time_elapsed = current_time.saturating_sub(position.last_update_time) as u128;

    let interest = (position.borrowed_amount as u128)
        .checked_mul(interest_rate as u128)
        .ok_or(LendingError::Overflow)?
        .checked_mul(time_elapsed)
        .ok_or(LendingError::Overflow)?
        .checked_div(RATE_SCALE)
        .ok_or(LendingError::Overflow)?
        .checked_div(SECONDS_PER_YEAR)
        .ok_or(LendingError::Overflow)? as u64;

    position.interest_accrued = position
        .interest_accrued
        .checked_add(interest)
        .ok_or(LendingError::Overflow)?;

    position.last_update_time = current_time;
    Ok(())
}

/// Calculate health factor scaled by PRICE_SCALE
/// health = (collateral_value * liquidation_threshold) / (borrowed + interest)
/// A value below PRICE_SCALE (1.0) means the position is liquidatable
pub fn health_factor(
    position: &UserPosition,
    pool: &LendingPool,
    oracle: &StubOracle,
) -> Result<u128> {
    // if nothing is borrowed the position is perfectly healthy
    let total_debt = position
        .borrowed_amount
        .checked_add(position.interest_accrued)
        .ok_or(LendingError::Overflow)? as u128;

    if total_debt == 0 {
        return Ok(u128::MAX);
    }

    let collateral_value = (position.collateral_deposited as u128)
        .checked_mul(oracle.price as u128)
        .ok_or(LendingError::Overflow)?
        .checked_div(PRICE_SCALE)
        .ok_or(LendingError::Overflow)?;

    let effective_collateral = collateral_value
        .checked_mul(pool.liquidation_threshold as u128)
        .ok_or(LendingError::Overflow)?
        .checked_div(100)
        .ok_or(LendingError::Overflow)?;

    let health = effective_collateral
        .checked_mul(PRICE_SCALE)
        .ok_or(LendingError::Overflow)?
        .checked_div(total_debt)
        .ok_or(LendingError::Overflow)?;

    Ok(health)
}
