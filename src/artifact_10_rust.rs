use anchor_lang::prelude::*;
use crate::errors::CastleDaoError;
use crate::state::*;

#[derive(Accounts)]
#[instruction(castle_id: u64)]
pub struct UpgradeFortress<'info> {
    #[account(
        mut,
        seeds = [b"castle", castle_id.to_le_bytes().as_ref()],
        bump = castle.bump,
        constraint = castle.lord == lord.key() @ CastleDaoError::UnauthorizedLord,
        constraint = !castle.under_siege @ CastleDaoError::CastleUnderSiege,
    )]
    pub castle: Account<'info, Castle>,

    #[account(mut)]
    pub lord: Signer<'info>,
}

pub fn upgrade_fortress(ctx: Context<UpgradeFortress>, _castle_id: u64) -> Result<()> {
    let castle = &mut ctx.accounts.castle;
    castle.level = castle.level.checked_add(1).ok_or(CastleDaoError::MathOverflow)?;
    castle.defense_power = castle
        .defense_power
        .checked_add(500)
        .ok_or(CastleDaoError::MathOverflow)?;
    Ok(())
}