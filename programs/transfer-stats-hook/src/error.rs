use pinocchio::error::ProgramError;

#[repr(u32)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HookError {
    /// The source token account is not currently inside a Token-2022 transfer.
    NotTransferring = 0,
    /// A token account is not a valid Token-2022 account of the given mint.
    InvalidTokenAccount = 1,
    /// The token accounts do not belong to the mint passed to the hook.
    MintMismatch = 2,
    /// The stats account does not exist or was never initialized.
    StatsNotInitialized = 3,
    /// The stats account is not the expected PDA, or is not writable.
    InvalidStatsAccount = 4,
    /// The extra account metas account does not exist or is not owned by this program.
    MetasNotInitialized = 5,
    /// The mint is not a Token-2022 mint or the authority does not match.
    InvalidMint = 6,
    /// A state account already exists.
    AlreadyInitialized = 7,
    /// A passed PDA does not match its derivation.
    InvalidPdaAddress = 8,
}

impl From<HookError> for ProgramError {
    fn from(e: HookError) -> Self {
        ProgramError::Custom(e as u32)
    }
}
