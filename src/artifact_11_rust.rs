use anchor_lang::prelude::*;
use crate::errors::CastleError;
use crate::state::{FortressState, FORTRESS_STATE_SEED};

#[derive(Accounts)]
pub struct DecayFortress<'info> {
    #[account(mut)]
    pub cranker: Signer<'info>,

    #[account(
        mut,
        seeds = [FORTRESS_STATE_SEED, fortress_state.guild_id.to_le_bytes().as_ref()],
        bump = fortress_state.bump
    )]
    pub fortress_state: Account<'info, FortressState>,
}

pub fn handler(ctx: Context<DecayFortress>) -> Result<()> {
    let fortress = &mut ctx.accounts.fortress_state;
    let current_slot = Clock::get()?.slot;

    let slots_elapsed = current_slot.saturating_sub(fortress.last_decay_slot);
    // Decay triggers every 1,000 slots (~6.6 minutes)
    if slots_elapsed >= 1000 {
        let decay_units = (slots_elapsed / 1000) as u32;
        let hp_reduction = decay_units.checked_mul(10).unwrap_or(u32::MAX); // 10 HP per epoch

        fortress.health_points = fortress.health_points.saturating_sub(hp_reduction);
        fortress.last_decay_slot = current_slot;

        msg!(
            "Fortress crank executed. Slots elapsed: {}. Remaining HP: {}",
            slots_elapsed,
            fortress.health_points
        );
    }

    Ok(())
}