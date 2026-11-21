//! Offline compatibility fixes for specific guest executables.

use crate::mem::{GuestMem, MemError};
use crate::save::content_fingerprint;

pub(crate) fn is_gold_miner_qvga(container: &[u8]) -> bool {
    container.len() == 194_174 && content_fingerprint([container]) == 0x9ee3_83e6_808c_a8df
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
