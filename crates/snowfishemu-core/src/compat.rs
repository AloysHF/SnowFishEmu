//! Offline compatibility fixes for specific guest executables.

use crate::consts::CODE_BASE;
use crate::mem::{GuestMem, MemError};
use crate::save::content_fingerprint;
use crate::sfa::HEADER_LEN_SFA;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Compatibility {
    Unmodified,
    GoldMiner,
    MengdePe,
    SanguoTd8,
    UniverseStorm,
}

impl Compatibility {
    pub(crate) fn detect(container: &[u8]) -> Self {
        match container.len() {
            156_976 if content_fingerprint([container]) == 0xe09d_72f8_2a77_3eb4 => Self::MengdePe,
            194_174 if content_fingerprint([container]) == 0x9ee3_83e6_808c_a8df => Self::GoldMiner,
            216_610 if content_fingerprint([container]) == 0x6960_1884_6a4d_15f6 => Self::SanguoTd8,
            269_688 if content_fingerprint([container]) == 0x06ba_3789_c896_1ea6 => {
                Self::UniverseStorm
            }
            _ => Self::Unmodified,
        }
    }
}

pub(crate) fn apply_gold_miner_offline_start(mem: &mut GuestMem) -> Result<(), MemError> {
    // The options menu's Start Game action opens the native level menu.
    // Reuse the skipped carrier-check block for the title's quick start.
    let patches: [(u32, u32); 7] = [
        (0x10_4f04, 0xea00_000e), // b 0x104f44 (open level menu)
        (0x10_4f08, 0xe3a0_1001), // mov r1, #1 (first level)
        (0x10_4f0c, 0xeaff_fdca), // b 0x10463c (initialize level)
        // Both the right soft key and OK reach this path as guest key 5.
        (0x10_5080, 0xe351_0005), // cmp r1, #5
        (0x10_5084, 0x112f_ff1e), // bxne lr
        (0x10_5088, 0xe59f_0000), // ldr r0, [pc] (game context)
        (0x10_508c, 0xeaff_ff9d), // b 0x104f08 (quick start)
    ];
    for (addr, word) in patches {
        mem.write_unsuppressed(addr, &word.to_le_bytes())?;
    }
    Ok(())
}

/// Finishes a pending check in the older SDK through the game's own callback.
pub(crate) fn take_mengde_pe_offline_check(
    mem: &mut GuestMem,
) -> Result<Option<(u32, u32)>, MemError> {
    const SDK_STATE_PTR: u32 = CODE_BASE + 0x24_fd8;
    const COMPLETE_CHECK: u32 = CODE_BASE + 0x96_b4;
    let sdk = mem.read_u32(SDK_STATE_PTR)?;
    let pending = sdk.wrapping_add(0x13c);
    if sdk == 0
        || mem.read_u32(pending)? != 1
        || mem.read_u32(sdk.wrapping_add(0x64))? != COMPLETE_CHECK
    {
        return Ok(None);
    }
    let context = mem.read_u32(sdk.wrapping_add(0x68))?;
    // Clear the pending check and release event dispatch before its callback.
    mem.write_u32(pending, 0)?;
    mem.write_u32(sdk.wrapping_add(0x18), u32::MAX)?;
    mem.write_u32(sdk.wrapping_add(0x10), 0)?;
    Ok(Some((COMPLETE_CHECK, context)))
}

pub(crate) fn repair_universe_storm_bitmap_block(src: u32, data: &mut [u8]) {
    match src {
        // The menu background has 8-bit indices, an RGB565 palette and
        // a four-byte color key. Its header advertises a different layout.
        0x12_932f => {
            if let Some(header @ [3, 4, 5]) = data.get_mut(0x184c..0x184f) {
                header.copy_from_slice(&[4, 5, 3]);
            }
        }
        // This 53-pixel-wide sprite has a stray high byte in its width.
        0x12_f658 => {
            if let Some(width @ [0x35, 0x07, 0, 0]) = data.get_mut(0xc89..0xc8d) {
                width[1] = 0;
            }
        }
        _ => {}
    }
}

pub(crate) fn universe_storm_lz_source(package: &[u8], src: u32, len: u32) -> Option<&[u8]> {
    // The built-in music and bitmap archives carry no relocation entries.
    // SDK state writes can overlap their guest addresses, so decode the
    // original package bytes for these embedded assets.
    let audio = src == 0x12_2b94 && len == 2217;
    let bitmap = src >= 0x12_3493 && src.checked_add(len)? <= 0x13_d414;
    if !audio && !bitmap {
        return None;
    }
    let offset = (src - CODE_BASE + HEADER_LEN_SFA) as usize;
    package.get(offset..offset + len as usize)
}
