use anchor_lang::prelude::*;
use anchor_spl::token::{self, Mint, Token, TokenAccount, Transfer};

declare_id!("CasTLeDaoEng1ne1111111111111111111111111111");

pub const PRECISION_FACTOR: u128 = 1_000_000_000;
pub const SECONDS_PER_DAY: u64 = 86_400;

#[program]
pub mod castle_dao_engine {
    use super::*;

    /// Initialize a new CastleDAO Realm Stronghold with defensive attributes
    pub fn initialize_realm(
        ctx: Context<InitializeRealm>,
        defense_rating: u32,
        base_reward_rate: u64,
        curation_threshold: u64,
        timelock_duration: i64,
    ) -> Result<()> {
        require!(defense_rating <= 10_000, CastleError::DefenseRatingExceeded);

        let realm = &mut ctx.accounts.realm;
        realm.authority = ctx.accounts.authority.key();
        realm.mint = ctx.accounts.token_mint.key();
        realm.vault = ctx.accounts.realm_vault.key();
        realm.defense_rating = defense_rating;
        realm.base_reward_rate = base_reward_rate;
        realm.curation_threshold = curation_threshold;
        realm.timelock_duration = timelock_duration;
        realm.total_staked = 0;
        realm.bounty_count = 0;
        realm.vault_authority_bump = ctx.bumps.vault_authority;
        realm.bump = ctx.bumps.realm;

        msg!("CastleDAO: Realm initialized. Defense Rating: {}", defense_rating);
        Ok(())
    }

    /// Stake recruits/knights into Castle Citadel to defend and accrue yield
    pub fn stake_recruit(ctx: Context<StakeRecruit>, amount: u64) -> Result<()> {
        require!(amount > 0, CastleError::ZeroStakeAmount);

        let recruit_stake = &mut ctx.accounts.recruit_stake;
        let clock = Clock::get()?;

        // If active stake exists, credit accrued rewards to yield backlog
        if recruit_stake.staked_amount > 0 {
            let accrued = calculate_accrued_yield(
                recruit_stake.staked_amount,
                ctx.accounts.realm.defense_rating,
                ctx.accounts.realm.base_reward_rate,
                recruit_stake.last_checkpoint_timestamp,
                clock.unix_timestamp,
            )?;
            recruit_stake.unclaimed_rewards = recruit_stake
                .unclaimed_rewards
                .checked_add(accrued)
                .ok_or(CastleError::ArithmeticOverflow)?;
        }

        // Transfer SPL tokens from user ATA to Realm Vault
        let cpi_accounts = Transfer {
            from: ctx.accounts.user_token_account.to_account_info(),
            to: ctx.accounts.realm_vault.to_account_info(),
            authority: ctx.accounts.user.to_account_info(),
        };
        let cpi_ctx = CpiContext::new(ctx.accounts.token_program.to_account_info(), cpi_accounts);
        token::transfer(cpi_ctx, amount)?;

        // Update stake state
        recruit_stake.owner = ctx.accounts.user.key();
        recruit_stake.realm = ctx.accounts.realm.key();
        recruit_stake.staked_amount = recruit_stake
            .staked_amount
            .checked_add(amount)
            .ok_or(CastleError::ArithmeticOverflow)?;
        recruit_stake.last_checkpoint_timestamp = clock.unix_timestamp;
        recruit_stake.bump = ctx.bumps.recruit_stake;

        let realm = &mut ctx.accounts.realm;
        realm.total_staked = realm
            .total_staked
            .checked_add(amount)
            .ok_or(CastleError::ArithmeticOverflow)?;

        msg!("CastleDAO: Successfully staked {} recruits", amount);
        Ok(())
    }

    /// Claim accumulated strategic quest rewards
    pub fn claim_quest_yield(ctx: Context<ClaimQuestYield>) -> Result<()> {
        let recruit_stake = &mut ctx.accounts.recruit_stake;
        let clock = Clock::get()?;

        let ongoing_yield = calculate_accrued_yield(
            recruit_stake.staked_amount,
            ctx.accounts.realm.defense_rating,
            ctx.accounts.realm.base_reward_rate,
            recruit_stake.last_checkpoint_timestamp,
            clock.unix_timestamp,
        )?;

        let total_payable = recruit_stake
            .unclaimed_rewards
            .checked_add(ongoing_yield)
            .ok_or(CastleError::ArithmeticOverflow)?;

        require!(total_payable > 0, CastleError::NoYieldToClaim);

        recruit_stake.unclaimed_rewards = 0;
        recruit_stake.last_checkpoint_timestamp = clock.unix_timestamp;

        // Sign transfer from Vault Authority PDA
        let realm_key = ctx.accounts.realm.key();
        let seeds = &[
            b"vault_authority".as_ref(),
            realm_key.as_ref(),
            &[ctx.accounts.realm.vault_authority_bump],
        ];
        let signer_seeds = &[&seeds[..]];

        let cpi_accounts = Transfer {
            from: ctx.accounts.realm_vault.to_account_info(),
            to: ctx.accounts.user_token_account.to_account_info(),
            authority: ctx.accounts.vault_authority.to_account_info(),
        };
        let cpi_ctx = CpiContext::new_with_signer(
            ctx.accounts.token_program.to_account_info(),
            cpi_accounts,
            signer_seeds,
        );
        token::transfer(cpi_ctx, total_payable)?;

        msg!("CastleDAO: Claimed {} yield tokens", total_payable);
        Ok(())
    }

    /// Submit content, tactical blueprints, or lore for decentralized curation
    pub fn submit_content(
        ctx: Context<SubmitContent>,
        content_hash: [u8; 32],
        requested_bounty: u64,
        metadata_uri: String,
    ) -> Result<()> {
        require!(metadata_uri.len() <= 128, CastleError::UriTooLong);
        require!(requested_bounty > 0, CastleError::InvalidBountyAmount);

        let realm = &mut ctx.accounts.realm;
        let bounty = &mut ctx.accounts.content_bounty;
        let clock = Clock::get()?;

        bounty.id = realm.bounty_count;
        bounty.realm = realm.key();
        bounty.creator = ctx.accounts.creator.key();
        bounty.content_hash = content_hash;
        bounty.requested_bounty = requested_bounty;
        bounty.metadata_uri = metadata_uri;
        bounty.approval_votes = 0;
        bounty.rejection_votes = 0;
        bounty.submission_timestamp = clock.unix_timestamp;
        bounty.state = ContentState::PendingReview;
        bounty.bump = ctx.bumps.content_bounty;

        realm.bounty_count = realm
            .bounty_count
            .checked_add(1)
            .ok_or(CastleError::ArithmeticOverflow)?;

        msg!("CastleDAO: Bounty #{} submitted for curation", bounty.id);
        Ok(())
    }

    /// Stakers vote to approve or reject submitted content
    pub fn curate_content(ctx: Context<CurateContent>, approve: bool) -> Result<()> {
        let stake = &ctx.accounts.recruit_stake;
        let bounty = &mut ctx.accounts.content_bounty;

        require!(stake.staked_amount > 0, CastleError::InsufficientVotingWeight);
        require!(
            bounty.state == ContentState::PendingReview,
            CastleError::BountyNotUnderReview
        );

        // Calculate quadratic voting weight: sqrt(staked_amount)
        let weight = integer_sqrt(stake.staked_amount);

        if approve {
            bounty.approval_votes = bounty
                .approval_votes
                .checked_add(weight)
                .ok_or(CastleError::ArithmeticOverflow)?;
        } else {
            bounty.rejection_votes = bounty
                .rejection_votes
                .checked_add(weight)
                .ok_or(CastleError::ArithmeticOverflow)?;
        }

        msg!(
            "CastleDAO: Curator voted. Weight: {}, Approved: {}",
            weight,
            approve
        );
        Ok(())
    }

    /// Settle content bounty post-timelock if quorum is achieved
    pub fn finalize_content_bounty(ctx: Context<FinalizeContentBounty>) -> Result<()> {
        let bounty = &mut ctx.accounts.content_bounty;
        let realm = &ctx.accounts.realm;
        let clock = Clock::get()?;

        require!(
            bounty.state == ContentState::PendingReview,
            CastleError::BountyNotUnderReview
        );

        let elapsed = clock
            .unix_timestamp
            .checked_sub(bounty.submission_timestamp)
            .ok_or(CastleError::ArithmeticUnderflow)?;

        require!(
            elapsed >= realm.timelock_duration,
            CastleError::TimelockNotElapsed
        );

        let net_positive_votes = bounty
            .approval_votes
            .checked_sub(bounty.rejection_votes)
            .unwrap_or(0);

        if net_positive_votes >= realm.curation_threshold {
            bounty.state = ContentState::Accepted;

            // Transfer reward from realm vault to content creator
            let realm_key = realm.key();
            let seeds = &[
                b"vault_authority".as_ref(),
                realm_key.as_ref(),
                &[realm.vault_authority_bump],
            ];
            let signer_seeds = &[&seeds[..]];

            let cpi_accounts = Transfer {
                from: ctx.accounts.realm_vault.to_account_info(),
                to: ctx.accounts.creator_token_account.to_account_info(),
                authority: ctx.accounts.vault_authority.to_account_info(),
            };
            let cpi_ctx = CpiContext::new_with_signer(
                ctx.accounts.token_program.to_account_info(),
                cpi_accounts,
                signer_seeds,
            );
            token::transfer(cpi_ctx, bounty.requested_bounty)?;
            msg!("CastleDAO: Content bounty #{} accepted and paid", bounty.id);
        } else {
            bounty.state = ContentState::Rejected;
            msg!("CastleDAO: Content bounty #{} rejected by quorum", bounty.id);
        }

        Ok(())
    }
}

// ----------------------------------------------------------------------------
// Helper Math Functions
// ----------------------------------------------------------------------------

fn calculate_accrued_yield(
    staked_amount: u64,
    defense_rating: u32,
    base_reward_rate: u64,
    last_timestamp: i64,
    current_timestamp: i64,
) -> Result<u64> {
    if current_timestamp <= last_timestamp || staked_amount == 0 {
        return Ok(0);
    }

    let elapsed = (current_timestamp - last_timestamp) as u128;
    let staked = staked_amount as u128;
    let rate = base_reward_rate as u128;

    // Multiplier = 1000 + defense_rating (1000 = 1.0x baseline, 10_000 max = 11.0x multiplier)
    let defense_multiplier = 1000u128
        .checked_add(defense_rating as u128)
        .ok_or(CastleError::ArithmeticOverflow)?;

    // Yield = (staked * rate * elapsed * defense_multiplier) / (SECONDS_PER_DAY * 1000 * PRECISION_FACTOR)
    let numerator = staked
        .checked_mul(rate)
        .and_then(|v| v.checked_mul(elapsed))
        .and_then(|v| v.checked_mul(defense_multiplier))
        .ok_or(CastleError::ArithmeticOverflow)?;

    let denominator = (SECONDS_PER_DAY as u128)
        .checked_mul(1_000)
        .and_then(|v| v.checked_mul(PRECISION_FACTOR))
        .ok_or(CastleError::ArithmeticOverflow)?;

    let yield_amount = numerator
        .checked_div(denominator)
        .ok_or(CastleError::ArithmeticUnderflow)?;

    Ok(yield_amount as u64)
}

fn integer_sqrt(n: u64) -> u64 {
    let mut x0 = n / 2;
    if x0 != 0 {
        let mut x1 = (x0 + n / x0) / 2;
        while x1 < x0 {
            x0 = x1;
            x1 = (x0 + n / x0) / 2;
        }
        x0
    } else {
        n
    }
}

// ----------------------------------------------------------------------------
// Context Accounts Validation
// ----------------------------------------------------------------------------

#[derive(Accounts)]
pub struct InitializeRealm<'info> {
    #[account(
        init,
        payer = authority,
        space = 8 + Realm::INIT_SPACE,
        seeds = [b"castle_realm", authority.key().as_ref()],
        bump
    )]
    pub realm: Account<'info, Realm>,

    pub token_mint: Account<'info, Mint>,

    #[account(
        init,
        payer = authority,
        token::mint = token_mint,
        token::authority = vault_authority,
    )]
    pub realm_vault: Account<'info, TokenAccount>,

    /// CHECK: PDA used as signing authority for realm_vault
    #[account(
        seeds = [b"vault_authority", realm.key().as_ref()],
        bump
    )]
    pub vault_authority: UncheckedAccount<'info>,

    #[account(mut)]
    pub authority: Signer<'info>,

    pub system_program: Program<'info, System>,
    pub token_program: Program<'info, Token>,
    pub rent: Sysvar<'info, Rent>,
}

#[derive(Accounts)]
pub struct StakeRecruit<'info> {
    #[account(mut)]
    pub realm: Account<'info, Realm>,

    #[account(
        init_if_needed,
        payer = user,
        space = 8 + RecruitStake::INIT_SPACE,
        seeds = [b"recruit_stake", realm.key().as_ref(), user.key().as_ref()],
        bump
    )]
    pub recruit_stake: Account<'info, RecruitStake>,

    #[account(
        mut,
        constraint = realm_vault.key() == realm.vault @ CastleError::InvalidVaultAccount
    )]
    pub realm_vault: Account<'info, TokenAccount>,

    #[account(
        mut,
        constraint = user_token_account.owner == user.key() @ CastleError::InvalidTokenOwner,
        constraint = user_token_account.mint == realm.mint @ CastleError::InvalidMint
    )]
    pub user_token_account: Account<'info, TokenAccount>,

    #[account(mut)]
    pub user: Signer<'info>,

    pub system_program: Program<'info, System>,
    pub token_program: Program<'info, Token>,
}

#[derive(Accounts)]
pub struct ClaimQuestYield<'info> {
    #[account(mut)]
    pub realm: Account<'info, Realm>,

    #[account(
        mut,
        seeds = [b"recruit_stake", realm.key().as_ref(), user.key().as_ref()],
        bump = recruit_stake.bump,
        has_one = owner @ CastleError::UnauthorizedStakeOwner,
        has_one = realm @ CastleError::InvalidRealmContext
    )]
    pub recruit_stake: Account<'info, RecruitStake>,

    #[account(
        mut,
        constraint = realm_vault.key() == realm.vault @ CastleError::InvalidVaultAccount
    )]
    pub realm_vault: Account<'info, TokenAccount>,

    /// CHECK: PDA vault authority
    #[account(
        seeds = [b"vault_authority", realm.key().as_ref()],
        bump = realm.vault_authority_bump
    )]
    pub vault_authority: UncheckedAccount<'info>,

    #[account(
        mut,
        constraint = user_token_account.owner == user.key() @ CastleError::InvalidTokenOwner,
        constraint = user_token_account.mint == realm.mint @ CastleError::InvalidMint
    )]
    pub user_token_account: Account<'info, TokenAccount>,

    pub user: Signer<'info>,
    pub token_program: Program<'info, Token>,
}

#[derive(Accounts)]
pub struct SubmitContent<'info> {
    #[account(mut)]
    pub realm: Account<'info, Realm>,

    #[account(
        init,
        payer = creator,
        space = 8 + ContentBounty::INIT_SPACE,
        seeds = [b"content_bounty", realm.key().as_ref(), realm.bounty_count.to_le_bytes().as_ref()],
        bump
    )]
    pub content_bounty: Account<'info, ContentBounty>,

    #[account(mut)]
    pub creator: Signer<'info>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct CurateContent<'info> {
    pub realm: Account<'info, Realm>,

    #[account(
        mut,
        seeds = [b"content_bounty", realm.key().as_ref(), content_bounty.id.to_le_bytes().as_ref()],
        bump = content_bounty.bump,
        has_one = realm @ CastleError::InvalidRealmContext
    )]
    pub content_bounty: Account<'info, ContentBounty>,

    #[account(
        seeds = [b"recruit_stake", realm.key().as_ref(), curator.key().as_ref()],
        bump = recruit_stake.bump,
        has_one = realm @ CastleError::InvalidRealmContext
    )]
    pub recruit_stake: Account<'info, RecruitStake>,

    pub curator: Signer<'info>,
}

#[derive(Accounts)]
pub struct FinalizeContentBounty<'info> {
    #[account(mut)]
    pub realm: Account<'info, Realm>,

    #[account(
        mut,
        seeds = [b"content_bounty", realm.key().as_ref(), content_bounty.id.to_le_bytes().as_ref()],
        bump = content_bounty.bump,
        has_one = realm @ CastleError::InvalidRealmContext
    )]
    pub content_bounty: Account<'info, ContentBounty>,

    #[account(
        mut,
        constraint = realm_vault.key() == realm.vault @ CastleError::InvalidVaultAccount
    )]
    pub realm_vault: Account<'info, TokenAccount>,

    /// CHECK: PDA vault authority
    #[account(
        seeds = [b"vault_authority", realm.key().as_ref()],
        bump = realm.vault_authority_bump
    )]
    pub vault_authority: UncheckedAccount<'info>,

    #[account(
        mut,
        constraint = creator_token_account.owner == content_bounty.creator @ CastleError::InvalidTokenOwner,
        constraint = creator_token_account.mint == realm.mint @ CastleError::InvalidMint
    )]
    pub creator_token_account: Account<'info, TokenAccount>,

    pub caller: Signer<'info>,
    pub token_program: Program<'info, Token>,
}

// ----------------------------------------------------------------------------
// State Accounts
// ----------------------------------------------------------------------------

#[account]
#[derive(InitSpace)]
pub struct Realm {
    pub authority: Pubkey,
    pub mint: Pubkey,
    pub vault: Pubkey,
    pub defense_rating: u32,
    pub base_reward_rate: u64,
    pub curation_threshold: u64,
    pub timelock_duration: i64,
    pub total_staked: u64,
    pub bounty_count: u64,
    pub vault_authority_bump: u8,
    pub bump: u8,
}

#[account]
#[derive(InitSpace)]
pub struct RecruitStake {
    pub owner: Pubkey,
    pub realm: Pubkey,
    pub staked_amount: u64,
    pub last_checkpoint_timestamp: i64,
    pub unclaimed_rewards: u64,
    pub bump: u8,
}

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, PartialEq, Eq, InitSpace)]
pub enum ContentState {
    PendingReview,
    Accepted,
    Rejected,
}

#[account]
#[derive(InitSpace)]
pub struct ContentBounty {
    pub id: u64,
    pub realm: Pubkey,
    pub creator: Pubkey,
    pub content_hash: [u8; 32],
    pub requested_bounty: u64,
    #[max_len(128)]
    pub metadata_uri: String,
    pub approval_votes: u64,
    pub rejection_votes: u64,
    pub submission_timestamp: i64,
    pub state: ContentState,
    pub bump: u8,
}

// ----------------------------------------------------------------------------
// Error Codes
// ----------------------------------------------------------------------------

#[error_code]
pub mod CastleError {
    #[msg("Defense rating exceeds the 10,000 basis points maximum limit.")]
    DefenseRatingExceeded,
    #[msg("Stake amount must be strictly greater than zero.")]
    ZeroStakeAmount,
    #[msg("Arithmetic overflow detected.")]
    ArithmeticOverflow,
    #[msg("Arithmetic underflow detected.")]
    ArithmeticUnderflow,
    #[msg("Zero yield available to claim.")]
    NoYieldToClaim,
    #[msg("Content metadata URI exceeds maximum permitted length.")]
    UriTooLong,
    #[msg("Requested bounty must be greater than zero.")]
    InvalidBountyAmount,
    #[msg("Curator has insufficient staked tokens to vote.")]
    InsufficientVotingWeight,
    #[msg("Content bounty is not under pending review.")]
    BountyNotUnderReview,
    #[msg("Challenge timelock duration has not elapsed yet.")]
    TimelockNotElapsed,
    #[msg("Vault token account does not match Realm configuration.")]
    InvalidVaultAccount,
    #[msg("Token account owner constraint violation.")]
    InvalidTokenOwner,
    #[msg("Mint account does not match realm mint.")]
    InvalidMint,
    #[msg("Unauthorized access to stake account.")]
    UnauthorizedStakeOwner,
    #[msg("Account does not belong to specified Castle Realm.")]
    InvalidRealmContext,
}