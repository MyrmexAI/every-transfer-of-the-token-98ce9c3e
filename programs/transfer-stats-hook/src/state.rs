use pinocchio::{error::ProgramError, Address};

pub const STATS_SEED: &[u8] = b"stats";
pub const EXTRA_METAS_SEED: &[u8] = b"extra-account-metas";

const MAGIC: [u8; 8] = *b"TXSTATS1";

/// Layout (little endian):
/// `[0..8]` magic, `[8]` bump, `[9..16]` zero, `[16..24]` transfer count (u64),
/// `[24..40]` total amount moved (u128), `[40..48]` largest transfer (u64).
pub const STATS_LEN: usize = 48;

const BUMP_OFFSET: usize = 8;
const COUNT_OFFSET: usize = 16;
const TOTAL_OFFSET: usize = 24;
const LARGEST_OFFSET: usize = 40;

pub fn init_stats(data: &mut [u8], bump: u8) -> Result<(), ProgramError> {
    if data.len() != STATS_LEN {
        return Err(ProgramError::InvalidAccountData);
    }
    data.fill(0);
    data[..8].copy_from_slice(&MAGIC);
    data[BUMP_OFFSET] = bump;
    Ok(())
}

/// Returns the stored bump if `data` is an initialized stats account.
pub fn stats_bump(data: &[u8]) -> Option<u8> {
    if data.len() != STATS_LEN || data[..8] != MAGIC {
        return None;
    }
    Some(data[BUMP_OFFSET])
}

/// Never fails on overflow: a transfer must not be blocked by bookkeeping.
pub fn record_transfer(data: &mut [u8], amount: u64) {
    let count = read_u64(data, COUNT_OFFSET).saturating_add(1);
    let total = read_u128(data, TOTAL_OFFSET).saturating_add(amount as u128);
    let largest = read_u64(data, LARGEST_OFFSET).max(amount);

    data[COUNT_OFFSET..COUNT_OFFSET + 8].copy_from_slice(&count.to_le_bytes());
    data[TOTAL_OFFSET..TOTAL_OFFSET + 16].copy_from_slice(&total.to_le_bytes());
    data[LARGEST_OFFSET..LARGEST_OFFSET + 8].copy_from_slice(&largest.to_le_bytes());
}

fn read_u64(data: &[u8], offset: usize) -> u64 {
    let mut buf = [0u8; 8];
    buf.copy_from_slice(&data[offset..offset + 8]);
    u64::from_le_bytes(buf)
}

fn read_u128(data: &[u8], offset: usize) -> u128 {
    let mut buf = [0u8; 16];
    buf.copy_from_slice(&data[offset..offset + 16]);
    u128::from_le_bytes(buf)
}

pub fn pda_stats(mint: &Address, program_id: &Address) -> (Address, u8) {
    Address::find_program_address(&[STATS_SEED, mint.as_ref()], program_id)
}

pub fn pda_extra_metas(mint: &Address, program_id: &Address) -> (Address, u8) {
    Address::find_program_address(&[EXTRA_METAS_SEED, mint.as_ref()], program_id)
}
