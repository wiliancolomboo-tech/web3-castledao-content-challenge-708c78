use anchor_lang::prelude::*;
use anchor_spl::token::{mint_to, Mint, MintTo, Token, TokenAccount};
use crate::errors::CastleDaoError;
use crate::state::*;

#[derive(Accounts)]
#[instruction(castle_id: u64)]
pub struct HarvestResources<'info> {
    #[account(
        seeds = [b"realm_config"],
        bump = realm_config.bump,
        has_one = ruby_mint,
    )]
    pub realm_config: Account<'info, RealmConfig>,

    #[account(
        mut,
        seeds = [b"castle", castle_id.to_le_bytes().as_ref()],
        bump = castle.bump,
        constraint = castle.lord == lord.key() @ CastleDaoError::UnauthorizedLord,
        constraint = !castle.under_siege @ CastleDaoError::CastleUnderSiege,
    )]
    pub castle: Account<'info, Castle>,

    #[account(
        mut,
        seeds = [b"ruby_mint"],
        bump = realm_config.mint_bump,
    )]
    pub ruby_mint: Account<'info, Mint>,

    #[account(
        mut,
        constraint = user_token_account.owner == lord.key(),
        constraint = user_token_account.mint == realm_config.ruby_mint,
    )]
    pub user_token_account: Account<'info, TokenAccount>,

    #[account(mut)]
    pub lord: Signer<'info>,

    pub token_program: Program<'info, Token>,
    pub clock: Sysvar<'info, Clock>,
}

pub fn harvest_resources(ctx: Context<HarvestResources>, _castle_id: u64) -> Result<()> {
    let now = ctx.accounts.clock.unix_timestamp;
    let castle = &mut ctx.accounts.castle;
    let config = &ctx.accounts.realm_config;

    let time_delta = now
        .checked_sub(castle.last_harvest_ts)
        .ok_or(CastleDaoError::MathOverflow)?;

    require!(time_delta > 0, CastleDaoError::ZeroClaimableResources);

    // Yield = time_delta * base_production_rate * level
    let yield_amount = (time_delta as u64)
        .checked_mul(config.base_production_rate)
        .and_then(|val| val.checked_mul(castle.level as u64))
        .ok_or(CastleDaoError::MathOverflow)?;

    castle.last_harvest_ts = now;

    // Execute Mint CPI using Ruby Mint PDA seeds
    let mint_bump = config.mint_bump;
    let seeds = &[b"ruby_mint".as_ref(), &[mint_bump]];
    let signer = &[&seeds[..]];

    let cpi_accounts = MintTo {
        mint: ctx.accounts.ruby_mint.to_account_info(),
        to: ctx.accounts.user_token_account.to_account_info(),
        authority: ctx.accounts.ruby_mint.to_account_info(),
    };

    let cpi_ctx = CpiContext::new_with_signer(
        ctx.accounts.token_program.to_account_info(),
        cpi_accounts,
        signer,
    );

    mint_to(cpi_ctx, yield_amount)?;
    Ok(())
}