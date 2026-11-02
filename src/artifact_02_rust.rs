use anchor_lang::prelude::*;
use anchor_lang::system_program::{transfer, Transfer};

declare_id!("CstL7oRkUv9D57K1kYvRz8b1z3T9P2s4W8x5L6Q7r8N");

pub const PRECISION_SCALE: u128 = 1_000_000_000; // Fixed-point 1e9 precision
pub const MAX_SUBMISSION_SLOTS: u64 = 432_000;    // ~3 days on Solana (400ms slots)
pub const MIN_SUBMISSION_BOND: u64 = 10_000_000;  // 0.01 SOL

#[program]
pub mod castledao_content {
    use super::*;

    /// Initializes a new challenge pool funded with an initial bounty escrow.
    pub fn initialize_challenge(
        ctx: Context<InitializeChallenge>,
        challenge_id: u64,
        submission_window_slots: u64,
        voting_window_slots: u64,
        min_curator_stake: u64,
        submission_bond: u64,
        bounty_amount: u64,
    ) -> Result<()> {
        require!(submission_bond >= MIN_SUBMISSION_BOND, ContentError::BondTooLow);
        require!(submission_window_slots > 0 && submission_window_slots <= MAX_SUBMISSION_SLOTS, ContentError::InvalidWindow);
        require!(voting_window_slots > 0, ContentError::InvalidWindow);

        let clock = Clock::get()?;
        let current_slot = clock.slot;

        let challenge = &mut ctx.accounts.challenge;
        challenge.authority = ctx.accounts.authority.key();
        challenge.challenge_id = challenge_id;
        challenge.submission_start_slot = current_slot;
        challenge.submission_end_slot = current_slot.checked_add(submission_window_slots).ok_or(ContentError::MathOverflow)?;
        challenge.voting_end_slot = challenge.submission_end_slot.checked_add(voting_window_slots).ok_or(ContentError::MathOverflow)?;
        challenge.min_curator_stake = min_curator_stake;
        challenge.submission_bond = submission_bond;
        challenge.total_bounty_pool = bounty_amount;
        challenge.total_submissions = 0;
        challenge.total_valid_submissions = 0;
        challenge.state = ChallengeState::AcceptingSubmissions;
        challenge.bump = ctx.bumps.challenge;
        challenge.vault_bump = ctx.bumps.vault;

        // Escrow initial bounty
        if bounty_amount > 0 {
            transfer(
                CpiContext::new(
                    ctx.accounts.system_program.to_account_info(),
                    Transfer {
                        from: ctx.accounts.authority.to_account_info(),
                        to: ctx.accounts.vault.to_account_info(),
                    },
                ),
                bounty_amount,
            )?;
        }

        emit!(ChallengeInitializedEvent {
            challenge_id,
            authority: challenge.authority,
            bounty_amount,
            submission_end_slot: challenge.submission_end_slot,
            voting_end_slot: challenge.voting_end_slot,
        });

        Ok(())
    }

    /// Submits a content piece against the challenge; locks anti-spam bond into vault PDA.
    pub fn submit_entry(
        ctx: Context<SubmitEntry>,
        content_hash: [u8; 32],
        metadata_uri: String,
    ) -> Result<()> {
        let clock = Clock::get()?;
        let challenge = &mut ctx.accounts.challenge;

        require!(clock.slot >= challenge.submission_start_slot, ContentError::WindowNotStarted);
        require!(clock.slot <= challenge.submission_end_slot, ContentError::SubmissionWindowClosed);
        require!(metadata_uri.len() <= 128, ContentError::UriTooLong);

        let bond_amount = challenge.submission_bond;

        // Lock anti-spam bond
        transfer(
            CpiContext::new(
                ctx.accounts.system_program.to_account_info(),
                Transfer {
                    from: ctx.accounts.creator.to_account_info(),
                    to: ctx.accounts.vault.to_account_info(),
                },
            ),
            bond_amount,
        )?;

        let submission = &mut ctx.accounts.submission;
        submission.challenge = challenge.key();
        submission.creator = ctx.accounts.creator.key();
        submission.content_hash = content_hash;
        submission.metadata_uri = metadata_uri;
        submission.aggregate_score = 0;
        submission.total_weight = 0;
        submission.slashing_votes = 0;
        submission.settled = false;
        submission.bump = ctx.bumps.submission;

        challenge.total_submissions = challenge.total_submissions.checked_add(1).ok_or(ContentError::MathOverflow)?;

        emit!(SubmissionRegisteredEvent {
            challenge_id: challenge.challenge_id,
            creator: submission.creator,
            content_hash,
        });

        Ok(())
    }

    /// Curators review entries, casting a score (0 - 100) or recommending slashing for spam.
    pub fn vote_entry(
        ctx: Context<VoteEntry>,
        score: u8,
        slash: bool,
    ) -> Result<()> {
        let clock = Clock::get()?;
        let challenge = &ctx.accounts.challenge;

        require!(clock.slot > challenge.submission_end_slot, ContentError::VotingNotOpen);
        require!(clock.slot <= challenge.voting_end_slot, ContentError::VotingClosed);
        require!(score <= 100, ContentError::InvalidScore);

        let curator_stake = ctx.accounts.curator.lamports();
        require!(curator_stake >= challenge.min_curator_stake, ContentError::InsufficientCuratorStake);

        let submission = &mut ctx.accounts.submission;
        let vote_record = &mut ctx.accounts.vote_record;

        vote_record.curator = ctx.accounts.curator.key();
        vote_record.submission = submission.key();
        vote_record.weight = curator_stake;
        vote_record.score = score;
        vote_record.slashed = slash;
        vote_record.bump = ctx.bumps.vote_record;

        if slash {
            submission.slashing_votes = submission.slashing_votes.checked_add(1).ok_or(ContentError::MathOverflow)?;
        } else {
            let weighted_points = (score as u128).checked_mul(curator_stake as u128).ok_or(ContentError::MathOverflow)?;
            submission.aggregate_score = submission.aggregate_score.checked_add(weighted_points).ok_or(ContentError::MathOverflow)?;
            submission.total_weight = submission.total_weight.checked_add(curator_stake).ok_or(ContentError::MathOverflow)?;
        }

        emit!(VoteCastEvent {
            submission: submission.key(),
            curator: ctx.accounts.curator.key(),
            score,
            slashed: slash,
            stake_weight: curator_stake,
        });

        Ok(())
    }

    /// Finalizes a submission: if slashed, bond is forfeited to bounty pool; if valid, bond returned + bounty split allocated.
    pub fn settle_submission(ctx: Context<SettleSubmission>) -> Result<()> {
        let clock = Clock::get()?;
        let challenge = &mut ctx.accounts.challenge;
        let submission = &mut ctx.accounts.submission;

        require!(clock.slot > challenge.voting_end_slot, ContentError::ChallengeActive);
        require!(!submission.settled, ContentError::AlreadySettled);

        submission.settled = true;

        let challenge_key = challenge.key();
        let vault_seeds: &[&[&[u8]]] = &[&[
            b"vault",
            challenge_key.as_ref(),
            &[challenge.vault_bump],
        ]];

        let is_slashed = submission.slashing_votes >= 3 && submission.slashing_votes > submission.total_weight.checked_div(challenge.min_curator_stake).unwrap_or(0);

        if is_slashed {
            // Forfeit bond: remains in the vault PDA, credited to the bounty pool
            challenge.total_bounty_pool = challenge.total_bounty_pool.checked_add(challenge.submission_bond).ok_or(ContentError::MathOverflow)?;
            emit!(SubmissionSlashedEvent {
                challenge_id: challenge.challenge_id,
                submission: submission.key(),
                forfeited_bond: challenge.submission_bond,
            });
        } else {
            // Refund bond to creator
            **ctx.accounts.vault.to_account_info().try_borrow_mut_lamports()? -= challenge.submission_bond;
            **ctx.accounts.creator.to_account_info().try_borrow_mut_lamports()? += challenge.submission_bond;

            challenge.total_valid_submissions = challenge.total_valid_submissions.checked_add(1).ok_or(ContentError::MathOverflow)?;

            // Calculate pro-rata reward payout based on aggregate weighted score
            if submission.total_weight > 0 && challenge.total_bounty_pool > 0 {
                let normalized_score = submission.aggregate_score.checked_div(submission.total_weight as u128).unwrap_or(0);
                
                // Reward scaling: (NormalizedScore / 100) * (Pool / MaxSubmissionsAllocation)
                let base_payout = (challenge.total_bounty_pool as u128)
                    .checked_mul(normalized_score)
                    .ok_or(ContentError::MathOverflow)?
                    .checked_div(100 * (challenge.total_submissions.max(1) as u128))
                    .ok_or(ContentError::MathOverflow)? as u64;

                let capped_payout = base_payout.min(ctx.accounts.vault.lamports());

                if capped_payout > 0 {
                    **ctx.accounts.vault.to_account_info().try_borrow_mut_lamports()? -= capped_payout;
                    **ctx.accounts.creator.to_account_info().try_borrow_mut_lamports()? += capped_payout;
                }

                emit!(RewardDistributedEvent {
                    challenge_id: challenge.challenge_id,
                    creator: submission.creator,
                    bounty_paid: capped_payout,
                    bond_refunded: challenge.submission_bond,
                });
            }
        }

        // Keeper incentive: micro-tip for executing crank
        let keeper_tip: u64 = 1_000_000; // 0.001 SOL
        if ctx.accounts.vault.lamports() >= keeper_tip {
            **ctx.accounts.vault.to_account_info().try_borrow_mut_lamports()? -= keeper_tip;
            **ctx.accounts.keeper.to_account_info().try_borrow_mut_lamports()? += keeper_tip;
        }

        Ok(())
    }
}

// -----------------------------------------------------------------------------
// ACCOUNT VALIDATION STRUCTS
// -----------------------------------------------------------------------------

#[derive(Accounts)]
#[instruction(challenge_id: u64)]
pub struct InitializeChallenge<'info> {
    #[account(
        init,
        payer = authority,
        space = 8 + Challenge::LEN,
        seeds = [b"challenge", challenge_id.to_le_bytes().as_ref()],
        bump
    )]
    pub challenge: Account<'info, Challenge>,

    #[account(
        mut,
        seeds = [b"vault", challenge.key().as_ref()],
        bump
    )]
    pub vault: SystemAccount<'info>,

    #[account(mut)]
    pub authority: Signer<'info>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct SubmitEntry<'info> {
    #[account(
        mut,
        seeds = [b"challenge", challenge.challenge_id.to_le_bytes().as_ref()],
        bump = challenge.bump
    )]
    pub challenge: Account<'info, Challenge>,

    #[account(
        init,
        payer = creator,
        space = 8 + Submission::LEN,
        seeds = [b"submission", challenge.key().as_ref(), creator.key().as_ref()],
        bump
    )]
    pub submission: Account<'info, Submission>,

    #[account(
        mut,
        seeds = [b"vault", challenge.key().as_ref()],
        bump = challenge.vault_bump
    )]
    pub vault: SystemAccount<'info>,

    #[account(mut)]
    pub creator: Signer<'info>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct VoteEntry<'info> {
    #[account(
        seeds = [b"challenge", challenge.challenge_id.to_le_bytes().as_ref()],
        bump = challenge.bump
    )]
    pub challenge: Account<'info, Challenge>,

    #[account(
        mut,
        seeds = [b"submission", challenge.key().as_ref(), submission.creator.as_ref()],
        bump = submission.bump
    )]
    pub submission: Account<'info, Submission>,

    #[account(
        init,
        payer = curator,
        space = 8 + VoteRecord::LEN,
        seeds = [b"vote", submission.key().as_ref(), curator.key().as_ref()],
        bump
    )]
    pub vote_record: Account<'info, VoteRecord>,

    #[account(mut)]
    pub curator: Signer<'info>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct SettleSubmission<'info> {
    #[account(
        mut,
        seeds = [b"challenge", challenge.challenge_id.to_le_bytes().as_ref()],
        bump = challenge.bump
    )]
    pub challenge: Account<'info, Challenge>,

    #[account(
        mut,
        seeds = [b"submission", challenge.key().as_ref(), submission.creator.as_ref()],
        bump = submission.bump
    )]
    pub submission: Account<'info, Submission>,

    #[account(
        mut,
        seeds = [b"vault", challenge.key().as_ref()],
        bump = challenge.vault_bump
    )]
    pub vault: SystemAccount<'info>,

    /// CHECK: Recipient of refunded bond and earned reward
    #[account(mut, address = submission.creator)]
    pub creator: AccountInfo<'info>,

    #[account(mut)]
    pub keeper: Signer<'info>,

    pub system_program: Program<'info, System>,
}

// -----------------------------------------------------------------------------
// STATE ENUMS AND ACCOUNTS
// -----------------------------------------------------------------------------

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, PartialEq, Eq)]
pub enum ChallengeState {
    AcceptingSubmissions,
    Voting,
    Settled,
}

#[account]
pub struct Challenge {
    pub authority: Pubkey,          // 32
    pub challenge_id: u64,          // 8
    pub submission_start_slot: u64, // 8
    pub submission_end_slot: u64,   // 8
    pub voting_end_slot: u64,       // 8
    pub min_curator_stake: u64,     // 8
    pub submission_bond: u64,       // 8
    pub total_bounty_pool: u64,     // 8
    pub total_submissions: u32,     // 4
    pub total_valid_submissions: u32,// 4
    pub state: ChallengeState,      // 1
    pub bump: u8,                   // 1
    pub vault_bump: u8,             // 1
}

impl Challenge {
    pub const LEN: usize = 32 + 8 + 8 + 8 + 8 + 8 + 8 + 8 + 4 + 4 + 1 + 1 + 1;
}

#[account]
pub struct Submission {
    pub challenge: Pubkey,       // 32
    pub creator: Pubkey,         // 32
    pub content_hash: [u8; 32],  // 32
    pub metadata_uri: String,    // 4 + 128
    pub aggregate_score: u128,   // 16 (weighted cumulative score)
    pub total_weight: u64,       // 8
    pub slashing_votes: u32,     // 4
    pub settled: bool,           // 1
    pub bump: u8,                // 1
}

impl Submission {
    pub const LEN: usize = 32 + 32 + 32 + (4 + 128) + 16 + 8 + 4 + 1 + 1;
}

#[account]
pub struct VoteRecord {
    pub curator: Pubkey,    // 32
    pub submission: Pubkey, // 32
    pub weight: u64,        // 8
    pub score: u8,          // 1
    pub slashed: bool,      // 1
    pub bump: u8,           // 1
}

impl VoteRecord {
    pub const LEN: usize = 32 + 32 + 8 + 1 + 1 + 1;
}

// -----------------------------------------------------------------------------
// EVENTS & ERRORS
// -----------------------------------------------------------------------------

#[event]
pub struct ChallengeInitializedEvent {
    pub challenge_id: u64,
    pub authority: Pubkey,
    pub bounty_amount: u64,
    pub submission_end_slot: u64,
    pub voting_end_slot: u64,
}

#[event]
pub struct SubmissionRegisteredEvent {
    pub challenge_id: u64,
    pub creator: Pubkey,
    pub content_hash: [u8; 32],
}

#[event]
pub struct VoteCastEvent {
    pub submission: Pubkey,
    pub curator: Pubkey,
    pub score: u8,
    pub slashed: bool,
    pub stake_weight: u64,
}

#[event]
pub struct RewardDistributedEvent {
    pub challenge_id: u64,
    pub creator: Pubkey,
    pub bounty_paid: u64,
    pub bond_refunded: u64,
}

#[event]
pub struct SubmissionSlashedEvent {
    pub challenge_id: u64,
    pub submission: Pubkey,
    pub forfeited_bond: u64,
}

#[error_code]
pub enum ContentError {
    #[msg("Bond amount is below protocol minimum.")]
    BondTooLow,
    #[msg("Window slot duration is invalid.")]
    InvalidWindow,
    #[msg("Submission window has not started.")]
    WindowNotStarted,
    #[msg("Submission window has closed.")]
    SubmissionWindowClosed,
    #[msg("Metadata URI exceeds max byte allocation.")]
    UriTooLong,
    #[msg("Voting window is not open.")]
    VotingNotOpen,
    #[msg("Voting window has concluded.")]
    VotingClosed,
    #[msg("Score must be between 0 and 100.")]
    InvalidScore,
    #[msg("Curator lamport balance below minimum stake threshold.")]
    InsufficientCuratorStake,
    #[msg("Challenge is still active; cannot settle before voting concludes.")]
    ChallengeActive,
    #[msg("This entry has already been settled.")]
    AlreadySettled,
    #[msg("Arithmetic overflow or underflow detected.")]
    MathOverflow,
}