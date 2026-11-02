use anchor_lang::prelude::*;

#[error_code]
pub enum CastleDaoError {
    #[msg("Castle is currently locked under active siege.")]
    CastleUnderSiege,
    #[msg("Signer is not authorized as the rightful Lord of this castle.")]
    UnauthorizedLord,
    #[msg("Siege battle window has not yet concluded.")]
    SiegeStillActive,
    #[msg("Castle is not currently engaged in any siege.")]
    CastleNotInSiege,
    #[msg("Numerical overflow detected during math operation.")]
    MathOverflow,
    #[msg("Production duration yields zero claimable resources.")]
    ZeroClaimableResources,
    #[msg("Insufficient attacking war collateral provided.")]
    InsufficientWarChest,
}