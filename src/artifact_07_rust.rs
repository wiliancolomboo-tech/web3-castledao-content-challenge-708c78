use anchor_lang::prelude::*;
use anchor_spl::token::{Mint, Token};
use crate::state::*;

#[derive(Accounts)]
pub struct InitializeRealm<'info> {
    #[account(
        init,
        payer = admin,
        space = RealmConfig::LEN,
        seeds = [b"realm_config"],
        bump
    )]
    pub realm_config: Account<'info, RealmConfig>,

    #[account(
        init,
        payer = admin,
        seeds = [b"ruby_mint"],
        bump,
        mint::decimals = 6,
        mint::authority = ruby_mint,
    )]
    pub ruby_mint: Account<'info, Mint>,

    #[account(mut)]
    pub admin: Signer<'info>,

    pub system_program: Program<'info, System>,
    pub token_program: Program<'info, Token>,
    pub rent: Sysvar<'info, Rent>,
}

pub fn initialize_realm(
    ctx: Context<InitializeRealm>,
    base_production_rate: u64,
    siege_duration: i64,
) -> Result<()> {
    let config = &mut ctx.accounts.realm_config;
    config.admin = ctx.accounts.admin.key();
    config.ruby_mint = ctx.accounts.ruby_mint.key();
    config.base_production_rate = base_production_rate;
    config.siege_duration = siege_duration;
    config.bump = ctx.bumps.realm_config;
    config.mint_bump = ctx.bumps.ruby_mint;
    Ok(())
}

#[derive(Accounts)]
#[instruction(castle_id: u64)]
pub struct ClaimCastle<'info> {
    #[account(
        init,
        payer = lord,
        space = Castle::LEN,
        seeds = [b"castle", castle_id.to_le_bytes().as_ref()],
        bump
    )]
    pub castle: Account<'info, Castle>,

    #[account(mut)]
    pub lord: Signer<'info>,

    pub system_program: Program<'info, System>,
    pub clock: Sysvar<'info, Clock>,
}

pub fn claim_castle(
    ctx: Context<ClaimCastle>,
    castle_id: u64,
    initial_defense: u64,
) -> Result<()> {
    let castle = &mut ctx.accounts.castle;
    castle.lord = ctx.accounts.lord.key();
    castle.castle_id = castle_id;
    castle.level = 1;
    castle.defense_power = initial_defense;
    castle.last_harvest_ts = ctx.accounts.clock.unix_timestamp;
    castle.under_siege = false;
    castle.bump = ctx.bumps.castle;
    Ok(())
}