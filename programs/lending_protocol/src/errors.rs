use anchor_lang::prelude::*;

#[error_code]
pub enum LendingError {
    #[msg("Invalid amount — must be greater than zero")]
    InvalidAmount,
    #[msg("Insufficient collateral to borrow against")]
    InsufficientCollateral,
    #[msg("Borrow amount exceeds maximum loan to value ratio")]
    ExceedsMaxLtv,
    #[msg("Position is healthy and cannot be liquidated")]
    PositionHealthy,
    #[msg("Insufficient borrowed balance")]
    InsufficientBorrow,
    #[msg("Insufficient collateral balance")]
    InsufficientCollateralBalance,
    #[msg("Arithmetic overflow")]
    Overflow,
    #[msg("Invalid fee or rate parameter")]
    InvalidParameter,
    #[msg("Borrow vault has insufficient liquidity")]
    InsufficientLiquidity,
    #[msg("Repay amount exceeds outstanding debt")]
    RepayExceedsDebt,
    #[msg("Credentials don't match")]
    CredentialMismatch,
    #[msg("Trying to mint invalid token")]
    InvalidMint,
    #[msg("Position is not open right now")]
    PositionNotOpened,
    #[msg("You still have debts")]
    BorrowAlive,
    #[msg("Cannot liquidate your own position")]
    CannotLiquidateSelf,
    #[msg("Invalid Oracle")]
    InvalidOracle,
    #[msg("Invalid or stale oracle price")]
    InvalidOraclePrice,
    #[msg("Pool is paused already")]
    AlreadyPaused,
    #[msg("Pool is open already")]
    AlreadyUnpaused,
    #[msg("Pool is paused")]
    Paused,
}
