use anchor_lang::prelude::*;

#[account]
pub struct RealmConfig {
    pub admin: Pubkey,
    pub ruby_mint: Pubkey,
    pub base_production_rate: u64, // Micro-tokens per second per level
    pub siege_duration: i64,       // Cooldown window in seconds
    pub bump: u8,
    pub mint_bump: u8,
}

impl RealmConfig {
    pub const LEN: usize = 8 + 32 + 32 + 8 + 8 + 1 + 1;
}

#[account]
pub struct Castle {
    pub lord: Pubkey,
    pub castle_id: u64,
    pub level: u8,
    pub defense_power: u64,
    pub last_harvest_ts: i64,
    pub under_siege: bool,
    pub bump: u8,
}

impl Castle {
    pub const LEN: usize = 8 + 32 + 8 + 1 + 8 + 8 + 1 + 1;
}

#[account]
pub struct SiegeSession {
    pub castle_id: u64,
    pub attacker: Pubkey,
    pub war_chest: u64,
    pub start_timestamp: i64,
    pub resolved: bool,
    pub bump: u8,
}

impl SiegeSession {
    pub const LEN: usize = 8 + 8 + 32 + 8 + 8 + 1 + 1;
}