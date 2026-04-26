use solana_program::program_error::ProgramError;
use thiserror::Error;

#[derive(Error, Debug, Clone, Copy, PartialEq, Eq)]
pub enum LiquidityError {
    #[error("Invalid instruction data")]
    InvalidInstruction,

    #[error("Math overflow")]
    MathOverflow,

    #[error("Math underflow")]
    MathUnderflow,
}

impl From<LiquidityError> for ProgramError {
    fn from(e: LiquidityError) -> Self {
        ProgramError::Custom(e as u32)
    }
}
