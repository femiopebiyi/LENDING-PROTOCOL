use anchor_lang::prelude::*;

use crate::errors::LendingError;
use crate::state::LendingPool;

#[derive(Accounts)]
pub struct PausePool<'info> {
    #[account(mut)]
    pub owner: Signer<'info>,

    #[account(
        seeds = [b"lendingpool", pool.owner.as_ref(), pool.seed.to_le_bytes().as_ref()],
        bump = pool.bump,
        has_one = owner @LendingError::CredentialMismatch
    )]
    pub pool: Account<'info, LendingPool>,
}

impl<'info> PausePool<'info> {
    fn pause_pool(&mut self) -> Result<()> {
        require!(self.pool.is_paused == false, LendingError::AlreadyPaused);

        self.pool.is_paused = true;

        Ok(())
    }
}

pub fn pause_pool_handler(ctx: Context<PausePool>) -> Result<()> {
    ctx.accounts.pause_pool()
}
