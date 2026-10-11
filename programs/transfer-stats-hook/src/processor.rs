use crate::{
    error::HookError,
    state::{self, EXTRA_METAS_SEED, STATS_LEN, STATS_SEED},
    token::{self, TOKEN_2022_ID},
};
use pinocchio::{
    cpi::{Seed, Signer},
    error::ProgramError,
    AccountView, Address, ProgramResult,
};
use pinocchio_system::create_account_with_minimum_balance_signed;

/// First 8 bytes of sha256("spl-transfer-hook-interface:execute").
pub const EXECUTE_DISCRIMINATOR: [u8; 8] = [105, 37, 101, 197, 75, 251, 102, 26];
/// First 8 bytes of sha256("spl-transfer-hook-interface:initialize-extra-account-metas").
pub const INITIALIZE_DISCRIMINATOR: [u8; 8] = [43, 34, 13, 49, 167, 88, 235, 235];

/// Account order of the Execute instruction: source, mint, destination, owner.
const EXECUTE_MINT_INDEX: u8 = 1;

const EXTRA_META_SIZE: usize = 35;
const EXTRA_METAS_COUNT: usize = 1;
const EXTRA_METAS_LEN: usize = 8 + 4 + 4 + EXTRA_META_SIZE * EXTRA_METAS_COUNT;

pub fn process_instruction(
    program_id: &Address,
    accounts: &mut [AccountView],
    data: &[u8],
) -> ProgramResult {
    if data.len() < 8 {
        return Err(ProgramError::InvalidInstructionData);
    }
    let (disc, args) = data.split_at(8);
    if disc == EXECUTE_DISCRIMINATOR {
        execute(program_id, accounts, args)
    } else if disc == INITIALIZE_DISCRIMINATOR {
        initialize_extra_account_metas(program_id, accounts)
    } else {
        Err(ProgramError::InvalidInstructionData)
    }
}

/// Accounts: `[source, mint, destination, owner, extra_metas, stats]`.
fn execute(program_id: &Address, accounts: &mut [AccountView], args: &[u8]) -> ProgramResult {
    let amount = u64::from_le_bytes(
        args.try_into()
            .map_err(|_| ProgramError::InvalidInstructionData)?,
    );
    let [source, mint, destination, _owner, extra_metas, stats, ..] = accounts else {
        return Err(ProgramError::NotEnoughAccountKeys);
    };

    if !source.owned_by(&TOKEN_2022_ID) || !destination.owned_by(&TOKEN_2022_ID) {
        return Err(HookError::InvalidTokenAccount.into());
    }
    {
        let source_data = source.try_borrow()?;
        if !token::is_transferring(&source_data)? {
            return Err(HookError::NotTransferring.into());
        }
        if token::account_mint(&source_data)? != mint.address().as_ref() {
            return Err(HookError::MintMismatch.into());
        }
    }
    {
        let destination_data = destination.try_borrow()?;
        if token::account_mint(&destination_data)? != mint.address().as_ref() {
            return Err(HookError::MintMismatch.into());
        }
    }

    if !extra_metas.owned_by(program_id) || extra_metas.is_data_empty() {
        return Err(HookError::MetasNotInitialized.into());
    }

    if !stats.owned_by(program_id) {
        return Err(HookError::StatsNotInitialized.into());
    }
    if !stats.is_writable() {
        return Err(HookError::InvalidStatsAccount.into());
    }
    let stats_address = stats.address().clone();
    let mut stats_data = stats.try_borrow_mut()?;
    let bump = state::stats_bump(&stats_data).ok_or(HookError::StatsNotInitialized)?;
    let expected = Address::create_program_address(
        &[STATS_SEED, mint.address().as_ref(), &[bump]],
        program_id,
    )
    .map_err(|_| HookError::InvalidStatsAccount)?;
    if expected != stats_address {
        return Err(HookError::InvalidStatsAccount.into());
    }

    state::record_transfer(&mut stats_data, amount);
    Ok(())
}

/// Accounts: `[extra_metas, mint, authority, system_program, stats]`.
/// The instruction data after the discriminator (a serialized list of extra
/// accounts) is ignored: the program always writes its own fixed list.
fn initialize_extra_account_metas(
    program_id: &Address,
    accounts: &mut [AccountView],
) -> ProgramResult {
    let [extra_metas, mint, authority, system_program, stats, ..] = accounts else {
        return Err(ProgramError::NotEnoughAccountKeys);
    };

    if system_program.address() != &pinocchio_system::ID {
        return Err(ProgramError::IncorrectProgramId);
    }
    if !authority.is_signer() {
        return Err(ProgramError::MissingRequiredSignature);
    }
    if !authority.is_writable() || !extra_metas.is_writable() || !stats.is_writable() {
        return Err(ProgramError::InvalidArgument);
    }

    if !mint.owned_by(&TOKEN_2022_ID) {
        return Err(HookError::InvalidMint.into());
    }
    {
        let mint_data = mint.try_borrow()?;
        match token::mint_authority(&mint_data)? {
            Some(key) if key == authority.address().as_ref() => {}
            _ => return Err(HookError::InvalidMint.into()),
        }
    }

    let (expected_metas, metas_bump) = state::pda_extra_metas(mint.address(), program_id);
    if &expected_metas != extra_metas.address() {
        return Err(HookError::InvalidPdaAddress.into());
    }
    let (expected_stats, stats_bump) = state::pda_stats(mint.address(), program_id);
    if &expected_stats != stats.address() {
        return Err(HookError::InvalidPdaAddress.into());
    }

    ensure_uninitialized(extra_metas)?;
    ensure_uninitialized(stats)?;

    let metas_bump_seed = [metas_bump];
    let metas_seeds = [
        Seed::from(EXTRA_METAS_SEED),
        Seed::from(mint.address().as_array()),
        Seed::from(&metas_bump_seed),
    ];
    create_account_with_minimum_balance_signed(
        extra_metas,
        EXTRA_METAS_LEN,
        program_id,
        authority,
        None,
        &[Signer::from(&metas_seeds)],
    )?;
    write_extra_metas(&mut extra_metas.try_borrow_mut()?)?;

    let stats_bump_seed = [stats_bump];
    let stats_seeds = [
        Seed::from(STATS_SEED),
        Seed::from(mint.address().as_array()),
        Seed::from(&stats_bump_seed),
    ];
    create_account_with_minimum_balance_signed(
        stats,
        STATS_LEN,
        program_id,
        authority,
        None,
        &[Signer::from(&stats_seeds)],
    )?;
    state::init_stats(&mut stats.try_borrow_mut()?, stats_bump)
}

fn ensure_uninitialized(account: &AccountView) -> ProgramResult {
    if !account.owned_by(&pinocchio_system::ID) || !account.is_data_empty() {
        return Err(HookError::AlreadyInitialized.into());
    }
    Ok(())
}

/// Writes the TLV-encoded validation data for the Execute instruction:
/// `[execute discriminator][u32 value length][u32 count][ExtraAccountMeta...]`
/// with one PDA of this program: seeds `["stats", <account 1 = mint>]`,
/// writable, not a signer.
fn write_extra_metas(data: &mut [u8]) -> ProgramResult {
    if data.len() != EXTRA_METAS_LEN {
        return Err(ProgramError::InvalidAccountData);
    }
    let value_len = (4 + EXTRA_META_SIZE * EXTRA_METAS_COUNT) as u32;
    data[0..8].copy_from_slice(&EXECUTE_DISCRIMINATOR);
    data[8..12].copy_from_slice(&value_len.to_le_bytes());
    data[12..16].copy_from_slice(&(EXTRA_METAS_COUNT as u32).to_le_bytes());

    let meta = &mut data[16..16 + EXTRA_META_SIZE];
    meta.fill(0);
    // Discriminator 1: PDA derived from the hook program itself.
    meta[0] = 1;
    // Packed seeds: literal (type 1, len, bytes), account key (type 3, index).
    let config = &mut meta[1..33];
    let mut n = 0;
    config[n] = 1;
    config[n + 1] = STATS_SEED.len() as u8;
    config[n + 2..n + 2 + STATS_SEED.len()].copy_from_slice(STATS_SEED);
    n += 2 + STATS_SEED.len();
    config[n] = 3;
    config[n + 1] = EXECUTE_MINT_INDEX;
    // is_signer = 0, is_writable = 1
    meta[34] = 1;
    Ok(())
}
