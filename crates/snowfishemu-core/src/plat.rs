//! Platform service table layout.
//!
//! The guest runtime reaches platform services through a fixed structure
//! planted in the plat region: a `root` function-pointer table whose slots
//! the guest loads and calls, a small `reg` record holding the canvas
//! descriptors, per-slot stub trampolines, and a handful of one-off records
//! (memory descriptor, app record). The emulator writes this layout before
//! booting the guest and intercepts execution at the stub addresses to
//! implement each slot natively.

use crate::consts::{HEAP_BASE, PLAT_BASE};

/// Guest address of the platform root (API) table.
pub const ROOT_ADDR: u32 = PLAT_BASE;
/// Span of the root table the layout initializes.
pub const ROOT_SIZE: u32 = 0x600;
/// Guest address of the reg record (platform magic + canvas descriptors).
pub const REG_ADDR: u32 = PLAT_BASE + 0x600;
/// Guest address of the canvas descriptor (above the stub area).
pub const CANVAS_ADDR: u32 = STUB_ADDR + 0x80;
/// Guest address of the 240x320 RGB565 framebuffer.
pub const FB_ADDR: u32 = CANVAS_ADDR + 0x10;
/// Framebuffer size in bytes (240 * 320 * 2).
pub const FB_SIZE: u32 = 0x25800;
/// Framebuffer dimensions.
pub const FB_WIDTH: u32 = 240;
pub const FB_HEIGHT: u32 = 320;
/// Guest address of the vtable record.
pub const VTABLE_ADDR: u32 = PLAT_BASE + 0x640;
/// Guest address of the app record the entry code reads.
pub const APP_RECORD_ADDR: u32 = PLAT_BASE + 0x650;
/// Guest address of the stub area shared by most root slots.
pub const STUB_ADDR: u32 = PLAT_BASE + 0x690;
/// Guest address of the fopen wrapper stub (root slots 0x58/0x5C).
pub const STUB_FOPEN_ADDR: u32 = STUB_ADDR + 0x58;
/// Guest address of the state-store stub (root slot 0x524).
pub const STUB_STORE_ADDR: u32 = STUB_ADDR + 0x60;
/// Guest address of the passthrough (`bx lr`) stub for un-planted slots.
pub const STUB_PASSTHROUGH_ADDR: u32 = STUB_ADDR + 0x78;
/// Guest address of the memory descriptor record.
pub const MEMDESC_ADDR: u32 = PLAT_BASE + 0x25F20;
/// Guest address of the cell the state-store stub writes r2 into.
pub const STATE_PTR_ADDR: u32 = PLAT_BASE + 0x25F30;
/// Guest address of the synthetic service object handed out by root slot
/// +0x524 (`service(type, id, &out)`).
///
/// The guests do not just receive the pointer: they immediately call
/// through `out->vtable[0x3C]`, so the object has to be a real guest
/// structure with a callable method table. It lives in the slack above the
/// memory descriptor, next to a table of plain return-0 trampolines.
pub const SERVICE_OBJ_ADDR: u32 = PLAT_BASE + 0x25F40;
/// Guest address of the synthetic service object's method table.
pub const SERVICE_VTABLE_ADDR: u32 = PLAT_BASE + 0x25F80;
/// Methods in the synthetic service object's table (the highest offset the
/// guests index is +0x3C, so 16 entries cover it with head-room).
pub const SERVICE_VTABLE_LEN: u32 = 24;

/// Per-stub value trampolines: each returns a fixed constant in r0.
pub const STUB_HEAP_BASE: u32 = STUB_ADDR + 0x08;
pub const STUB_CODE_BASE: u32 = STUB_ADDR + 0x18;
pub const STUB_WIDTH: u32 = STUB_ADDR + 0x28;
pub const STUB_HEIGHT: u32 = STUB_ADDR + 0x38;
pub const STUB_CANVAS: u32 = STUB_ADDR + 0x48;

/// Root-table slot offsets that point at the generic return-0 stub.
pub const ROOT_SLOTS_GENERIC: &[u32] = &[
    0x000, 0x010, 0x014, 0x018, 0x01C, 0x028, 0x02C, 0x038, 0x03C, 0x040, 0x048, 0x050, 0x060,
    0x064, 0x068, 0x07C, 0x0AC, 0x0B0, 0x0C8, 0x0CC, 0x0D0, 0x0D4, 0x0D8, 0x0DC, 0x0E0, 0x0E4,
    0x0E8, 0x0EC, 0x0F0, 0x108, 0x11C, 0x12C, 0x130, 0x140, 0x14C, 0x154, 0x15C, 0x180, 0x184,
    0x194, 0x198, 0x1B8, 0x1E4, 0x264, 0x4E8, 0x4F4, 0x500, 0x504, 0x534, 0x568, 0x56C, 0x570,
];

/// Slot pointing at the canvas stub (returns the canvas descriptor).
pub const ROOT_SLOT_CANVAS: u32 = 0x004;
/// Slot pointing at the width stub (returns 240).
pub const ROOT_SLOT_WIDTH: u32 = 0x008;
/// Slot pointing at the height stub (returns 320).
pub const ROOT_SLOT_HEIGHT: u32 = 0x00C;
/// Slot pointing at the heap-base stub.
pub const ROOT_SLOT_HEAP: u32 = 0x030;
/// Slot pointing at the code-base stub.
pub const ROOT_SLOT_CODE: u32 = 0x034;
/// Slots pointing at the fopen stub.
pub const ROOT_SLOTS_FOPEN: &[u32] = &[0x058, 0x05C];
/// Slots pointing at the plain return-1 stub.
pub const ROOT_SLOTS_RET1: &[u32] = &[0x04C];
/// Slot pointing at the state-store stub.
pub const ROOT_SLOT_STATE: u32 = 0x524;

/// reg-record magic (`"10FS"`).
pub const REG_MAGIC: u32 = 0x5346_3031;

/// ARM trampoline: `mov r0,#0; bx lr`.
pub const STUB_RET0: [u8; 8] = [0x00, 0x00, 0xA0, 0xE3, 0x1E, 0xFF, 0x2F, 0xE1];
/// ARM trampoline: `bx lr` — passthrough for slots the platform build does
/// not implement. The reference device walk leaves r0 (the first argument)
/// unchanged; returning through this stub reproduces that net effect.
pub const STUB_PASSTHROUGH: [u8; 4] = [0x1E, 0xFF, 0x2F, 0xE1];
/// ARM trampoline: `mov r0,#1; bx lr`.
pub const STUB_RET1: [u8; 8] = [0x01, 0x00, 0xA0, 0xE3, 0x1E, 0xFF, 0x2F, 0xE1];
/// ARM trampoline: `ldr r0,[pc,#0]; bx lr; .word <value>` — the value lives
/// at stub+8 and is filled in per stub.
pub const STUB_VALUE_HEAD: [u8; 8] = [0x00, 0x00, 0x9F, 0xE5, 0x1E, 0xFF, 0x2F, 0xE1];
/// ARM trampoline: `ldr r3,[pc,#8]; str r2,[r3]; mov r0,#1; bx lr;
/// .word <ptr>; .word 0` — stores the guest's r2 at the pointer.
pub const STUB_STORE_HEAD: [u8; 16] = [
    0x08, 0x30, 0x9F, 0xE5, 0x00, 0x30, 0x82, 0xE5, 0x01, 0x00, 0xA0, 0xE3, 0x1E, 0xFF, 0x2F, 0xE1,
];

/// Fixed canvas dimensions of QVGA builds.
pub const SCREEN_WIDTH: u32 = 240;
pub const SCREEN_HEIGHT: u32 = 320;

#[inline]
fn w(mem: &mut crate::mem::GuestMem, addr: u32, value: u32) {
    mem.write_u32(addr, value)
        .expect("platform layout writes fixed mapped addresses");
}

#[inline]
fn w16(mem: &mut crate::mem::GuestMem, addr: u32, value: u16) {
    mem.write_u16(addr, value)
        .expect("platform layout writes fixed mapped addresses");
}

#[inline]
fn wb(mem: &mut crate::mem::GuestMem, addr: u32, data: &[u8]) {
    mem.write(addr, data)
        .expect("platform layout writes fixed mapped addresses");
}

/// Writes the full platform layout into guest memory.
///
/// `mem` is indexed by guest address (the machine's memory map covers the
/// plat region); `code_base` is the load address of the code segment and
/// `image_size` its total size, used for the loader info block around the
/// image.
pub fn layout(mem: &mut crate::mem::GuestMem, code_base: u32, image_size: u32) {
    // --- root table -------------------------------------------------------
    for slot in ROOT_SLOTS_GENERIC {
        w(mem, ROOT_ADDR + slot, STUB_ADDR);
    }
    w(mem, ROOT_ADDR + ROOT_SLOT_CANVAS, STUB_CANVAS);
    w(mem, ROOT_ADDR + ROOT_SLOT_WIDTH, STUB_WIDTH);
    w(mem, ROOT_ADDR + ROOT_SLOT_HEIGHT, STUB_HEIGHT);
    w(mem, ROOT_ADDR + ROOT_SLOT_HEAP, STUB_HEAP_BASE);
    w(mem, ROOT_ADDR + ROOT_SLOT_CODE, STUB_CODE_BASE);
    for slot in ROOT_SLOTS_FOPEN {
        w(mem, ROOT_ADDR + slot, STUB_FOPEN_ADDR);
    }
    for slot in ROOT_SLOTS_RET1 {
        w(mem, ROOT_ADDR + slot, STUB_FOPEN_ADDR);
    }
    w(mem, ROOT_ADDR + ROOT_SLOT_STATE, STUB_STORE_ADDR);

    // --- reg record -------------------------------------------------------
    w(mem, REG_ADDR, REG_MAGIC);
    w(mem, REG_ADDR + 0x04, ROOT_ADDR);
    w(mem, REG_ADDR + 0x08, STUB_ADDR);
    // Canvas: {framebuffer, width, height, stride}.
    w(mem, CANVAS_ADDR, FB_ADDR);
    w(mem, CANVAS_ADDR + 0x04, SCREEN_WIDTH);
    w(mem, CANVAS_ADDR + 0x08, SCREEN_HEIGHT);
    w(mem, CANVAS_ADDR + 0x0C, SCREEN_WIDTH);

    // --- vtable and app record --------------------------------------------
    w(mem, VTABLE_ADDR, code_base);
    w(mem, APP_RECORD_ADDR, VTABLE_ADDR);
    w(mem, APP_RECORD_ADDR + 0x04, code_base);
    w16(mem, APP_RECORD_ADDR + 0x08, 0x104);
    w16(mem, APP_RECORD_ADDR + 0x0A, 0);
    w(mem, APP_RECORD_ADDR + 0x0C, code_base);
    w(mem, APP_RECORD_ADDR + 0x10, image_size);
    w(mem, APP_RECORD_ADDR + 0x18, 0);

    // --- stub area ---------------------------------------------------------
    wb(mem, STUB_ADDR, &STUB_RET0);
    // Default-fill every un-planted root slot with the passthrough stub so a
    // game calling an API this build lacks gets its arguments back (the
    // reference device's net behavior) instead of jumping to zero.
    let planted: &[u32] = &[
        ROOT_SLOT_CANVAS,
        ROOT_SLOT_WIDTH,
        ROOT_SLOT_HEIGHT,
        ROOT_SLOT_HEAP,
        ROOT_SLOT_CODE,
        ROOT_SLOT_STATE,
    ];
    for off in (0..ROOT_SIZE).step_by(4) {
        let planted_here = ROOT_SLOTS_GENERIC.contains(&off)
            || ROOT_SLOTS_FOPEN.contains(&off)
            || ROOT_SLOTS_RET1.contains(&off)
            || planted.contains(&off);
        if !planted_here {
            w(mem, ROOT_ADDR + off, STUB_PASSTHROUGH_ADDR);
        }
    }
    // The passthrough stub (`bx lr`) lives right after the value stubs.
    wb(mem, STUB_PASSTHROUGH_ADDR, &STUB_PASSTHROUGH);
    wb(mem, STUB_FOPEN_ADDR, &STUB_RET1);
    for (stub, value) in [
        (STUB_HEAP_BASE, HEAP_BASE),
        (STUB_CODE_BASE, crate::consts::CODE_REGION_BASE),
        (STUB_WIDTH, SCREEN_WIDTH),
        (STUB_HEIGHT, SCREEN_HEIGHT),
        (STUB_CANVAS, CANVAS_ADDR),
    ] {
        wb(mem, stub, &STUB_VALUE_HEAD);
        w(mem, stub + 8, value);
    }
    wb(mem, STUB_STORE_ADDR, &STUB_STORE_HEAD);
    w(mem, STUB_STORE_ADDR + 0x10, STATE_PTR_ADDR);

    // --- memory descriptor -------------------------------------------------
    w(mem, MEMDESC_ADDR, 8);
    w(mem, MEMDESC_ADDR + 4, 0x1000);

    // --- synthetic service object (root slot +0x524) -----------------------
    // A guest that asks for a service immediately calls through the returned
    // object, so the object's first word is a method table whose entries all
    // point at the generic return-0 trampoline: the call is answered instead
    // of jumping to 0.
    for i in 0..SERVICE_VTABLE_LEN {
        w(mem, SERVICE_VTABLE_ADDR + i * 4, STUB_ADDR);
    }
    w(mem, SERVICE_OBJ_ADDR, SERVICE_VTABLE_ADDR);

    // --- loader info block around the code image ---------------------------
    w(mem, code_base - 8, REG_ADDR);
    w(mem, code_base - 4, ROOT_ADDR);
    w(mem, code_base + image_size - 8, ROOT_ADDR);
    w(mem, code_base + 0xC0, ROOT_ADDR);
    // Global cell in the guest image pointing at the memory descriptor.
    w(mem, code_base + 0xBC, MEMDESC_ADDR);
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::{SCREEN_HEIGHT, SCREEN_WIDTH};
    use crate::consts::{CODE_BASE, HEAP_BASE};
    use crate::mem::GuestMem;

    #[test]
    fn layout_plants_expected_values() {
        let mut mem = GuestMem::new(0x10000);
        let image_size = 0x1000;
        layout(&mut mem, CODE_BASE, image_size);

        // Generic slots and per-slot stubs.
        assert_eq!(mem.read_u32(ROOT_ADDR).unwrap(), STUB_ADDR);
        assert_eq!(
            mem.read_u32(ROOT_ADDR + ROOT_SLOT_WIDTH).unwrap(),
            STUB_WIDTH
        );
        assert_eq!(
            mem.read_u32(ROOT_ADDR + ROOT_SLOT_HEIGHT).unwrap(),
            STUB_HEIGHT
        );
        assert_eq!(
            mem.read_u32(ROOT_ADDR + ROOT_SLOTS_FOPEN[1]).unwrap(),
            STUB_FOPEN_ADDR
        );
        assert_eq!(mem.read_u32(ROOT_ADDR + 0x4C).unwrap(), STUB_FOPEN_ADDR);
        assert_eq!(
            mem.read_u32(ROOT_ADDR + ROOT_SLOT_STATE).unwrap(),
            STUB_STORE_ADDR
        );

        // reg record and canvas.
        assert_eq!(mem.read_u32(REG_ADDR).unwrap(), REG_MAGIC);
        assert_eq!(mem.read_u32(REG_ADDR + 4).unwrap(), ROOT_ADDR);
        assert_eq!(mem.read_u32(CANVAS_ADDR + 4).unwrap(), SCREEN_WIDTH);
        assert_eq!(mem.read_u32(CANVAS_ADDR + 8).unwrap(), SCREEN_HEIGHT);
        assert_eq!(mem.read_u32(CANVAS_ADDR).unwrap(), FB_ADDR);

        // Value stubs return their constants through ldr r0,[pc,#0].
        assert_eq!(mem.read_u32(STUB_HEAP_BASE + 8).unwrap(), HEAP_BASE);
        assert_eq!(mem.read_u32(STUB_HEIGHT + 8).unwrap(), SCREEN_HEIGHT);
        assert_eq!(mem.read_u32(STUB_CANVAS + 8).unwrap(), CANVAS_ADDR);
        // Framebuffer sits right above the canvas in the plat region.
        assert_eq!(FB_ADDR, CANVAS_ADDR + 0x10);

        // Loader info block around the image.
        assert_eq!(mem.read_u32(CODE_BASE - 8).unwrap(), REG_ADDR);
        assert_eq!(mem.read_u32(CODE_BASE - 4).unwrap(), ROOT_ADDR);
        assert_eq!(mem.read_u32(CODE_BASE + image_size - 8).unwrap(), ROOT_ADDR);
        assert_eq!(mem.read_u32(CODE_BASE + 0xC0).unwrap(), ROOT_ADDR);
        assert_eq!(mem.read_u32(CODE_BASE + 0xBC).unwrap(), MEMDESC_ADDR);
    }
}
