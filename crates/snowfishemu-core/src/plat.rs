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

/// Guest address of the callable service entry handed out by root slot
/// +0x524 for hashed (SIM/ICCID-style) lookups.
///
/// Two SDK revisions disagree about what the returned value is. One jumps
/// straight to it (`ldr pc, [out]`), the other treats it as a method table
/// and loads `entry[0x50]` before calling. A bare code stub fails the second
/// shape: indexing into the shared stub area reads one of the value
/// trampolines' literal words, and the guest ends up executing platform data
/// instead of returning. This stublet satisfies both shapes: its first two
/// words are a return-0 trampoline, and every later word points back at that
/// trampoline.
pub const SERVICE_CALLABLE_ADDR: u32 = SERVICE_VTABLE_ADDR + 0x100;
/// Number of 4-byte words in the callable stublet (covers method offsets
/// up to +0x7C).
pub const SERVICE_CALLABLE_LEN: u32 = 32;

/// The ICCID the identity callable writes when the handset claims a SIM.
///
/// The SDK calls the +0x524 entry with r0 = its ICCID buffer and reads the
/// subscriber identity back out of it; an empty buffer leaves every account
/// record keyed on nothing and pushes the flow into registration. The
/// string is a plausible 19-digit SIM serial.
pub const SIM_ICCID: &str = "8986012345678901234";

/// ARM stublet: copy the 20-byte ICCID string into r0 and return 0.
///
/// `ldr r1, [pc,#imm]` + four word copies + `mov r0,#0; bx lr`, followed by
/// the string-pointer pool word and the string itself. 17 words total; the
/// callable stublet has 32, so the method-table entries from word 18 on
/// still point at the entry.
pub const ICCID_STUBLET: [u32; 13] = [
    0xE59F102C, // ldr r1, [pc, #0x2c]  -> pool word at +0x34
    0xE5912000, // ldr r2, [r1]
    0xE5802000, // str r2, [r0]
    0xE5912004, // ldr r2, [r1, #4]
    0xE5802004, // str r2, [r0, #4]
    0xE5912008, // ldr r2, [r1, #8]
    0xE5802008, // str r2, [r0, #8]
    0xE591200C, // ldr r2, [r1, #0xc]
    0xE580200C, // str r2, [r0, #0xc]
    0xE5912010, // ldr r2, [r1, #0x10]
    0xE5802010, // str r2, [r0, #0x10]
    0xE3A00000, // mov r0, #0
    0xE12FFF1E, // bx lr
];

/// Builds the ICCID stublet image: the code words, the string-pointer pool
/// entry, and the string bytes (NUL-padded to a word boundary).
pub fn iccid_stublet_image() -> Vec<u8> {
    let mut v = Vec::new();
    for word in ICCID_STUBLET {
        v.extend_from_slice(&word.to_le_bytes());
    }
    // The ldr's pool word sits at entry+0x34 (pc = entry+8, imm 0x2c) and
    // points at the string at entry+0x38.
    let string_addr = SERVICE_CALLABLE_ADDR + 0x38;
    v.extend_from_slice(&string_addr.to_le_bytes());
    v.extend_from_slice(SIM_ICCID.as_bytes());
    v.push(0); // NUL
    while v.len() % 4 != 0 {
        v.push(0);
    }
    v
}

/// Guest address of the second callable entry: the subscriber-identity
/// record stublet installed when `SF_IDENTITY` is set (see
/// [`identity_record_image`]). Sits in the slack behind the first callable.
pub const SERVICE_CALLABLE2_ADDR: u32 = SERVICE_CALLABLE_ADDR + SERVICE_CALLABLE_LEN * 4;

/// The +0x524 type/id pair that requests the subscriber-identity record.
///
/// The pair is the shared SDK's hash of its identity service and appears
/// unchanged in every SnowFish V2 image of the corpus. The callable handed
/// back for this lookup is expected to write the whole identity record —
/// an XOR-0xF5-encoded, comma-separated field list closed by a raw
/// checksum byte — into the buffer the caller passes in r0. The SDK
/// verifies the checksum, splits the decoded list into its identity
/// fields and refuses the pay session ("invalid SIM") when the record
/// does not parse.
pub const IDENTITY_RECORD_TYPE: u32 = 0x794C_973A;
pub const IDENTITY_RECORD_ID: u32 = 0xEADB_BDAE;

/// Encodes the subscriber-identity record from the plain comma-separated
/// field list: every payload byte is XOR-0xF5 encoded (the SDK decodes it
/// while scanning for the field separators) and a raw trailing byte holds
/// the sum of the encoded payload, which is what the SDK's checksum gate
/// compares against.
pub fn identity_record(fields: &str) -> Vec<u8> {
    let mut v: Vec<u8> = fields.bytes().map(|b| b ^ 0xF5).collect();
    let sum = v.iter().fold(0u8, |a, b| a.wrapping_add(*b));
    v.push(sum);
    v
}

/// ARM stublet for the identity record: copies the encoded record into the
/// caller's buffer (`ldr r1, =record; ldr r2, =len; byte loop over r0`)
/// and returns the record length, which the caller uses as the copy
/// length for its own identity cells.
pub fn identity_record_image(record: &[u8]) -> Vec<u8> {
    // Layout: 10 code words, two pool words (the record address and its
    // length), then the record bytes.
    let rec_addr = SERVICE_CALLABLE2_ADDR + 0x34;
    let code: [u32; 10] = [
        0xE59F1024, // ldr r1, [pc, #0x24]  -> pool word at +0x2C (the record)
        0xE59F2024, // ldr r2, [pc, #0x24]  -> pool word at +0x30 (the length)
        0xE3A03000, // mov r3, #0
        0xE7D1C003, // ldrb ip, [r1, r3]
        0xE7C0C003, // strb ip, [r0, r3]
        0xE2833001, // add r3, r3, #1
        0xE1530002, // cmp r3, r2
        0xBAFFFFFA, // blt -6               -> the ldrb above
        0xE1A00002, // mov r0, r2           (the length as the return)
        0xE12FFF1E, // bx lr
    ];
    let mut v = Vec::new();
    for word in code {
        v.extend_from_slice(&word.to_le_bytes());
    }
    // +0x28..0x2C: slack. +0x2C: the record-address pool word, +0x30: the
    // length pool word, +0x34: the record bytes.
    while v.len() < 0x2C {
        v.push(0);
    }
    v.extend_from_slice(&rec_addr.to_le_bytes());
    while v.len() < 0x30 {
        v.push(0);
    }
    v.extend_from_slice(&(record.len() as u32).to_le_bytes());
    v.extend_from_slice(record);
    while v.len() % 4 != 0 {
        v.push(0);
    }
    v
}

/// Guest addresses of the synthetic HTTP transaction wrapper the +0x56C
/// delivery plants for the SDK's response parser.
///
/// The SDK's result consumer dereferences the planted object as a wrapper
/// (`obj->vtable[0xC](obj, &out)` hands back a data pointer whose word[0]
/// is the HTTP status, word[0x10] the body length and word[0x14] the body
/// pointer), so the old plant — a pointer at static image text — made the
/// parser execute string bytes and walk a garbage body. The wrapper here
/// has a real method table: vtable[0xC] is a stublet that stores
/// `wrapper[4]` (the data block) into the caller's out slot, and
/// vtable[4] is the shared passthrough (`bx lr`) the release paths call.
pub const TX_DATA_ADDR: u32 = SERVICE_CALLABLE2_ADDR + 0x200;
pub const TX_BODY_ADDR: u32 = TX_DATA_ADDR + 0x20;
pub const TX_BODY_MAX: u32 = 0x400;
pub const TX_WRAPPER_ADDR: u32 = TX_BODY_ADDR + TX_BODY_MAX;
pub const TX_WRAPPER_VT_ADDR: u32 = TX_WRAPPER_ADDR + 0x20;
pub const TX_GETTER_ADDR: u32 = TX_WRAPPER_VT_ADDR + 0x20;

/// Words of the transaction wrapper's status getter:
/// `*out = wrapper[4]` (the data block), then return.
pub const TX_GETTER: [u32; 3] = [
    0xE5900004, // ldr r0, [r0, #4]
    0xE5810000, // str r0, [r1]
    0xE12FFF1E, // bx lr
];

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

    // --- callable service entry (hashed +0x524 lookups) --------------------
    wb(mem, SERVICE_CALLABLE_ADDR, &STUB_RET0);
    for i in 2..SERVICE_CALLABLE_LEN {
        w(mem, SERVICE_CALLABLE_ADDR + i * 4, SERVICE_CALLABLE_ADDR);
    }

    // --- synthetic HTTP transaction wrapper (+0x56C delivery) -------------
    // wrapper[0] = the method table, wrapper[4] = the data block; the
    // table's [0xC] is the status getter, [4] the release passthrough.
    // [data+0] = the HTTP status (200), [data+0x10] = the body length
    // (the delivery hook updates it), [data+0x14] = the body pointer.
    for i in 0..8 {
        w(mem, TX_WRAPPER_VT_ADDR + i * 4, STUB_PASSTHROUGH_ADDR);
    }
    w(mem, TX_WRAPPER_VT_ADDR + 0xC, TX_GETTER_ADDR);
    wb(
        mem,
        TX_GETTER_ADDR,
        &TX_GETTER
            .iter()
            .copied()
            .flat_map(u32::to_le_bytes)
            .collect::<Vec<u8>>(),
    );
    w(mem, TX_WRAPPER_ADDR, TX_WRAPPER_VT_ADDR);
    w(mem, TX_WRAPPER_ADDR + 4, TX_DATA_ADDR);
    // The status stays non-2xx: the +0x56C slot's callers are the HTTP
    // layer's cancel/reset calls, and a 2xx here sends the boot give-up
    // handler into the success branch (the ssu account write) before the
    // flow has its account objects. The release branch is what the old
    // plant's garbage status produced, minus the garbage-pointer walk.
    w(mem, TX_DATA_ADDR, 0);
    w(mem, TX_DATA_ADDR + 0x10, 0);
    w(mem, TX_DATA_ADDR + 0x14, TX_BODY_ADDR);

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

    /// The hashed service lookup hands out one value that has to survive
    /// both a direct `ldr pc, [entry]` and a virtual `ldr pc, [entry + k]`.
    /// Indexing the shared code stub instead reads `STUB_CANVAS`'s literal
    /// word, which sends the guest into platform data and aborts the boot
    /// event; the dedicated stublet keeps every method offset callable.
    #[test]
    fn callable_service_entry_answers_direct_and_virtual_calls() {
        let mut mem = GuestMem::new(0x10000);
        layout(&mut mem, CODE_BASE, 0x1000);

        let mut head = [0u8; 8];
        mem.read(SERVICE_CALLABLE_ADDR, &mut head).unwrap();
        assert_eq!(
            head, STUB_RET0,
            "direct calls must land on the return-0 trampoline"
        );
        assert_ne!(SERVICE_CALLABLE_ADDR, STUB_ADDR);
        for i in 2..SERVICE_CALLABLE_LEN {
            assert_eq!(
                mem.read_u32(SERVICE_CALLABLE_ADDR + i * 4).unwrap(),
                SERVICE_CALLABLE_ADDR,
                "method offset {:#x} must resolve to code",
                i * 4
            );
        }
        // The concrete failure mode: indexing the shared stub area yields a
        // canvas pointer rather than a callable address.
        assert_eq!(mem.read_u32(STUB_ADDR + 0x50).unwrap(), CANVAS_ADDR);
    }
}
