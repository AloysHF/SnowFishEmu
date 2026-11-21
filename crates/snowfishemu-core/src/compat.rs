//! Offline compatibility fixes for specific guest executables.

use crate::consts::CODE_BASE;
use crate::mem::{GuestMem, MemError};
use crate::save::content_fingerprint;
use crate::sfa::HEADER_LEN_SFA;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Compatibility {
    Unmodified,
    GoldMiner,
    UniverseStorm,
}

impl Compatibility {
    pub(crate) fn detect(container: &[u8]) -> Self {
        match container.len() {
            194_174 if content_fingerprint([container]) == 0x9ee3_83e6_808c_a8df => Self::GoldMiner,
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
