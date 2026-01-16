use solana_program_error::ProgramError;
use thiserror::Error;

#[derive(Error, Debug, Copy, Clone)]
pub enum ClmmError {
    #[error("Pool not initialized")]
    PoolNotInitialized,

    #[error("Invalid tick range")]
    InvalidTickRange,

    #[error("Math overflow")]
    MathOverflow,

    #[error("Position not found")]
    PositionNotFound

    #[error("Insufficient liquidity")]
    InsufficientLiquidity
}

impl From<ClmmError> for ProgramError {
    fn from(e: ClmmError) -> Self {
        ProgramError::Custom(e as u32)
    }
}
