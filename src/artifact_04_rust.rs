use anchor_lang::prelude::*;

pub mod errors;
pub mod instructions;
pub mod state;

use instructions::*;

declare_id!("CasTLeDaoEng1ne1111111111111111111111111111");

#[program]
pub mod castledao_engine {
    use super::*;

    pub fn initialize_realm(
        ctx: Context<InitializeRealm>,
        base_production_rate: u64,
        siege_duration: i64,
    ) -> Result<()> {
        instructions::initialize::initialize_realm(ctx, base_production_rate, siege_duration)
    }

    pub fn claim_castle(
        ctx: Context<ClaimCastle>,
        castle_id: u64,
        initial_defense: u64,
    ) -> Result<()> {
        instructions::initialize::claim_castle(ctx, castle_id, initial_defense)
    }

    pub fn upgrade_fortress(
        ctx: Context<UpgradeFortress>,
        castle_id: u64,
    ) -> Result<()> {
        instructions::upgrade::upgrade_fortress(ctx, castle_id)
    }

    pub fn harvest_resources(
        ctx: Context<HarvestResources>,
        castle_id: u64,
    ) -> Result<()> {
        instructions::harvest::harvest_resources(ctx, castle_id)
    }

    pub fn initiate_siege(
        ctx: Context<InitiateSiege>,
        castle_id: u64,
        war_chest_amount: u64,
    ) -> Result<()> {
        instructions::siege::initiate_siege(ctx, castle_id, war_chest_amount)
    }

    pub fn resolve_siege(
        ctx: Context<ResolveSiege>,
        castle_id: u64,
    ) -> Result<()> {
        instructions::siege::resolve_siege(ctx, castle_id)
    }
}