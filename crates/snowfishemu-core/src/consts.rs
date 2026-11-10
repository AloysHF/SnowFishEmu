//! Guest memory map and loader constants.
//!
//! These mirror the platform loader contract: the code segment is mapped one
//! page above the code region base so the loader info block (reg/root
//! pointers) can live in the reserved head, and fixed service regions are
//! assigned for the stack, application data, the platform table, and the heap.

/// Base of the code region (mapped span starts here).
pub const CODE_REGION_BASE: u32 = 0x0010_0000;
/// Load address of the code segment; relocations are relative to this.
pub const CODE_BASE: u32 = 0x0010_1000;
/// Maximum code image size (code + bss), page-rounded.
pub const CODE_MAX_SIZE: u32 = 0x0020_0000;

/// Stack region: grows down from the top.
pub const STACK_BASE: u32 = 0x0030_0000;
pub const STACK_SIZE: u32 = 0x0010_0000;
pub const STACK_TOP: u32 = STACK_BASE + STACK_SIZE;

/// Application data region.
pub const APP_DATA_BASE: u32 = 0x0044_0000;
pub const APP_DATA_SIZE: u32 = 0x0010_0000;

/// Platform table region (root table, reg struct, stubs).
pub const PLAT_BASE: u32 = 0x0054_0000;
pub const PLAT_SIZE: u32 = 0x0004_0000;

/// Heap region.
pub const HEAP_BASE: u32 = 0x0060_0000;
pub const HEAP_SIZE: u32 = 0x0010_0000;

/// Sentinel return address inside the plat region; guest calls stop here.
pub const STOP_ADDRESS: u32 = 0x0054_1000;
