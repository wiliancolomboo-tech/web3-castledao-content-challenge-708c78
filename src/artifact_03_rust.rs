use anchor_lang::prelude::*;
use anchor_spl::token::{self, Mint, Token, TokenAccount, Transfer};

declare_id!("Cast1eDaoDynamicContentEngine11111111111");

pub const MAX_REALM_NAME_LEN: usize = 32;
pub const CONTENT_HASH_LEN: usize = 32;

#[program]
pub mod castle_dao {
    use super::*;

    /// Initializes a new Realm governance domain.
    pub fn initialize_realm(
        ctx: Context<InitializeRealm>,
        realm_name: String,
        protocol_fee_bps: u16,
    ) -> Result<()> {
        require!(
            realm_name.len() <= MAX_REALM_NAME_LEN,
            CastleDaoError::NameTooLong
        );
        require!(
            protocol_fee_bps <= 1000, // Max 10%
            CastleDaoError::InvalidFeeConfiguration
        );

        let realm = &mut ctx.accounts.realm;
        realm.authority = ctx.accounts.authority.key();
        realm.mint = ctx.accounts.mint.key();
        realm.realm_name = realm_name;
        realm.castle_count = 0;
        realm.protocol_fee_bps = protocol_fee_bps;
        realm.bump = ctx.bumps.realm;

        emit!(RealmCreatedEvent {
            realm: realm.key(),
            authority: realm.authority,
            mint: realm.mint,
        });

        Ok(())
    }

    /// Initializes a specific Castle territory under a Realm.
    pub fn create_castle(
        ctx: Context<CreateCastle>,
        castle_id: u64,
        defense_capacity: u64,
    ) -> Result<()> {
        let realm = &mut ctx.accounts.realm;
        let castle = &mut ctx.accounts.castle;

        castle.realm = realm.key();
        castle.lord = ctx.accounts.lord.key();
        castle.castle_id = castle_id;
        castle.defense_power = 0;
        castle.defense_capacity = defense_capacity;
        castle.active_challenges = 0;
        castle.vault = ctx.accounts.castle_vault.key();
        castle.bump = ctx.bumps.castle;

        realm.castle_count = realm
            .castle_count
            .checked_add(1)
            .ok_or(CastleDaoError::ArithmeticOverflow)?;

        emit!(CastleCreatedEvent {
            castle: castle.key(),
            lord: castle.lord,
            castle_id,
        });

        Ok(())
    }

    /// Staking mechanism: Increases defense power by locking governance/realm tokens.
    pub fn stake_defense(ctx: Context<StakeDefense>, amount: u64) -> Result<()> {
        require!(amount > 0, CastleDaoError::ZeroTransferAmount);

        let castle = &mut ctx.accounts.castle;
        let new_power = castle
            .defense_power
            .checked_add(amount)
            .ok_or(CastleDaoError::ArithmeticOverflow)?;

        require!(
            new_power <= castle.defense_capacity,
            CastleDaoError::DefenseCapacityExceeded
        );

        // Transfer tokens to the castle PDA vault
        let cpi_accounts = Transfer {
            from: ctx.accounts.lord_token_account.to_account_info(),
            to: ctx.accounts.castle_vault.to_account_info(),
            authority: ctx.accounts.lord.to_account_info(),
        };
        let cpi_ctx = CpiContext::new(ctx.accounts.token_program.to_account_info(), cpi_accounts);
        token::transfer(cpi_ctx, amount)?;

        castle.defense_power = new_power;

        emit!(DefenseStakedEvent {
            castle: castle.key(),
            amount,
            new_defense_power: new_power,
        });

        Ok(())
    }

    /// Launches a content challenge with an escrowed reward purse.
    pub fn launch_challenge(
        ctx: Context<LaunchChallenge>,
        challenge_id: u64,
        reward_amount: u64,
        duration_slots: u64,
        target_score_threshold: u32,
    ) -> Result<()> {
        require!(reward_amount > 0, CastleDaoError::ZeroTransferAmount);
        require!(duration_slots > 0, CastleDaoError::InvalidDuration);

        let castle = &mut ctx.accounts.castle;
        let challenge = &mut ctx.accounts.challenge;
        let current_slot = Clock::get()?.slot;

        challenge.castle = castle.key();
        challenge.challenge_id = challenge_id;
        challenge.bounty_amount = reward_amount;
        challenge.target_score_threshold = target_score_threshold;
        challenge.expiry_slot = current_slot
            .checked_add(duration_slots)
            .ok_or(CastleDaoError::ArithmeticOverflow)?;
        challenge.settled = false;
        challenge.winner = Pubkey::default();
        challenge.bump = ctx.bumps.challenge;

        castle.active_challenges = castle
            .active_challenges
            .checked_add(1)
            .ok_or(CastleDaoError::ArithmeticOverflow)?;

        // Escrow funds from creator to Castle Vault
        let cpi_accounts = Transfer {
            from: ctx.accounts.creator_token_account.to_account_info(),
            to: ctx.accounts.castle_vault.to_account_info(),
            authority: ctx.accounts.creator.to_account_info(),
        };
        let cpi_ctx = CpiContext::new(ctx.accounts.token_program.to_account_info(), cpi_accounts);
        token::transfer(cpi_ctx, reward_amount)?;

        emit!(ChallengeLaunchedEvent {
            castle: castle.key(),
            challenge_id,
            bounty_amount: reward_amount,
            expiry_slot: challenge.expiry_slot,
        });

        Ok(())
    }

    /// Submits a cryptographic hash of the content (IPFS CID digest) for evaluation.
    pub fn submit_content_entry(
        ctx: Context<SubmitContentEntry>,
        content_hash: [u8; CONTENT_HASH_LEN],
    ) -> Result<()> {
        let challenge = &ctx.accounts.challenge;
        let current_slot = Clock::get()?.slot;

        require!(
            current_slot <= challenge.expiry_slot,
            CastleDaoError::ChallengeExpired
        );
        require!(!challenge.settled, CastleDaoError::ChallengeAlreadySettled);

        let entry = &mut ctx.accounts.entry;
        entry.challenge = challenge.key();
        entry.contender = ctx.accounts.contender.key();
        entry.content_hash = content_hash;
        entry.score = 0;
        entry.evaluated = false;
        entry.bump = ctx.bumps.entry;

        emit!(ContentSubmittedEvent {
            challenge: challenge.key(),
            contender: entry.contender,
            content_hash,
        });

        Ok(())
    }

    /// Settles the challenge, verifies scoring, and dispatches the token reward via PDA CPI.
    pub fn settle_and_distribute(
        ctx: Context<SettleAndDistribute>,
        verified_score: u32,
    ) -> Result<()> {
        let challenge = &mut ctx.accounts.challenge;
        let entry = &mut ctx.accounts.entry;
        let castle = &mut ctx.accounts.castle;

        require!(!challenge.settled, CastleDaoError::ChallengeAlreadySettled);
        require!(
            verified_score >= challenge.target_score_threshold,
            CastleDaoError::ScoreBelowThreshold
        );

        entry.score = verified_score;
        entry.evaluated = true;

        challenge.settled = true;
        challenge.winner = entry.contender;

        castle.active_challenges = castle
            .active_challenges
            .checked_sub(1)
            .ok_or(CastleDaoError::ArithmeticOverflow)?;

        // Execute PDA transfer from Castle Vault to Contender
        let realm_key = castle.realm;
        let castle_id_bytes = castle.castle_id.to_le_bytes();
        let castle_bump = &[castle.bump];
        let seeds: &[&[&[u8]]] = &[&[
            b"castle",
            realm_key.as_ref(),
            castle_id_bytes.as_ref(),
            castle_bump,
        ]];

        let cpi_accounts = Transfer {
            from: ctx.accounts.castle_vault.to_account_info(),
            to: ctx.accounts.contender_token_account.to_account_info(),
            authority: castle.to_account_info(),
        };
        let cpi_ctx = CpiContext::new_with_signer(
            ctx.accounts.token_program.to_account_info(),
            cpi_accounts,
            seeds,
        );

        token::transfer(cpi_ctx, challenge.bounty_amount)?;

        emit!(ChallengeSettledEvent {
            challenge: challenge.key(),
            winner: entry.contender,
            payout: challenge.bounty_amount,
            score: verified_score,
        });

        Ok(())
    }
}

// -----------------------------------------------------------------------------
// ACCOUNT VALIDATION STRUCTS
// -----------------------------------------------------------------------------

#[derive(Accounts)]
#[instruction(realm_name: String)]
pub struct InitializeRealm<'info> {
    #[account(
        init,
        payer = authority,
        space = 8 + RealmConfig::LEN,
        seeds = [b"realm", realm_name.as_bytes()],
        bump
    )]
    pub realm: Account<'info, RealmConfig>,
    pub mint: Account<'info, Mint>,
    #[account(mut)]
    pub authority: Signer<'info>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
#[instruction(castle_id: u64)]
pub struct CreateCastle<'info> {
    #[account(
        mut,
        seeds = [b"realm", realm.realm_name.as_bytes()],
        bump = realm.bump,
    )]
    pub realm: Account<'info, RealmConfig>,
    #[account(
        init,
        payer = lord,
        space = 8 + CastleAccount::LEN,
        seeds = [b"castle", realm.key().as_ref(), &castle_id.to_le_bytes()],
        bump
    )]
    pub castle: Account<'info, CastleAccount>,
    #[account(
        init,
        payer = lord,
        seeds = [b"vault", castle.key().as_ref()],
        bump,
        token::mint = mint,
        token::authority = castle,
    )]
    pub castle_vault: Account<'info, TokenAccount>,
    pub mint: Account<'info, Mint>,
    #[account(mut)]
    pub lord: Signer<'info>,
    pub token_program: Program<'info, Token>,
    pub system_program: Program<'info, System>,
    pub rent: Sysvar<'info, Rent>,
}

#[derive(Accounts)]
pub struct StakeDefense<'info> {
    #[account(
        mut,
        seeds = [b"castle", castle.realm.as_ref(), &castle.castle_id.to_le_bytes()],
        bump = castle.bump,
        has_one = lord,
        has_one = vault @ CastleDaoError::InvalidVaultAccount
    )]
    pub castle: Account<'info, CastleAccount>,
    #[account(mut)]
    pub castle_vault: Account<'info, TokenAccount>,
    #[account(mut)]
    pub lord_token_account: Account<'info, TokenAccount>,
    pub lord: Signer<'info>,
    pub token_program: Program<'info, Token>,
}

#[derive(Accounts)]
#[instruction(challenge_id: u64)]
pub struct LaunchChallenge<'info> {
    #[account(
        mut,
        seeds = [b"castle", castle.realm.as_ref(), &castle.castle_id.to_le_bytes()],
        bump = castle.bump,
        has_one = vault @ CastleDaoError::InvalidVaultAccount
    )]
    pub castle: Account<'info, CastleAccount>,
    #[account(
        init,
        payer = creator,
        space = 8 + ChallengeAccount::LEN,
        seeds = [b"challenge", castle.key().as_ref(), &challenge_id.to_le_bytes()],
        bump
    )]
    pub challenge: Account<'info, ChallengeAccount>,
    #[account(mut)]
    pub castle_vault: Account<'info, TokenAccount>,
    #[account(mut)]
    pub creator_token_account: Account<'info, TokenAccount>,
    #[account(mut)]
    pub creator: Signer<'info>,
    pub token_program: Program<'info, Token>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct SubmitContentEntry<'info> {
    #[account(
        seeds = [b"challenge", challenge.castle.as_ref(), &challenge.challenge_id.to_le_bytes()],
        bump = challenge.bump
    )]
    pub challenge: Account<'info, ChallengeAccount>,
    #[account(
        init,
        payer = contender,
        space = 8 + EntryAccount::LEN,
        seeds = [b"entry", challenge.key().as_ref(), contender.key().as_ref()],
        bump
    )]
    pub entry: Account<'info, EntryAccount>,
    #[account(mut)]
    pub contender: Signer<'info>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct SettleAndDistribute<'info> {
    #[account(
        mut,
        seeds = [b"castle", castle.realm.as_ref(), &castle.castle_id.to_le_bytes()],
        bump = castle.bump,
        has_one = vault @ CastleDaoError::InvalidVaultAccount
    )]
    pub castle: Account<'info, CastleAccount>,
    #[account(
        mut,
        seeds = [b"challenge", castle.key().as_ref(), &challenge.challenge_id.to_le_bytes()],
        bump = challenge.bump,
        has_one = castle @ CastleDaoError::InvalidCastleAccount
    )]
    pub challenge: Account<'info, ChallengeAccount>,
    #[account(
        mut,
        seeds = [b"entry", challenge.key().as_ref(), entry.contender.as_ref()],
        bump = entry.bump,
        has_one = challenge @ CastleDaoError::InvalidChallengeAccount
    )]
    pub entry: Account<'info, EntryAccount>,
    #[account(mut)]
    pub castle_vault: Account<'info, TokenAccount>,
    #[account(
        mut,
        constraint = contender_token_account.owner == entry.contender @ CastleDaoError::InvalidRecipient
    )]
    pub contender_token_account: Account<'info, TokenAccount>,
    /// Lord or assigned oracle validator must sign settlement.
    #[account(
        constraint = verifier.key() == castle.lord @ CastleDaoError::UnauthorizedVerifier
    )]
    pub verifier: Signer<'info>,
    pub token_program: Program<'info, Token>,
}

// -----------------------------------------------------------------------------
// STATE STRUCTURES
// -----------------------------------------------------------------------------

#[account]
pub struct RealmConfig {
    pub authority: Pubkey,       // 32
    pub mint: Pubkey,            // 32
    pub realm_name: String,      // 4 + 32
    pub castle_count: u64,       // 8
    pub protocol_fee_bps: u16,   // 2
    pub bump: u8,                // 1
}

impl RealmConfig {
    pub const LEN: usize = 32 + 32 + (4 + MAX_REALM_NAME_LEN) + 8 + 2 + 1;
}

#[account]
pub struct CastleAccount {
    pub realm: Pubkey,           // 32
    pub lord: Pubkey,            // 32
    pub vault: Pubkey,           // 32
    pub castle_id: u64,          // 8
    pub defense_power: u64,      // 8
    pub defense_capacity: u64,   // 8
    pub active_challenges: u32,  // 4
    pub bump: u8,                // 1
}

impl CastleAccount {
    pub const LEN: usize = 32 + 32 + 32 + 8 + 8 + 8 + 4 + 1;
}

#[account]
pub struct ChallengeAccount {
    pub castle: Pubkey,          // 32
    pub winner: Pubkey,          // 32
    pub challenge_id: u64,       // 8
    pub bounty_amount: u64,      // 8
    pub expiry_slot: u64,        // 8
    pub target_score_threshold: u32, // 4
    pub settled: bool,           // 1
    pub bump: u8,                // 1
}

impl ChallengeAccount {
    pub const LEN: usize = 32 + 32 + 8 + 8 + 8 + 4 + 1 + 1;
}

#[account]
pub struct EntryAccount {
    pub challenge: Pubkey,       // 32
    pub contender: Pubkey,       // 32
    pub content_hash: [u8; 32],  // 32
    pub score: u32,              // 4
    pub evaluated: bool,         // 1
    pub bump: u8,                // 1
}

impl EntryAccount {
    pub const LEN: usize = 32 + 32 + 32 + 4 + 1 + 1;
}

// -----------------------------------------------------------------------------
// EVENTS & ERRORS
// -----------------------------------------------------------------------------

#[event]
pub struct RealmCreatedEvent {
    pub realm: Pubkey,
    pub authority: Pubkey,
    pub mint: Pubkey,
}

#[event]
pub struct CastleCreatedEvent {
    pub castle: Pubkey,
    pub lord: Pubkey,
    pub castle_id: u64,
}

#[event]
pub struct DefenseStakedEvent {
    pub castle: Pubkey,
    pub amount: u64,
    pub new_defense_power: u64,
}

#[event]
pub struct ChallengeLaunchedEvent {
    pub castle: Pubkey,
    pub challenge_id: u64,
    pub bounty_amount: u64,
    pub expiry_slot: u64,
}

#[event]
pub struct ContentSubmittedEvent {
    pub challenge: Pubkey,
    pub contender: Pubkey,
    pub content_hash: [u8; 32],
}

#[event]
pub struct ChallengeSettledEvent {
    pub challenge: Pubkey,
    pub winner: Pubkey,
    pub payout: u64,
    pub score: u32,
}

#[error_code]
pub enum CastleDaoError {
    #[msg("Realm name exceeds allowed 32-byte bound.")]
    NameTooLong,
    #[msg("Protocol fee exceeds maximum allowed basis points (1000).")]
    InvalidFeeConfiguration,
    #[msg("Arithmetic overflow detected during integer operation.")]
    ArithmeticOverflow,
    #[msg("Transfer amount must be strictly greater than zero.")]
    ZeroTransferAmount,
    #[msg("Defense capacity would be exceeded by this deposit.")]
    DefenseCapacityExceeded,
    #[msg("Invalid duration provided for challenge.")]
    InvalidDuration,
    #[msg("Challenge expiry slot has elapsed.")]
    ChallengeExpired,
    #[msg("Challenge has already been settled.")]
    ChallengeAlreadySettled,
    #[msg("Score achieved is below minimum target threshold.")]
    ScoreBelowThreshold,
    #[msg("Castle vault token account does not match expected PDA.")]
    InvalidVaultAccount,
    #[msg("Specified castle does not match challenge target.")]
    InvalidCastleAccount,
    #[msg("Specified challenge does not match entry target.")]
    InvalidChallengeAccount,
    #[msg("Contender token account owner mismatch.")]
    InvalidRecipient,
    #[msg("Signer is not authorized to verify challenge scores.")]
    UnauthorizedVerifier,
}