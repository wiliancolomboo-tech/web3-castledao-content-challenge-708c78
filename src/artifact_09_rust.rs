use anchor_lang::prelude::*;
use anchor_spl::token::{transfer, Token, TokenAccount, Transfer};
use crate::errors::CastleDaoError;
use crate::state::*;

#[derive(Accounts)]
#[instruction(castle_id: u64)]
pub struct InitiateSiege<'info> {
    #[account(
        seeds = [b"realm_config"],
        bump = realm_config.bump,
    )]
    pub realm_config: Account<'info, RealmConfig>,

    #[account(
        mut,
        seeds = [b"castle", castle_id.to_le_bytes().as_ref()],
        bump = castle.bump,
        constraint = !castle.under_siege @ CastleDaoError::CastleUnderSiege,
    )]
    pub castle: Account<'info, Castle>,

    #[account(
        init,
        payer = attacker,
        space = SiegeSession::LEN,
        seeds = [b"siege", castle_id.to_le_bytes().as_ref()],
        bump
    )]
    pub siege_session: Account<'info, SiegeSession>,

    #[account(
        mut,
        constraint = attacker_token_vault.owner == attacker.key(),
    )]
    pub attacker_token_vault: Account<'info, TokenAccount>,

    #[account(
        mut,
        seeds = [b"escrow", castle_id.to_le_bytes().as_ref()],
        bump,
    )]
    pub siege_escrow_vault: Account<'info, TokenAccount>,

    #[account(mut)]
    pub attacker: Signer<'info>,

    pub token_program: Program<'info, Token>,
    pub system_program: Program<'info, System>,
    pub clock: Sysvar<'info, Clock>,
}

pub fn initiate_siege(
    ctx: Context<InitiateSiege>,
    castle_id: u64,
    war_chest_amount: u64,
) -> Result<()> {
    require!(war_chest_amount >= 1_000_000, CastleDaoError::InsufficientWarChest);

    // Escrow attacker's war chest
    let cpi_accounts = Transfer {
        from: ctx.accounts.attacker_token_vault.to_account_info(),
        to: ctx.accounts.siege_escrow_vault.to_account_info(),
        authority: ctx.accounts.attacker.to_account_info(),
    };
    let cpi_ctx = CpiContext::new(ctx.accounts.token_program.to_account_info(), cpi_accounts);
    transfer(cpi_ctx, war_chest_amount)?;

    let castle = &mut ctx.accounts.castle;
    castle.under_siege = true;

    let siege = &mut ctx.accounts.siege_session;
    siege.castle_id = castle_id;
    siege.attacker = ctx.accounts.attacker.key();
    siege.war_chest = war_chest_amount;
    siege.start_timestamp = ctx.accounts.clock.unix_timestamp;
    siege.resolved = false;
    siege.bump = ctx.bumps.siege_session;

    Ok(())
}

#[derive(Accounts)]
#[instruction(castle_id: u64)]
pub struct ResolveSiege<'info> {
    #[account(
        seeds = [b"realm_config"],
        bump = realm_config.bump,
    )]
    pub realm_config: Account<'info, RealmConfig>,

    #[account(
        mut,
        seeds = [b"castle", castle_id.to_le_bytes().as_ref()],
        bump = castle.bump,
        constraint = castle.under_siege @ CastleDaoError::CastleNotInSiege,
    )]
    pub castle: Account<'info, Castle>,

    #[account(
        mut,
        seeds = [b"siege", castle_id.to_le_bytes().as_ref()],
        bump = siege_session.bump,
        close = castle_lord, // Rent returned upon conclusion
    )]
    pub siege_session: Account<'info, SiegeSession>,

    #[account(mut)]
    /// CHECK: Validated against current castle lord
    pub castle_lord: AccountInfo<'info>,

    #[account(mut)]
    /// CHECK: Validated against attacker key stored in siege_session
    pub attacker: AccountInfo<'info>,

    #[account(
        mut,
        seeds = [b"escrow", castle_id.to_le_bytes().as_ref()],
        bump,
    )]
    pub siege_escrow_vault: Account<'info, TokenAccount>,

    #[account(mut)]
    pub defender_token_account: Account<'info, TokenAccount>,

    pub token_program: Program<'info, Token>,
    pub clock: Sysvar<'info, Clock>,
}

pub fn resolve_siege(ctx: Context<ResolveSiege>, castle_id: u64) -> Result<()> {
    let now = ctx.accounts.clock.unix_timestamp;
    let config = &ctx.accounts.realm_config;
    let siege = &ctx.accounts.siege_session;
    let castle = &mut ctx.accounts.castle;

    require!(
        now >= siege.start_timestamp + config.siege_duration,
        CastleDaoError::SiegeStillActive
    );
    require!(castle.lord == ctx.accounts.castle_lord.key(), CastleDaoError::UnauthorizedLord);
    require!(siege.attacker == ctx.accounts.attacker.key(), CastleDaoError::UnauthorizedLord);

    // Combat Resolution Model:
    // If war_chest > (defense_power * 100), attacker overpowers fortress.
    // Otherwise defender holds, and slashed war chest transfers to defender.
    let conquest_threshold = castle.defense_power.checked_mul(100).ok_or(CastleDaoError::MathOverflow)?;

    let castle_id_bytes = castle_id.to_le_bytes();
    let escrow_seeds = &[b"escrow", castle_id_bytes.as_ref(), &[ctx.bumps.siege_escrow_vault]];
    let signer = &[&escrow_seeds[..]];

    if siege.war_chest > conquest_threshold {
        // Attacker victory: Transfer castle lordship
        castle.lord = siege.attacker;
        castle.defense_power = castle.defense_power / 2; // Defense halved in battle
        castle.under_siege = false;

        // Attacker claims back their surviving war chest
        let cpi_accounts = Transfer {
            from: ctx.accounts.siege_escrow_vault.to_account_info(),
            to: ctx.accounts.defender_token_account.to_account_info(), // Passed in attacker's receiver
            authority: ctx.accounts.siege_escrow_vault.to_account_info(),
        };
        let cpi_ctx = CpiContext::new_with_signer(
            ctx.accounts.token_program.to_account_info(),
            cpi_accounts,
            signer,
        );
        transfer(cpi_ctx, siege.war_chest)?;
    } else {
        // Defender victory: Attacker war chest slashed to Defender
        castle.under_siege = false;
        let cpi_accounts = Transfer {
            from: ctx.accounts.siege_escrow_vault.to_account_info(),
            to: ctx.accounts.defender_token_account.to_account_info(),
            authority: ctx.accounts.siege_escrow_vault.to_account_info(),
        };
        let cpi_ctx = CpiContext::new_with_signer(
            ctx.accounts.token_program.to_account_info(),
            cpi_accounts,
            signer,
        );
        transfer(cpi_ctx, siege.war_chest)?;
    }

    Ok(())
}