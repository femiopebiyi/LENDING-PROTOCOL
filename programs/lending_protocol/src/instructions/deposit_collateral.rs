use anchor_lang::prelude::*;
use anchor_spl::{
    associated_token::AssociatedToken,
    token_interface::{transfer_checked, Mint, TokenAccount, TokenInterface, TransferChecked},
};

use crate::state::{LendingPool, UserPosition};
use crate::{errors::LendingError, helpers::accrue_interest};

#[derive(Accounts)]
pub struct DepositCollateral<'info> {
    #[account(mut)]
    pub depositor: Signer<'info>,

    #[account(
        mut,
        seeds = [b"lendingpool", pool.owner.as_ref(), pool.seed.to_le_bytes().as_ref()],
        bump = pool.bump
    )]
    pub pool: Account<'info, LendingPool>,

    #[account(
        mint::token_program = token_program,
        address = pool.collateral_mint @ LendingError::InvalidMint
    )]
    pub collateral_mint: InterfaceAccount<'info, Mint>,

    #[account(
        mut,
        associated_token::mint = collateral_mint,
        associated_token::authority = pool,
        associated_token::token_program = token_program,
    )]
    pub collateral_vault: InterfaceAccount<'info, TokenAccount>,

    #[account(
        mut,
        associated_token::token_program = token_program,
        associated_token::authority = depositor,
        associated_token::mint = collateral_mint,
    )]
    pub user_ata_collateral: InterfaceAccount<'info, TokenAccount>,

    #[account(
        init_if_needed,
        payer = depositor,
        space = 8 + UserPosition::INIT_SPACE,
        seeds = [b"userposition", pool.key().as_ref(), depositor.key().as_ref()],
        bump
    )]
    pub user_position: Account<'info, UserPosition>,

    pub associated_token_program: Program<'info, AssociatedToken>,
    pub token_program: Interface<'info, TokenInterface>,
    pub system_program: Program<'info, System>,
}

impl<'info> DepositCollateral<'info> {
    fn deposit_collateral(&mut self, amount: u64, bumps: &DepositCollateralBumps) -> Result<()> {
        require!(!self.pool.is_paused, LendingError::Paused);
        require!(amount > 0, LendingError::InvalidAmount);

        let clock = Clock::get()?;
        // 2. Initialize position on first deposit
        if !self.user_position.is_open {
            self.user_position.is_open = true;
            self.user_position.owner = self.depositor.key();
            self.user_position.pool = self.pool.key();
            self.user_position.collateral_deposited = 0;
            self.user_position.borrowed_amount = 0;
            self.user_position.interest_accrued = 0;
            self.user_position.last_update_time = clock.unix_timestamp;
            self.user_position.bump = bumps.user_position;
        } else {
            accrue_interest(
                &mut self.user_position,
                clock.unix_timestamp,
                self.pool.interest_rate,
            )?;
        }

        transfer_checked(
            CpiContext::new(
                self.token_program.to_account_info(),
                TransferChecked {
                    from: self.user_ata_collateral.to_account_info(),
                    to: self.collateral_vault.to_account_info(),
                    mint: self.collateral_mint.to_account_info(),
                    authority: self.depositor.to_account_info(),
                },
            ),
            amount,
            self.collateral_mint.decimals,
        )?;

        self.user_position.collateral_deposited = self
            .user_position
            .collateral_deposited
            .checked_add(amount)
            .ok_or(LendingError::Overflow)?;

        self.pool.total_collateral = self
            .pool
            .total_collateral
            .checked_add(amount)
            .ok_or(LendingError::Overflow)?;

        Ok(())
    }
}

pub fn deposit_collateral_handler(ctx: Context<DepositCollateral>, amount: u64) -> Result<()> {
    ctx.accounts.deposit_collateral(amount, &ctx.bumps)
}
