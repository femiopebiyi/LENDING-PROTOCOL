use anchor_lang::prelude::*;

use crate::errors::LendingError;
use crate::state::LendingPool;

#[derive(Accounts)]
pub struct UnpausePool<'info> {
    #[account(mut)]
    pub owner: Signer<'info>,

    #[account(
        seeds = [b"lendingpool", pool.owner.as_ref(), pool.seed.to_le_bytes().as_ref()],
        bump = pool.bump,
        has_one = owner @LendingError::CredentialMismatch
    )]
    pub pool: Account<'info, LendingPool>,
}

impl<'info> UnpausePool<'info> {
    fn unpause_pool(&mut self) -> Result<()> {
        require!(self.pool.is_paused == true, LendingError::AlreadyUnpaused);

        self.pool.is_paused = false;

        Ok(())
    }
}

pub fn unpause_pool_handler(ctx: Context<UnpausePool>) -> Result<()> {
    ctx.accounts.unpause_pool()
}
