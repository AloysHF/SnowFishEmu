//! Save-state serialization.
//!
//! The payload is a bincode snapshot of the machine's runtime state wrapped
//! in a versioned envelope: an 8-byte signature, a little-endian format
//! version, and a content fingerprint so states from other packages are
//! rejected before decoding. Diagnostic counters and the wrapper table (which
//! is rebuilt identically from the image at load time) are not serialized.

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::cpu::ArmCpu;
use crate::gdi::Gdi;
use crate::machine::FileSlot;
use crate::mem::GuestMem;

/// Envelope signature (`"SFEMSTAT"`).
pub const SAVE_STATE_MAGIC: &[u8; 8] = b"SFEMSTAT";
/// Envelope format version; bump when the state layout changes.
pub const SAVE_STATE_FORMAT: u32 = 1;
/// Envelope: signature + format version + content fingerprint.
pub const SAVE_STATE_HEADER_LEN: usize = SAVE_STATE_MAGIC.len() + 4 + 8;

/// Save-state encode/decode errors.
#[derive(Debug, Error)]
pub enum SaveStateError {
    #[error("not a save state (bad signature)")]
    BadSignature,
    #[error("unsupported save-state version {0}")]
    UnsupportedVersion(u32),
    #[error("save state belongs to different content")]
    ContentMismatch,
    #[error("save state is truncated")]
    Truncated,
    #[error("save-state payload error: {0}")]
    Payload(String),
}

/// The serializable runtime state of a [`crate::machine::SnowFishMachine`].
///
/// Everything the guest can observe across frames is captured; static load
/// artifacts (container, resource file, wrapper table) are restored by the
/// owning machine instead.
#[derive(Serialize, Deserialize)]
pub(crate) struct MachineState {
    pub(crate) mem: GuestMem,
    pub(crate) cpu: ArmCpu,
    pub(crate) handler: u32,
    pub(crate) file_slots: Vec<FileSlot>,
    pub(crate) events: crate::machine::EventQueue,
    pub(crate) frame_count: u64,
    pub(crate) lz_calls: u64,
    pub(crate) last_pc: u32,
    pub(crate) branch_log: Vec<(u32, u32)>,
    pub(crate) gdi: Gdi,
}

/// FNV-1a 64-bit content fingerprint over the container and resource bytes.
pub(crate) fn content_fingerprint(parts: [&[u8]; 2]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for part in parts {
        for &byte in part {
            hash ^= byte as u64;
            hash = hash.wrapping_mul(0x0000_0100_0000_01B3);
        }
    }
    hash
}

/// Encodes a state snapshot into the versioned envelope.
pub(crate) fn encode(state: &MachineState, fingerprint: u64) -> Result<Vec<u8>, SaveStateError> {
    let payload = bincode::serialize(state).map_err(|e| SaveStateError::Payload(e.to_string()))?;
    let mut blob = Vec::with_capacity(SAVE_STATE_HEADER_LEN + payload.len());
    blob.extend_from_slice(SAVE_STATE_MAGIC);
    blob.extend_from_slice(&SAVE_STATE_FORMAT.to_le_bytes());
    blob.extend_from_slice(&fingerprint.to_le_bytes());
    blob.extend_from_slice(&payload);
    Ok(blob)
}

/// Decodes an envelope, rejecting foreign signatures, versions, and content.
pub(crate) fn decode(
    data: &[u8],
    expected_fingerprint: u64,
) -> Result<MachineState, SaveStateError> {
    if data.len() < SAVE_STATE_HEADER_LEN || &data[..SAVE_STATE_MAGIC.len()] != SAVE_STATE_MAGIC {
        return Err(SaveStateError::BadSignature);
    }
    let version = u32::from_le_bytes(
        data[SAVE_STATE_MAGIC.len()..SAVE_STATE_MAGIC.len() + 4]
            .try_into()
            .unwrap(),
    );
    if version != SAVE_STATE_FORMAT {
        return Err(SaveStateError::UnsupportedVersion(version));
    }
    let fingerprint = u64::from_le_bytes(
        data[SAVE_STATE_MAGIC.len() + 4..SAVE_STATE_HEADER_LEN]
            .try_into()
            .unwrap(),
    );
    if fingerprint != expected_fingerprint {
        return Err(SaveStateError::ContentMismatch);
    }
    bincode::deserialize(&data[SAVE_STATE_HEADER_LEN..]).map_err(|e| match e.as_ref() {
        bincode::ErrorKind::Io(io) if io.kind() == std::io::ErrorKind::UnexpectedEof => {
            SaveStateError::Truncated
        }
        _ => SaveStateError::Payload(e.to_string()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fingerprint_is_stable_and_input_sensitive() {
        let a = content_fingerprint([b"hello", b""]);
        let b = content_fingerprint([b"hello", b""]);
        let c = content_fingerprint([b"hellp", b""]);
        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn decode_rejects_bad_signature_version_and_content() {
        let state = MachineState {
            mem: GuestMem::new(0x1000),
            cpu: ArmCpu::new(),
            handler: 0,
            file_slots: Vec::new(),
            events: Default::default(),
            frame_count: 0,
            lz_calls: 0,
            last_pc: 0,
            branch_log: Vec::new(),
            gdi: Gdi::new(),
        };
        let blob = encode(&state, 42).unwrap();

        let mut wrong_version = blob.clone();
        wrong_version[8..12].copy_from_slice(&999u32.to_le_bytes());
        assert!(matches!(
            decode(&wrong_version, 42),
            Err(SaveStateError::UnsupportedVersion(999))
        ));

        assert!(matches!(
            decode(&blob, 43),
            Err(SaveStateError::ContentMismatch)
        ));

        let truncated = &blob[..SAVE_STATE_HEADER_LEN - 1];
        assert!(matches!(
            decode(truncated, 42),
            Err(SaveStateError::BadSignature)
        ));
    }
}
