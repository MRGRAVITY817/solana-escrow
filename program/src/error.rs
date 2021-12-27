use solana_program::program_error::ProgramError;
use thiserror::Error;

#[derive(Error, Debug, Copy, Clone)]
pub enum EscrowError {
    #[error("Invalid Instruction")]
    InvalidInstruction,
    #[error("Not Rent Exempt")]
    NotRentExempt,
    #[error("Amount Overflow")]
    AmountOverflow,
    #[error("Expected Amount Mismatch")]
    ExpectedAmountMismatch,
}

// By implementing From trait, we can convert EscrowError to ProgramError when using `?`
impl From<EscrowError> for ProgramError {
    fn from(e: EscrowError) -> Self {
        ProgramError::Custom(e as u32)
    }
}
