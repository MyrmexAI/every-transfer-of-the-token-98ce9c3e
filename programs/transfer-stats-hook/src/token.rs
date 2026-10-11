use crate::error::HookError;
use pinocchio::{error::ProgramError, Address};

pub const TOKEN_2022_ID: Address =
    Address::from_str_const("TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb");

const BASE_ACCOUNT_LEN: usize = 165;
const ACCOUNT_TYPE_OFFSET: usize = BASE_ACCOUNT_LEN;
const ACCOUNT_TYPE_ACCOUNT: u8 = 2;
const TLV_START: usize = ACCOUNT_TYPE_OFFSET + 1;
const EXT_UNINITIALIZED: u16 = 0;
const EXT_TRANSFER_HOOK_ACCOUNT: u16 = 15;

const MINT_LEN: usize = 82;
const MINT_AUTHORITY_TAG_OFFSET: usize = 0;
const MINT_AUTHORITY_OFFSET: usize = 4;
const MINT_IS_INITIALIZED_OFFSET: usize = 45;

/// Mint field (first 32 bytes) of a base Token-2022 account.
pub fn account_mint(data: &[u8]) -> Result<&[u8], ProgramError> {
    if data.len() < BASE_ACCOUNT_LEN {
        return Err(HookError::InvalidTokenAccount.into());
    }
    Ok(&data[..32])
}

/// Value of the `transferring` flag of the TransferHookAccount extension.
/// An account without that extension is not part of a hooked transfer.
pub fn is_transferring(data: &[u8]) -> Result<bool, ProgramError> {
    if data.len() <= TLV_START || data[ACCOUNT_TYPE_OFFSET] != ACCOUNT_TYPE_ACCOUNT {
        return Err(HookError::InvalidTokenAccount.into());
    }

    let mut offset = TLV_START;
    while offset + 4 <= data.len() {
        let ext_type = u16::from_le_bytes([data[offset], data[offset + 1]]);
        let ext_len = u16::from_le_bytes([data[offset + 2], data[offset + 3]]) as usize;
        let start = offset + 4;
        let end = start
            .checked_add(ext_len)
            .filter(|end| *end <= data.len())
            .ok_or(HookError::InvalidTokenAccount)?;

        if ext_type == EXT_UNINITIALIZED {
            break;
        }
        if ext_type == EXT_TRANSFER_HOOK_ACCOUNT {
            if ext_len < 1 {
                return Err(HookError::InvalidTokenAccount.into());
            }
            return Ok(data[start] != 0);
        }
        offset = end;
    }
    Err(HookError::InvalidTokenAccount.into())
}

/// Mint authority of an initialized mint, if any.
pub fn mint_authority(data: &[u8]) -> Result<Option<&[u8]>, ProgramError> {
    if data.len() < MINT_LEN || data[MINT_IS_INITIALIZED_OFFSET] != 1 {
        return Err(HookError::InvalidMint.into());
    }
    let tag = u32::from_le_bytes([
        data[MINT_AUTHORITY_TAG_OFFSET],
        data[MINT_AUTHORITY_TAG_OFFSET + 1],
        data[MINT_AUTHORITY_TAG_OFFSET + 2],
        data[MINT_AUTHORITY_TAG_OFFSET + 3],
    ]);
    match tag {
        0 => Ok(None),
        1 => Ok(Some(
            &data[MINT_AUTHORITY_OFFSET..MINT_AUTHORITY_OFFSET + 32],
        )),
        _ => Err(HookError::InvalidMint.into()),
    }
}
