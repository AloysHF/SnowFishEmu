//! Emulator machine: package loading, wrapper dispatch, and the guest ABI.
//!
//! The guest reaches platform services through compiler wrapper stublets
//! scanned out of the code image (see [`scan_wrappers`]); hooked slots run
//! natively here and resume at the wrapper epilogue, everything else
//! executes through the planted root-table stubs unchanged.

use crate::consts::{
    CODE_BASE, FRAME_MS, HEAP_BASE, STACK_TOP, STOP_ADDRESS, TIMER_JOB_ADDR, TIMER_JOB_SIZE,
};
use crate::cpu::ArmCpu;
use crate::gdi::Gdi;
use crate::lz;
use crate::mem::GuestMem;
use crate::plat::{self, APP_RECORD_ADDR};
use crate::sfa::{ResourcePack, SfaImage};
use log::{debug, info, warn};
use thiserror::Error;

/// Machine and package errors.
#[derive(Debug, Error)]
pub enum MachineError {
    #[error("package error: {0}")]
    Package(#[from] crate::sfa::SfaError),
    #[error("memory error: {0}")]
    Memory(#[from] crate::mem::MemError),
    #[error("no wrapper for slot +0x{0:X}")]
    MissingWrapper(u32),
    #[error("guest call faulted at pc 0x{0:08X}")]
    GuestFault(u32),
    #[error("key index out of range: {0}")]
    BadKey(u8),
}

/// One wrapper stublet found in the code image.
#[derive(Debug, Clone, Copy)]
pub struct Wrapper {
    /// Root-table slot offset (imm12 of the `ldr pc` word).
    pub slot: u32,
    /// Address of the `ldr rX,[rX,#-4]` instruction (interception point).
    pub hook: u32,
    /// Address of the epilogue (`pop {pc}` / `add sp` landing).
    pub ret: u32,
}

/// Scans the code image for platform-call wrapper stublets.
///
/// Pattern (per word, at address A): `ldr rX,[rX,#-4]`, `mov lr,pc`,
/// `ldr pc,[rX,#imm]`, then either `pop {pc}` / `ldr pc,[sp],#4` (ret =
/// A+0xC) or `add sp,sp,#imm` followed by one of those (ret = A+0xC, the
/// `add sp` itself; the epilogue then runs natively).
pub fn scan_wrappers(code: &[u8]) -> Vec<Wrapper> {
    let rd = |i: usize| -> u32 {
        if i + 4 <= code.len() {
            u32::from_le_bytes([code[i], code[i + 1], code[i + 2], code[i + 3]])
        } else {
            0
        }
    };
    let mut out = Vec::new();
    let words = code.len() / 4;
    for w in 0..words {
        let w0 = rd(w * 4);
        let w1 = rd(w * 4 + 4);
        let w2 = rd(w * 4 + 8);
        let w3 = rd(w * 4 + 12);
        let w4 = rd(w * 4 + 16);
        let loader = w0 == 0xE513_3004 || w0 == 0xE51C_C004;
        if !loader || w1 != 0xE1A0_E00F {
            continue;
        }
        let ldr_pc = w2 & 0xFFFF_F000;
        if ldr_pc != 0xE593_F000 && ldr_pc != 0xE59C_F000 {
            continue;
        }
        let slot = w2 & 0xFFF;
        let ret_pop = w3 == 0xE8BD_8000 || w3 == 0xE49D_F004;
        let ret_add_sp =
            w3 & 0xFFFF_F000 == 0xE28D_D000 && (w4 == 0xE8BD_8000 || w4 == 0xE49D_F004);
        if !ret_pop && !ret_add_sp {
            continue;
        }
        let hook = CODE_BASE + (w * 4) as u32;
        let ret = hook + 0xC;
        out.push(Wrapper { slot, hook, ret });
    }
    out
}

/// Converts a guest UTF-16LE string to ASCII (`>0x7F` becomes `'?'`).
fn guest_utf16_to_ascii(mem: &GuestMem, ptr: u32, cap: usize) -> Result<String, MachineError> {
    let mut out = String::new();
    let mut i: u32 = 0;
    while (out.len() + 1) < cap {
        let lo = mem.read_u8(ptr + i * 2)?;
        let hi = mem.read_u8(ptr + i * 2 + 1)?;
        let word = (hi as u32) << 8 | lo as u32;
        if word == 0 {
            break;
        }
        out.push(if word > 0x7F { '?' } else { word as u8 as char });
        i += 1;
    }
    Ok(out)
}

/// Reads a NUL-terminated guest string (capped).
fn guest_read_string(mem: &GuestMem, ptr: u32, cap: usize) -> Result<String, MachineError> {
    let mut out = String::new();
    for i in 0..cap {
        let b = mem.read_u8(ptr + i as u32)?;
        if b == 0 || b >= 0x7F {
            break;
        }
        out.push(b as char);
    }
    Ok(out)
}

/// Classifies a basename against the platform's virtual-file list.
/// Returns `Some(kind)` for platform files (fopen is refused) and `None`
/// for ordinary files.
fn vfile_kind(basename: &str) -> Option<u32> {
    if basename == "__sfssuid" {
        return Some(0);
    }
    if basename == "ssu112" {
        return Some(1);
    }
    if basename == "__sfnetinfo__" {
        return Some(2);
    }
    if basename.contains("__sfgpsav") {
        return Some(4);
    }
    if basename.contains("_save_") && basename.contains(".dat") {
        return Some(4);
    }
    if basename.contains("_login") && basename.contains(".dat") {
        return Some(3);
    }
    if let Some(rest) = basename.strip_prefix("ssu") {
        let digits = rest.split('.').next().unwrap_or("");
        if !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()) {
            return Some(1);
        }
    }
    None
}

fn basename(path: &str) -> &str {
    path.rsplit(['\\', '/']).next().unwrap_or(path)
}

/// Whether a path names one of the SDK's own service records.
///
/// Those are the files the platform store has to remember across handles:
/// the `__`-prefixed billing records and the account record
/// (`__sfssuid`/`ssu<id>`) the guest rewrites whenever it registers.
fn service_file(path: &str) -> bool {
    let base = basename(path);
    base.starts_with("__") || matches!(vfile_kind(base), Some(0) | Some(1))
}

/// Where a file slot's bytes come from.
///
/// The distinction matters because a guest that writes a file has to read
/// the same bytes back: the resource-package download writes the package
/// to disk and then re-opens it to check its size, so a write has to land
/// in memory and be served by the following read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub(crate) enum SlotBacking {
    /// Slot is free.
    #[default]
    None,
    /// Guest-writable memory file; contents are what the guest wrote.
    Memory,
    /// Read-only view of the loaded game container.
    Container,
    /// Read-only view of the loaded resource container.
    Res,
    /// Read-only view of a resource pack extracted from a resource
    /// container, identified by its index in the machine's pack table.
    Pack(usize),
}

/// One open file slot served from container bytes.
#[derive(Debug, Default, Clone, serde::Serialize, serde::Deserialize)]
pub(crate) struct FileSlot {
    pub(crate) in_use: bool,
    pub(crate) offset: usize,
    pub(crate) size: usize,
    pub(crate) name: String,
    /// In-memory backing store for writable files (empty for the
    /// container/res-backed read-only slots).
    pub(crate) data: Vec<u8>,
    /// Which store backs this slot.
    pub(crate) backing: SlotBacking,
}

/// Queued guest event.
#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
pub struct Event {
    pub selector: i32,
    pub param: u32,
}

/// Event queue (64 entries, drop-on-full like the reference).
#[derive(Debug, Default, Clone, serde::Serialize, serde::Deserialize)]
pub struct EventQueue {
    events: Vec<Event>,
}

impl EventQueue {
    pub fn post(&mut self, selector: i32, param: u32) {
        if self.events.len() < 64 {
            self.events.push(Event { selector, param });
        }
    }
    pub fn pop(&mut self) -> Option<Event> {
        if self.events.is_empty() {
            None
        } else {
            Some(self.events.remove(0))
        }
    }
}

/// State of the optional offline HTTP responder.
///
/// The shared SDK opens one socket, writes its request through +0xE4, then
/// reads the whole response through +0xE0 before its completion callback is
/// allowed to run. On a real handset the carrier-facing endpoint answers;
/// here the platform can stand in for that endpoint locally. The responder
/// is inert unless enabled.
#[derive(Default)]
struct HttpResponder {
    enabled: bool,
    response: Vec<u8>,
    pos: usize,
    /// Async request descriptor handed to +0x570: `[status, callback, arg]`.
    request: Option<u32>,
    /// `(handle, socket object)` registered through +0xE8.
    attached: Option<(u32, u32)>,
    notified: bool,
    /// When set, a request whose path contains `asset_match` is answered with
    /// these local bytes instead of the payload-free 200. This is how a
    /// side-loaded resource package (a real local file, not invented server
    /// data) is delivered for the download the SDK would otherwise make.
    asset: Vec<u8>,
    asset_match: String,
    serve_asset: bool,
    /// The resource pack the current request asks for, if any.
    ///
    /// The guest names the pack in the request path; the pack table maps
    /// that name to the bytes the publisher's container carries.
    pack: Option<usize>,
}

impl HttpResponder {
    /// A well-formed, payload-free HTTP/1.0 answer. The SDK only needs the
    /// status line and an explicit zero-length body to finish the
    /// transaction; synthesising a tariff record would invent server data
    /// the emulator cannot know.
    fn response_bytes() -> Vec<u8> {
        b"HTTP/1.0 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_vec()
    }

    /// Build the response for the current transaction: if the request matched
    /// a resource pack, serve its bytes; otherwise the empty 200 or the
    /// side-loaded asset.
    fn build_response(&mut self, packs: &[ResourcePack]) {
        let body: &[u8] = match self.pack {
            Some(index) => packs.get(index).map(|p| p.bytes.as_slice()).unwrap_or(&[]),
            None if self.serve_asset => self.asset.as_slice(),
            None => &[],
        };
        if body.is_empty() {
            self.response = Self::response_bytes();
            return;
        }
        let head = format!(
            "HTTP/1.0 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        );
        let mut response = head.into_bytes();
        response.extend_from_slice(body);
        self.response = response;
    }

    /// Whether a response was served, fully read and reported to the guest.
    fn transaction_done(&self) -> bool {
        self.notified && !self.response.is_empty() && self.pos >= self.response.len()
    }
}

/// MTK key codes for the 32 logical input slots.
pub const KEY_MAP: [u8; 32] = [
    0x08, 0x09, 0x0A, 0x0B, 0x0C, 0x0D, 0x0E, 0x0F, 0x10, 0x11, 0x01, 0x03, 0x00, 0x04, 0x05, 0x06,
    0x07, 0x16, 0x12, 0x14, 0x13, 0x00, 0x18, 0x19, 0x1A, 0x02, 0x00, 0x00, 0x00, 0x00, 0x00, 0x17,
];

/// The emulator machine.
pub struct SnowFishMachine {
    pub mem: GuestMem,
    pub cpu: ArmCpu,
    code_base: u32,
    image_end: u32,
    wrappers: Vec<Wrapper>,
    handler: u32,
    file_slots: Vec<FileSlot>,
    container: Vec<u8>,
    /// The resource containers supplied next to the package. Each one is a
    /// publisher resource-pack generator; the packs they carry are served
    /// in place of the carrier download the guest would otherwise make.
    res_containers: Vec<Vec<u8>>,
    /// Resource packs extracted from the resource containers, largest first.
    /// Every pack keeps the file name its container declares, so a request
    /// can be answered with the pack it actually asks for.
    res_packs: Vec<ResourcePack>,
    events: EventQueue,
    frame_count: u64,
    lz_calls: u64,
    trace: bool,
    last_pc: u32,
    branch_log: Vec<(u32, u32)>,
    // The response paths already injected once (SF_DCSMS).
    dcsms_injected: std::collections::HashSet<String>,
    // Function-entry skips: (entry, r0) — when the PC reaches `entry` the
    // call returns immediately with r0. Used by the offline billing shim
    // to report the SDK's SMS send as accepted.
    fn_skips: Vec<(u32, u32)>,
    gdi: Gdi,
    timer: Option<GuestTimer>,
    ui_timers: Vec<UiTimer>,
    ui_clock_ms: u64,
    /// Persistent contents of the guest-writable service files (the SMS
    /// billing records), keyed by full path.
    named_files: std::collections::HashMap<String, Vec<u8>>,
    /// Free-play provisioning (`SF_FREE_PLAY`): the platform answers as a
    /// registered, billing-capable handset instead of an empty one. See
    /// [`Self::set_free_play`].
    free_play: bool,
    /// Optional local stand-in for the SDK's carrier-facing HTTP endpoint.
    /// See [`Self::set_offline_http`].
    http: HttpResponder,
    /// Re-deliver the app-activate/resume event on each frame. See
    /// [`Self::set_app_pump`].
    app_pump: bool,
}

/// A platform UI timer armed through slot +0xAC,
/// `handle = arm(context, period_ms, callback)`.
///
/// This is the shared SDK's own message pump: every `period_ms` of
/// emulated time the platform re-enters `callback`, and the callback is
/// what dispatches the app handler's `selector 10` (the guest side picks
/// the parameter — the message id it registered the pump under).
#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
pub struct UiTimer {
    pub period_ms: u32,
    pub callback: u32,
    pub context: u32,
    pub due_ms: u64,
}

/// A guest-registered timer callback (platform slot +0x500,
/// `timerStart(id, callback, context)`).
///
/// The shared SDK pay callback also carries three data-section references
/// decoded from its own literal loads: the pattern-item array, the item
/// count byte, and the match target buffer (see [`Self::decode_tables`]).
#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
pub struct GuestTimer {
    pub id: u32,
    pub entry: u32,
    pub ctx: u32,
    pub item_array: u32,
    pub count_addr: u32,
    pub target_addr: u32,
}

impl GuestTimer {
    /// Decodes the callback's data-table references from its own literal
    /// loads. The shared SDK callback loads the pattern-item array into r6,
    /// the item count byte address into r7 and the match target into r1 via
    /// `ldr rX, [pc, #imm]` within its first 0x80 bytes; anything that does
    /// not fit that shape leaves the fields zero and the offline shim skips.
    fn decode_tables(mem: &GuestMem, entry: u32) -> (u32, u32, u32) {
        let mut item_array = 0;
        let mut count_addr = 0;
        let mut target_addr = 0;
        for off in (0u32..0x80).step_by(4) {
            let Ok(word) = mem.read_u32(entry + off) else {
                break;
            };
            if word & 0xFFFF0000 != 0xE59F0000 {
                continue; // not `ldr rX, [pc, #imm]`
            }
            let rd = (word >> 12) & 0xF;
            let pool = entry + off + 8 + (word & 0xFFF);
            let Ok(value) = mem.read_u32(pool) else {
                continue;
            };
            match rd {
                6 => item_array = value,
                7 => count_addr = value,
                1 => target_addr = value,
                _ => {}
            }
        }
        (item_array, count_addr, target_addr)
    }
}

impl SnowFishMachine {
    /// Loads a package (the whole `.app` container) and prepares the guest.
    ///
    /// `res_containers` are the publisher resource containers that ship next
    /// to the package. Each carries the resource pack the guest would
    /// otherwise download from the carrier endpoint; the packs are extracted
    /// here so the platform can serve the real bytes offline.
    pub fn load(container: Vec<u8>, res_containers: &[Vec<u8>]) -> Result<Self, MachineError> {
        // Static function-entry skips: SF_FSKIP="addr:r0;addr:r0" (hex).
        // When the guest PC reaches `addr` the call returns at once with
        // r0 = `val`. Used by the offline billing shim to report the SDK's
        // SMS send method (the task object's vtable +0x18 entry) as
        // accepted; the address is per-binary, resolved from the task
        // class vtable.
        let mut fn_skips = Vec::new();
        if let Ok(skips) = std::env::var("SF_FSKIP") {
            for pair in skips.split(';') {
                if let Some((a, v)) = pair.split_once(':') {
                    if let (Ok(entry), Ok(val)) =
                        (u32::from_str_radix(a, 16), u32::from_str_radix(v, 16))
                    {
                        fn_skips.push((entry, val));
                    }
                }
            }
        }
        let image = SfaImage::parse(&container)?;
        let mut mem = GuestMem::new(image.header.image_size());
        let image_size = image.header.image_size();
        mem.write(CODE_BASE, &image.code)?;
        // SF_PATCH="addr:insn,addr:insn" (hex): overwrite guest code words
        // after load. Lets an experiment fix a call argument (e.g. the
        // stage value a builder call passes) without editing the binary.
        if let Ok(patches) = std::env::var("SF_PATCH") {
            for pair in patches.split(',') {
                if let Some((a, v)) = pair.split_once(':') {
                    if let (Ok(addr), Ok(word)) =
                        (u32::from_str_radix(a, 16), u32::from_str_radix(v, 16))
                    {
                        let off = addr.wrapping_sub(CODE_BASE) as usize;
                        if off + 4 <= image.code.len() {
                            mem.write(addr, &word.to_le_bytes())?;
                            debug!("[plat] patched {addr:#010x} <- {word:#010x}");
                        }
                    }
                }
            }
        }
        // SF_WATCH="addr,addr" (hex): record the writing instruction for
        // every guest write that touches these addresses.
        if let Ok(vars) = std::env::var("SF_WATCH") {
            mem.watch = vars
                .split(',')
                .filter_map(|a| u32::from_str_radix(a, 16).ok())
                .collect();
        }
        plat::layout(&mut mem, CODE_BASE, image_size);
        let wrappers = scan_wrappers(&image.code);
        info!(
            "loaded .app: image 0x{image_size:X}, {} wrappers: {}",
            wrappers.len(),
            wrappers
                .iter()
                .map(|w| format!("+0x{:X}@{:#X}", w.slot, w.hook))
                .collect::<Vec<_>>()
                .join(" ")
        );
        let trace = std::env::var_os("SF_TRACE").is_some();
        // The resource container carries the resource packs the guest would
        // otherwise download; extract them so the platform can serve the
        // real bytes offline.
        let mut res_packs: Vec<ResourcePack> = Vec::new();
        for data in res_containers {
            let Ok(img) = SfaImage::parse(data) else {
                continue;
            };
            res_packs.extend(ResourcePack::find_all_in(&img));
        }
        // Two containers may declare the same pack name; the larger one
        // wins, so the guest always sees the most complete copy.
        res_packs.sort_by(|a, b| {
            b.bytes
                .len()
                .cmp(&a.bytes.len())
                .then_with(|| a.name.cmp(&b.name))
        });
        res_packs.dedup_by(|a, b| a.name == b.name);
        Ok(Self {
            mem,
            cpu: ArmCpu::new(),
            code_base: CODE_BASE,
            image_end: CODE_BASE + image_size,
            wrappers,
            handler: 0,
            file_slots: vec![FileSlot::default(); 4],
            container,
            res_containers: res_containers.to_vec(),
            res_packs,
            events: EventQueue::default(),
            frame_count: 0,
            lz_calls: 0,
            trace,
            last_pc: 0,
            branch_log: Vec::new(),
            dcsms_injected: std::collections::HashSet::new(),
            fn_skips,
            gdi: Gdi::new(),
            timer: None,
            ui_timers: Vec::new(),
            ui_clock_ms: 0,
            named_files: std::collections::HashMap::new(),
            free_play: std::env::var_os("SF_FREE_PLAY").is_some_and(|v| v != "0"),
            http: HttpResponder {
                enabled: std::env::var_os("SF_OFFLINE_HTTP").is_some_and(|v| v != "0"),
                asset: std::env::var_os("SF_SERVE_ASSET")
                    .and_then(|p| std::fs::read(p).ok())
                    .unwrap_or_default(),
                asset_match: std::env::var("SF_SERVE_MATCH").unwrap_or_else(|_| ".pak".into()),
                ..HttpResponder::default()
            },
            app_pump: std::env::var_os("SF_NO_APP_PUMP").is_none(),
        })
    }

    /// Finds the resource pack a request path asks for.
    ///
    /// The guest names the pack in the request path, e.g.
    /// `.../GM_PAK/<name>.pak`. The pack table holds the name every
    /// publisher container declares, so the request is matched by name
    /// rather than by position.
    fn pack_for_request(&self, request: &str) -> Option<usize> {
        self.res_packs.iter().position(|pack| {
            request
                .to_ascii_lowercase()
                .contains(&pack.name.to_ascii_lowercase())
        })
    }

    /// The resource packs the platform can serve, with the names the
    /// publisher containers declare for them.
    pub fn resource_packs(&self) -> Vec<(String, usize)> {
        self.res_packs
            .iter()
            .map(|p| (p.name.clone(), p.bytes.len()))
            .collect()
    }

    /// Turns free-play provisioning on or off.
    ///
    /// The corpus consists of trial releases: at boot the shared SDK runs its
    /// purchase service, which asks the platform whether the handset can be
    /// billed over the carrier channel (`+0x4C`), looks the subscriber
    /// identity up (`+0x524`), and probes the account record
    /// (`__sfssuid`/`ssu<id>`) on every drive root. A platform that answers
    /// "no" to any of those parks the title on a registration page.
    ///
    /// Free play answers the way a registered handset would. It does not
    /// touch the guest's own state machine: the title still has to accept the
    /// answers and open its own play state.
    pub fn set_free_play(&mut self, on: bool) {
        self.free_play = on;
    }

    /// Whether free-play provisioning is on.
    pub fn free_play(&self) -> bool {
        self.free_play
    }

    /// Enables the local HTTP responder (`SF_OFFLINE_HTTP=1`).
    ///
    /// The SDK's billing endpoint is long gone. With this switch the
    /// platform answers the socket read path itself with a payload-free
    /// `200 OK` and delivers the async completion notification the SDK
    /// registered through +0x570, so the guest's own HTTP result parser
    /// runs instead of the transaction hanging forever.
    pub fn set_offline_http(&mut self, on: bool) {
        self.http.enabled = on;
    }

    /// Whether the local HTTP responder is on.
    pub fn offline_http(&self) -> bool {
        self.http.enabled
    }

    /// Side-loads a local file to answer download requests whose path contains
    /// `asset_match` (default `.pak`). This stands in for the carrier endpoint
    /// that would push a resource package, using a real local artifact rather
    /// than fabricated server data. Pass empty bytes to disable.
    pub fn set_serve_asset(&mut self, data: Vec<u8>) {
        self.http.asset = data;
    }

    /// Sets the substring a request path must contain for the side-loaded asset
    /// to be served (default `.pak`).
    pub fn set_serve_asset_match(&mut self, match_str: String) {
        self.http.asset_match = match_str;
    }

    /// The currently side-loaded asset bytes, if any.
    pub fn serve_asset(&self) -> &[u8] {
        &self.http.asset
    }

    /// Enables or disables the per-frame app-activate/resume pump.
    ///
    /// The shared SDK only advances its screen state machine (and repaints)
    /// on selector 3. A foregrounded handset keeps that event active on the
    /// frame cadence; disabling the pump is useful when diagnosing guests
    /// that should not receive repeated activations.
    pub fn set_app_pump(&mut self, on: bool) {
        self.app_pump = on;
    }

    /// Whether the app-activate/resume pump is on.
    pub fn app_pump(&self) -> bool {
        self.app_pump
    }

    /// Guest address of the state byte (app record +0x16).
    pub fn state_addr(&self) -> u32 {
        APP_RECORD_ADDR + 0x16
    }

    /// Wrapper lookup by hook address.
    fn wrapper_at(&self, hook: u32) -> Option<&Wrapper> {
        self.wrappers.iter().find(|w| w.hook == hook)
    }

    /// Slots implemented natively here (everything else runs natively
    /// through the root-table stubs).
    fn hooked_slot_ret(&self, w: &Wrapper) -> Option<u32> {
        // The offline responder needs the socket/notifier slots that are
        // otherwise answered by the generic return-0 stub. Keep them on
        // that stub when the responder is off so other titles are untouched.
        if self.http.enabled
            && matches!(
                w.slot,
                0xC8 | 0xCC | 0xD4 | 0xD8 | 0xDC | 0xE0 | 0xE8 | 0xEC | 0xF0 | 0x568 | 0x570
            )
        {
            return Some(w.ret);
        }
        match w.slot {
            0x48 | 0x4C | 0x7C | 0x58 | 0x5C | 0x60 | 0x64 | 0x68 | 0x108 | 0x524 | 0xAC | 0xE4 => {
                Some(w.ret)
            }
            0x12C | 0x130 | 0x134 | 0x140 | 0x14C | 0x154 | 0x158 | 0x180 | 0x198 | 0x194
            | 0x184 | 0x15C | 0x010 => Some(w.ret),
            _ => None,
        }
    }

    /// Runs the guest from the current PC until the stop sentinel, the
    /// instruction budget, or a fault. Handles wrapper interception.
    fn run_until_return(&mut self, budget: u64) -> Result<u64, MachineError> {
        let mut executed = 0u64;
        // SF_PCTRACE="lo-hi" (hex): log every fetch inside the range with
        // the link register — bounded per call to keep logs readable.
        let pc_range: Option<(u32, u32)> = std::env::var("SF_PCTRACE").ok().and_then(|s| {
            s.split_once('-').and_then(|(a, b)| {
                match (u32::from_str_radix(a, 16), u32::from_str_radix(b, 16)) {
                    (Ok(a), Ok(b)) => Some((a, b)),
                    _ => None,
                }
            })
        });
        let mut traced = 0u64;
        while executed < budget {
            let pc = self.cpu.pc();
            let aligned = if self.cpu.thumb_mode() {
                pc & !1
            } else {
                pc & !3
            };
            if let Some((lo, hi)) = pc_range {
                if aligned >= lo && aligned <= hi && traced < 300 {
                    traced += 1;
                    debug!(
                        "[trace] pc={aligned:#010x} lr={:#010x} r0={:#010x} r1={:#010x}",
                        self.cpu.reg_get(armv4t_emu::reg::LR),
                        self.cpu.reg_get(0),
                        self.cpu.reg_get(1)
                    );
                }
            }
            if let Some(&(entry, val)) = self.fn_skips.iter().find(|&&(e, _)| e == aligned) {
                debug!("[plat] fn skip @{entry:#010x} -> r0={val:#x}");
                self.cpu.reg_set(0, val);
                let lr = self.cpu.reg_get(armv4t_emu::reg::LR);
                self.cpu.reg_set(armv4t_emu::reg::PC, lr);
                continue;
            }
            if aligned != pc {
                self.cpu.reg_set(0xF, aligned); // PC
            }
            if aligned == STOP_ADDRESS {
                return Ok(executed);
            }
            if aligned < 0x1000 {
                // Fetching from the zero vector: a call through a null
                // pointer. Dump the branch trail for diagnosis.
                debug!("guest executing at {aligned:#010x}");
                for (from, to) in &self.branch_log {
                    debug!("  branch {from:#010x} -> {to:#010x}");
                }
                return Err(MachineError::GuestFault(aligned));
            }
            if let Some(w) = self.wrapper_at(aligned) {
                if let Some(ret) = self.hooked_slot_ret(w) {
                    self.exec_slot(w.slot, ret)?;
                    continue;
                }
                // Un-hooked slot: execute the wrapper body natively so the
                // root-table stub returns its canned value.
                if self.trace || w.slot == 0x48 {
                    debug!(
                        "[plat] native slot +0x{:X} lr={:#010x} r0={:08x} r1={:08x} r2={:08x}",
                        w.slot,
                        self.cpu.reg_get(armv4t_emu::reg::LR),
                        self.cpu.reg_get(0),
                        self.cpu.reg_get(1),
                        self.cpu.reg_get(2)
                    );
                }
                if w.slot == 0x500 {
                    // timerStart(id, callback, context): record the latest
                    // registration so the frame loop can deliver the callback.
                    let (id, entry, ctx) = (
                        self.cpu.reg_get(0),
                        self.cpu.reg_get(1),
                        self.cpu.reg_get(2),
                    );
                    let (item_array, count_addr, target_addr) =
                        GuestTimer::decode_tables(&self.mem, entry);
                    self.timer = Some(GuestTimer {
                        id,
                        entry,
                        ctx,
                        item_array,
                        count_addr,
                        target_addr,
                    });
                    debug!(
                        "[plat] timer registered id={id} cb={entry:#010x} ctx={ctx:#010x}                          items={item_array:#010x} count={count_addr:#010x} target={target_addr:#010x}"
                    );
                }
            }
            // Record non-sequential transfers for fault diagnostics.
            if executed > 0 && aligned != self.last_pc.wrapping_add(4) {
                if self.branch_log.len() >= 96 {
                    self.branch_log.remove(0);
                }
                self.branch_log.push((self.last_pc, aligned));
            }
            self.last_pc = aligned;
            if !self.cpu.step_once(&mut self.mem) {
                debug!("guest fault at pc {aligned:#010x}");
                for (from, to) in &self.branch_log {
                    debug!("  branch {from:#010x} -> {to:#010x}");
                }
                return Err(MachineError::GuestFault(aligned));
            }
            if !self.mem.watch.is_empty() {
                if let Some(wa) = self.mem.watch_hit.take() {
                    debug!(
                        "[watch] write to {wa:#010x} at pc={aligned:#010x} lr={:#010x}",
                        self.cpu.reg_get(armv4t_emu::reg::LR)
                    );
                }
            }
            executed += 1;
        }
        Ok(executed)
    }

    /// Executes one hooked platform slot natively and resumes at `ret`.
    fn exec_slot(&mut self, slot: u32, ret: u32) -> Result<(), MachineError> {
        let r = |i: u8| self.cpu.reg_get(i);
        if std::env::var_os("SF_SLOTLOG").is_some() {
            debug!(
                "[slot] +{slot:#x} r0={:#x} r1={:#x} r2={:#x} r3={:#x} lr={:#010x}",
                r(0),
                r(1),
                r(2),
                r(3),
                self.cpu.reg_get(armv4t_emu::reg::LR)
            );
        }
        let result = match slot {
            // +0x0C is the screen-height query (returns 320 through the
            // planted value stub). Serving it as a heap allocator made the
            // guests' canvas geometry globals hold heap pointers, which
            // collapsed every dialog fill to a degenerate rectangle.
            0x7C => {
                let msg = if r(0) > 0x1000 {
                    guest_read_string(&self.mem, r(0), 128)?
                } else {
                    format!("fmt@0x{:X}", r(0))
                };
                if self.trace {
                    info!(
                        "[plat] +0x7C logfmt=\"{msg}\" lr={:#010x} -> 0",
                        self.cpu.reg_get(armv4t_emu::reg::LR)
                    );
                } else {
                    debug!("[plat] +0x7C logfmt=\"{msg}\" -> 0");
                }
                Some(0u32)
            }
            0x58 => Some(self.hook_fopen(r(0), r(1))?),
            0xE4 => {
                // socket write(handle, buffer, length): the SDK compares the
                // result against -1 for errors and otherwise treats it as the
                // number of bytes actually sent. Answering zero claims that
                // the whole request is still queued.
                debug!(
                    "[plat] +0xE4 write h={:#x} buf={:#010x} len={:#x} lr={:#010x}",
                    r(0),
                    r(1),
                    r(2),
                    self.cpu.reg_get(armv4t_emu::reg::LR)
                );
                if self.http.enabled {
                    let mut bytes = vec![0u8; (r(2) as usize).min(400)];
                    if self.mem.read(r(1), &mut bytes).is_ok() {
                        let text: String = bytes
                            .iter()
                            .map(|&b| {
                                if (0x20..0x7f).contains(&b) {
                                    b as char
                                } else {
                                    '.'
                                }
                            })
                            .collect();
                        debug!("[http] request: {text}");
                        let hex: String = bytes
                            .iter()
                            .map(|b| format!("{b:02x}"))
                            .collect::<Vec<_>>()
                            .join("");
                        debug!("[http] request hex: {hex}");
                        // A request names the resource pack it wants; the
                        // pack table answers with the bytes the publisher's
                        // container carries for that exact name.
                        self.http.pack = self.pack_for_request(&text);
                        if let Some(index) = self.http.pack {
                            debug!(
                                "[http] request matches resource pack {:?} ({} bytes), will serve it",
                                self.res_packs[index].name,
                                self.res_packs[index].bytes.len()
                            );
                            self.http.serve_asset = true;
                        } else if !self.http.asset.is_empty()
                            && !self.http.asset_match.is_empty()
                            && text.contains(&self.http.asset_match)
                        {
                            debug!(
                                "[http] request matches asset filter {:?}, will serve side-loaded bytes",
                                self.http.asset_match
                            );
                            self.http.serve_asset = true;
                        }
                    }
                }
                Some(r(2))
            }
            // --- offline HTTP responder ---------------------------------
            // +0xC8 select(host, event record, handler, object) is a
            // synchronous notifier: a zero return makes the SDK call the
            // handler immediately. The handler reads a signed halfword at
            // the start of the event record, where 1 means "connected".
            0xC8 => {
                let ev = r(1);
                if ev > 0x1000 {
                    self.mem.write_u16(ev, 1)?;
                }
                debug!(
                    "[plat] +0xC8 select host={:#x} ev={ev:#010x} handler={:#010x} obj={:#010x}",
                    r(0),
                    r(2),
                    r(3)
                );
                Some(0)
            }
            // connect / setup / state queries: a connected socket with no
            // error is enough for the synchronous SDK request path.
            0xCC | 0xD4 | 0xD8 | 0xDC | 0xF0 | 0x568 => Some(0),
            // +0xE8(handle, socket object) registers the object with the
            // async notifier; +0xEC(handle) looks it back up. The generic
            // stub returned zero here, which made the event dispatcher
            // silently drop every asynchronous socket event.
            0xE8 => {
                self.http.attached = Some((r(0), r(1)));
                debug!("[plat] +0xE8 attach h={:#x} obj={:#010x}", r(0), r(1));
                Some(0)
            }
            0xEC => {
                let obj = self
                    .http
                    .attached
                    .filter(|(h, _)| *h == r(0))
                    .map(|(_, o)| o)
                    .unwrap_or(0);
                debug!("[plat] +0xEC lookup h={:#x} -> {obj:#010x}", r(0));
                Some(obj)
            }
            // +0x570 receives a three-word async request descriptor:
            // [status (-1 pending), callback, context]. A real platform's
            // network thread writes the status and calls the callback once
            // the transaction completes; no guest code does it for us.
            // The SDK reuses the same descriptor for each new request, so a
            // registration after a consumed transaction starts a fresh one.
            0x570 => {
                if self.http.transaction_done() {
                    self.http.response.clear();
                    self.http.pos = 0;
                    self.http.notified = false;
                    self.http.serve_asset = false;
                    self.http.pack = None;
                }
                self.http.request = Some(r(0));
                debug!(
                    "[plat] +0x570 req={:#010x} status={:#x} cb={:#010x} arg={:#x}",
                    r(0),
                    self.mem.read_u32(r(0)).unwrap_or(0),
                    self.mem.read_u32(r(0) + 4).unwrap_or(0),
                    self.mem.read_u32(r(0) + 8).unwrap_or(0)
                );
                Some(0)
            }
            // +0xE0 is recv(handle, buffer, length). The SDK reads the
            // response right after writing the request, then closes the
            // HTTP/1.0 connection; a zero-length second read is the EOF.
            0xE0 => {
                if self.http.response.is_empty() {
                    self.http.build_response(&self.res_packs);
                }
                let buf = r(1);
                let want = r(2) as usize;
                let left = self.http.response.len().saturating_sub(self.http.pos);
                let n = want.min(left);
                if n > 0 && buf > 0x1000 {
                    self.mem
                        .write(buf, &self.http.response[self.http.pos..self.http.pos + n])?;
                }
                self.http.pos += n;
                debug!(
                    "[plat] +0xE0 recv buf={buf:#010x} want={want:#x} -> {n} (pos={}/{})",
                    self.http.pos,
                    self.http.response.len()
                );
                Some(n as u32)
            }
            0x48 => Some(self.hook_sms_fetch(r(0), r(1))?),
            0x524 => {
                // Service lookup with two observed call shapes:
                //
                // * `service(type, id, *out)` with a small numeric type —
                //   the guests then call `(*out)->vtable[0x3C]` without
                //   checking the result, so an unwritten out-pointer becomes
                //   a jump to 0 (mota's boot). Hand back the synthetic
                //   object so the call is answered.
                // * A caller passing register garbage as the type (sanguo
                //   reaches this slot with hashed leftovers) walks whatever
                //   lands in `*out` as a long table, so the object is worse
                //   than nothing there. Keep the historical behaviour —
                //   record the out-pointer and report success.
                //
                // The SIM lookup takes a third shape: the type/id pair is
                // the SDK's own hash of the "subscriber identity" service;
                // a zero return means "found". Some SDK revisions call the
                // returned value directly, others load `value[0x50]` as a
                // virtual method, so the entry must be both executable and
                // indexable (see SERVICE_CALLABLE_ADDR). Reporting an error
                // here makes the SDK log `GETICCID err` and drop to the
                // registration page, so free play answers it with that
                // dual-shape trampoline.
                let out = r(2);
                if r(0) <= 0x100 && out > 0x1000 {
                    self.mem.write_u32(out, plat::SERVICE_OBJ_ADDR)?;
                    debug!(
                        "[plat] +0x524 service type={:#x} id={:#x} out={out:#010x} -> {:#010x}",
                        r(0),
                        r(1),
                        plat::SERVICE_OBJ_ADDR
                    );
                    Some(1u32)
                } else {
                    self.mem.write_u32(plat::STATE_PTR_ADDR, out)?;
                    if self.free_play && out > 0x1000 {
                        self.mem.write_u32(out, plat::SERVICE_CALLABLE_ADDR)?;
                        debug!(
                            "[plat] +0x524 sim lookup type={:#x} id={:#x} out={out:#010x} -> callable entry",
                            r(0),
                            r(1)
                        );
                        Some(0u32)
                    } else {
                        debug!(
                            "[plat] +0x524 untargeted type={:#x} id={:#x} out={out:#010x}",
                            r(0),
                            r(1)
                        );
                        Some(1u32)
                    }
                }
            }
            0x4C => {
                let lr = self.cpu.reg_get(armv4t_emu::reg::LR);
                debug!("[plat] +0x4C lr={lr:#010x}");
                // "Can this platform bill over the carrier channel?" — the
                // SDK's pay service asks once at boot and every title gates
                // its own state machine on the answer:
                //
                // * several titles park in a "no service" state when the
                //   answer is zero (五子棋 boots straight to its pay page
                //   instead of the publisher splash, 新星际撞球 pins its
                //   `iGameState` at 102 and never reaches 2, the play state);
                // * the pay service itself returns early when it reads zero,
                //   skipping its whole SMS/purchase pass.
                //
                // The platform is a phone with a SIM, so the honest answer is
                // a non-zero one.  SF_4C="lr:val;lr:val;*:val" (hex) overrides
                // it per call site for experiments.
                let mut base = 1u32;
                let mut result = None;
                if let Ok(policy) = std::env::var("SF_4C") {
                    // "*" sets the base value; a matching `lr:val` entry
                    // wins regardless of its order in the list.
                    let lr_low = lr & 0xFFFFFF;
                    for pair in policy.split(';') {
                        let (a, v) = pair.split_once(':').unwrap_or(("*", "0"));
                        let val = u32::from_str_radix(v, 16).unwrap_or(0);
                        match u32::from_str_radix(a, 16) {
                            Ok(site) => {
                                if site == lr_low {
                                    result = Some(val);
                                }
                            }
                            Err(_) if a == "*" => base = val,
                            _ => {}
                        }
                    }
                }
                Some(result.unwrap_or(base))
            }
            0x5C => Some(self.hook_fread(r(0), r(1), r(2))?),
            0x60 => Some(self.hook_fwrite(r(0), r(1), r(2))?),
            0x64 => Some(self.hook_fseek(r(0), r(1), r(2))?),
            0x68 => {
                let h = r(0);
                if (1..=4).contains(&h) && self.file_slots[(h - 1) as usize].in_use {
                    let slot = &self.file_slots[(h - 1) as usize];
                    debug!(
                        "[plat] +0x68 fclose h={h} path=\"{}\" (pos={}/{})",
                        slot.name, slot.offset, slot.size
                    );
                    let s = &mut self.file_slots[(h - 1) as usize];
                    *s = FileSlot::default();
                } else {
                    debug!("[plat] +0x68 fclose h={h} -> -1 (bad handle)");
                }
                Some(u32::MAX) // fclose always returns -1
            }
            0x108 => Some(self.hook_lz(r(0), r(1), r(2), r(3))?),
            0xAC => {
                // arm(context, period_ms, callback) -> handle
                //
                // Every title in the corpus arms this repeating UI timer
                // during create, then leans on it for all of its per-frame
                // work: the SDK's callback re-enters the app handler with
                // selector 10 and a message id of its own choosing (3 for
                // the framework builds, 1 for the older ones). Leaving the
                // slot unanswered keeps the app's screen stack empty, so
                // the titles paint a bare background and stop.
                //
                // Not every call is a callback arm — some guests pass a
                // message id instead of a code pointer — so anything that
                // does not land in the mapped code range is answered with a
                // handle but never pumped.
                let (ctx, period, cb) = (r(0), r(1).max(1), r(2));
                if cb < self.code_base || cb >= self.image_end {
                    debug!(
                        "[plat] +0xAC arm ctx={ctx:#010x} period={period}ms target={cb:#010x} (not code, not pumped)"
                    );
                    Some(0)
                } else {
                    if self.ui_timers.len() >= 8 {
                        self.ui_timers.remove(0);
                    }
                    let handle = self.ui_timers.len() as u32 + 1;
                    let record = self.find_message_record(period).unwrap_or(0);
                    debug!("[plat] +0xAC record={record:#010x}");
                    self.ui_timers.push(UiTimer {
                        period_ms: period,
                        callback: cb,
                        context: if record != 0 { record } else { ctx },
                        due_ms: self.ui_clock_ms + period as u64,
                    });
                    debug!(
                        "[plat] +0xAC arm ctx={ctx:#010x} period={period}ms cb={cb:#010x} -> handle {handle}"
                    );
                    Some(handle)
                }
            }
            0x12C | 0x130 | 0x134 | 0x140 | 0x14C | 0x154 | 0x158 | 0x180 | 0x198 | 0x194
            | 0x184 | 0x15C | 0x010 => {
                // GDI slots leave r0 untouched (the reference writes PC only).
                self.exec_gdi(slot, ret)?;
                None
            }
            _ => {
                warn!("unhandled hooked slot +0x{slot:X} at ret {ret:#010x}");
                Some(0)
            }
        };
        if let Some(v) = result {
            self.cpu.reg_set(0, v);
        }
        self.cpu.reg_set(armv4t_emu::reg::PC, ret); // wrapper epilogue
        Ok(())
    }

    /// Wrapper stack adjustment (`add sp,#imm` at the epilogue).
    fn sp_adjust(&self, ret: u32) -> u32 {
        if let Ok(word) = self.mem.read_u32(ret) {
            if word & 0xFFFF_F000 == 0xE28D_D000 {
                return word & 0xFFF;
            }
        }
        0
    }

    /// Original caller-pushed stack argument `n` (1 = the 5th argument).
    fn stack_arg(&self, sp: u32, adj: u32, n: u32) -> Result<u32, MachineError> {
        Ok(self
            .mem
            .read_u32(sp.wrapping_add(adj).wrapping_add(4 * n))?)
    }

    /// The current pen colour of a GDI call: the canvas object carries a
    /// packed RGB565 word at +0x2C and the RGB888 triple set by slot +0x180
    /// at +0x28. Zero in the packed word means "unset", so the triple wins;
    /// when neither is present the last colour pushed through +0x180 is used.
    fn pen_color(&self, obj: u32) -> u16 {
        if obj > 0x1000 {
            if let Ok(c16) = self.mem.read_u16(obj + 0x2C) {
                if c16 != 0 {
                    return c16;
                }
            }
            let mut b3 = [0u8; 3];
            if self.mem.read(obj + 0x28, &mut b3).is_ok() {
                let v = (b3[0] as u32) << 16 | (b3[1] as u32) << 8 | b3[2] as u32;
                if v != 0 {
                    return crate::gdi::rgb888_to_565(v);
                }
            }
        }
        self.gdi.fill_color()
    }

    /// Executes one hooked GDI slot. Stack args follow the reference's
    /// `SP + adj + 4*n` layout; r0 stays untouched.
    fn exec_gdi(&mut self, slot: u32, ret: u32) -> Result<(), MachineError> {
        let sp = self.cpu.reg_get(armv4t_emu::reg::SP);
        let r = |i: u8| self.cpu.reg_get(i);
        match slot {
            0x12C => {
                // update(obj, x, y, w, h) — a dirty-rectangle commit, not a
                // fill. The guest paints into the surface and then asks the
                // platform to push that rectangle to the display; the second
                // stack word is *not* a colour (every title in the corpus
                // leaves stale stack contents there — code pointers, string
                // pointers, zero), so reading it as RGB565 painted random
                // colours over the frame. Our surface *is* the display, so
                // presenting the rectangle costs nothing.
                let adj = self.sp_adjust(ret);
                let x = r(1) as i32;
                let y = r(2) as i32;
                let w = r(3) as i32;
                let h = self.stack_arg(sp, adj, 1)? as i32;
                // A full-screen commit is the guest's frame boundary: the
                // label cursor is per-frame state (some titles commit with
                // this slot instead of the flush slot).
                if x == 0 && y == 0 && w == plat::FB_WIDTH as i32 && h == plat::FB_HEIGHT as i32 {
                    self.gdi.reset_text_cursor();
                }
                if self.trace {
                    let stale = self.stack_arg(sp, adj, 2).unwrap_or(0);
                    debug!(
                        "[gdi]+0x{slot:X} update rect {x},{y} {w}x{h} obj={:#010x} stale={stale:#010x} lr={:#010x}",
                        r(0),
                        self.cpu.reg_get(armv4t_emu::reg::LR)
                    );
                }
            }
            0x154 => {
                // text(obj, str u16le, x, y, font) — the font selector is
                // ignored (single embedded font set).
                //
                // The anchor is the raw (x, y) — the reference reads r2/r3
                // and only slides a label back below the bottom edge. The
                // SDK's dialog renderer issues one call per line with the
                // *same* origin and relies on the platform's label cursor:
                // a call repeating the previous origin draws on the next
                // row, so its four-line dialog bodies stack instead of
                // overprinting.
                let str_ptr = r(1);
                let x = r(2) as i32;
                let y = r(3) as i32;
                let mut chars = Vec::with_capacity(64);
                for i in 0..256u32 {
                    let v = self.mem.read_u16(str_ptr.wrapping_add(i * 2))?;
                    chars.push(v);
                    if v == 0 {
                        break;
                    }
                }
                // The pen color lives in the canvas object at +0x28 (the
                // slot +0x180 store). Zero is a real color — black — so the
                // field is used as-is; the white default only covers a
                // missing object where the load fails.
                let mut rgb24: u32 = 0xFFFFFF;
                if r(0) > 0x1000 {
                    let mut b3 = [0u8; 3];
                    if self.mem.read(r(0) + 0x28, &mut b3).is_ok() {
                        rgb24 = (b3[0] as u32) << 16 | (b3[1] as u32) << 8 | b3[2] as u32;
                    }
                }
                let color = crate::gdi::rgb888_to_565(rgb24);
                // Labels anchored past the surface edges: the bottom softkey
                // row sits at `height + n` and the right-hand label at
                // `width - 5`. The platform slides such a label back inside
                // instead of clipping it away, which is what makes the last
                // row and the right-hand label readable. A label wider than
                // the surface keeps its origin (the run clips at the edge
                // like the reference) unless the origin itself is past the
                // edge.
                let (tw, th) = crate::gdi::Gdi::text_extent(&chars);
                let x = if x + tw > plat::FB_WIDTH as i32 {
                    (plat::FB_WIDTH as i32 - tw).max(0)
                } else {
                    x
                };
                let y = if th > 0 && y + th > plat::FB_HEIGHT as i32 {
                    (plat::FB_HEIGHT as i32 - th).max(0)
                } else {
                    y
                };
                // The label cursor: a repeated origin continues the block
                // one row down; anything else restarts at its own origin.
                let (dx, dy) = match self.gdi.text_cursor {
                    Some((ox, oy, next_y)) if ox == x && oy == y => (x, next_y),
                    _ => (x, y),
                };
                // One call advances the cursor by one row; the row height is
                // the tallest cell in the label (8 ASCII / 16 wide).
                self.gdi
                    .op_text_block(&mut self.mem, &chars, dx, dy, color, None);
                self.gdi.text_cursor = Some((x, y, dy + th.max(8)));
                if self.trace {
                    let text: String = chars
                        .iter()
                        .take_while(|&&c| c != 0)
                        .filter_map(|&c| char::from_u32(c as u32))
                        .collect();
                    let adj = self.sp_adjust(ret);
                    debug!(
                        "[gdi]+0x{slot:X} text raw=({},{}) @{x},{y}->{dx},{dy} obj={:#010x} c={color:#06x} len={} font={:#x} \"{text}\"",
                        r(2),
                        r(3),
                        r(0),
                        chars.len(),
                        self.stack_arg(sp, adj, 1).unwrap_or(0)
                    );
                }
            }
            0x14C => {
                // textrun(obj, str u16le, first, count, x, y, flags) —
                // the paragraph body renderer behind every dialog.
                //
                // The guest hands the platform a character range of the
                // UTF-16 string together with the block origin it laid out.
                // The range is frequently the whole paragraph (the intro
                // screens scroll it by moving y between frames), so the
                // firmware breaks the lines itself at the surface edge; the
                // line height follows the same cell metrics the guest
                // measures with (8 ASCII / 16 wide). Without the wrapping
                // the body renders as one clipped line, which is what every
                // intro and battle dialog looked like before.
                let adj = self.sp_adjust(ret);
                let str_ptr = r(1);
                let first = r(2);
                let count = r(3);
                let x = self.stack_arg(sp, adj, 1)? as i32;
                let y = self.stack_arg(sp, adj, 2)? as i32;
                let mut chars = Vec::with_capacity(count.min(512) as usize);
                for i in 0..count.min(512) {
                    let v = self
                        .mem
                        .read_u16(str_ptr.wrapping_add((first + i).wrapping_mul(2)))?;
                    if v == 0 {
                        break;
                    }
                    chars.push(v);
                }
                let color = self.pen_color(r(0));
                if self.trace {
                    let text: String = chars
                        .iter()
                        .filter_map(|&c| char::from_u32(c as u32))
                        .collect();
                    debug!(
                        "[gdi]+0x{slot:X} textrun @{x},{y} n={count} s1={:#x} s2={:#x} s3={:#x} s4={:#x} c={color:#06x} \"{text}\"",
                        self.stack_arg(sp, adj, 1).unwrap_or(0),
                        self.stack_arg(sp, adj, 2).unwrap_or(0),
                        self.stack_arg(sp, adj, 3).unwrap_or(0),
                        self.stack_arg(sp, adj, 4).unwrap_or(0)
                    );
                }
                let right = plat::FB_WIDTH as i32;
                self.gdi
                    .op_text_block(&mut self.mem, &chars, x, y, color, Some(right));
            }
            0x198 => {
                // draw(obj, bmp, w, h, _, dx, dy)
                let adj = self.sp_adjust(ret);
                let bmp_ptr = r(1);
                let w = self.stack_arg(sp, adj, 1)? as i32;
                let h = self.stack_arg(sp, adj, 2)? as i32;
                let dx = self.stack_arg(sp, adj, 4)? as i32;
                let dy = self.stack_arg(sp, adj, 5)? as i32;
                self.draw_bitmap(slot, bmp_ptr, Some((w, h)), dx, dy)?;
            }
            0x134 => {
                // blit(obj, bmp, x, y, flags) — the five-argument form of
                // +0x198: the bitmap is drawn at its own size, so the
                // descriptor supplies the clip window.
                let bmp_ptr = r(1);
                let dx = r(2) as i32;
                let dy = r(3) as i32;
                self.draw_bitmap(slot, bmp_ptr, None, dx, dy)?;
            }
            0x194 => {
                self.gdi.op_bmp();
            }
            0x184 => {
                // clear(obj, r, g, b) — records the pen color only.
                let (rgb24, rgb565) = self.gdi.op_clear(r(1), r(2), r(3));
                if r(0) > 0x1000 {
                    let bytes = rgb24.to_le_bytes();
                    self.mem.write(r(0) + 0x28, &bytes[..3])?;
                    self.mem.write_u16(r(0) + 0x2C, rgb565)?;
                }
            }
            0x180 => {
                // set-color(obj, rgb888) — records the pen/text color (the
                // values seen are RGB grays/whites; the text reads at that
                // address fault, so r1 is the color, not a pointer).
                let rgb24 = r(1) & 0xFFFFFF;
                let (rgb24, rgb565) =
                    self.gdi
                        .op_clear((rgb24 >> 16) & 0xFF, (rgb24 >> 8) & 0xFF, rgb24 & 0xFF);
                if self.trace {
                    debug!("[gdi]+0x180 set-color rgb24={rgb24:#08x} rgb565={rgb565:#06x}");
                }
                if r(0) > 0x1000 {
                    let bytes = rgb24.to_le_bytes();
                    let _ = self.mem.write(r(0) + 0x28, &bytes[..3]);
                    let _ = self.mem.write_u16(r(0) + 0x2C, rgb565);
                }
            }
            0x15C => {
                // fill2(obj, x, y, w, h) — color from the canvas object.
                let adj = self.sp_adjust(ret);
                let x = r(1) as i32;
                let y = r(2) as i32;
                let w = r(3) as i32;
                let h = self.stack_arg(sp, adj, 1)? as i32;
                let color = self.pen_color(r(0));
                if self.trace {
                    let stack: Vec<u32> = (0..4u32)
                        .filter_map(|i| self.mem.read_u32(sp.wrapping_add(i * 4)).ok())
                        .collect();
                    debug!(
                        "[gdi]+0x{slot:X} fill2 @{x},{y} {w}x{h} c={color:#06x} lr={:#010x} sp={sp:#010x} adj={adj} stk={stack:x?}",
                        self.cpu.reg_get(armv4t_emu::reg::LR)
                    );
                }
                self.gdi.op_fill2(&mut self.mem, x, y, w, h, color);
            }
            0x010 => {
                self.gdi.op_flush(&self.mem);
                // The label cursor is per-frame state: the next frame's
                // dialog re-issues its first line at the block origin.
                self.gdi.reset_text_cursor();
            }
            // +0x130/+0x140: the long-form fill variants for the dialog
            // chrome (r1 = x, r2 = y; the canvas color from the context).
            0x130 | 0x140 => {
                let x = r(1) as i32;
                let y = r(2) as i32;
                let color = self.gdi.fill_color();
                if self.trace {
                    debug!("[gdi]+0x{slot:X} fill @({x},{y}) c={color:#06x}");
                }
                self.gdi.op_fill2(&mut self.mem, x, y, 240, 8, color);
            }
            _ => {
                if self.trace {
                    debug!(
                        "[gdi]+0x{slot:X} unhandled r0={:#010x} r1={:#010x} r2={:#010x} r3={:#010x} lr={:#010x}",
                        r(0),
                        r(1),
                        r(2),
                        r(3),
                        self.cpu.reg_get(armv4t_emu::reg::LR)
                    );
                }
            }
        }
        Ok(())
    }

    /// Blits a bitmap descriptor (`sw, sh, key, pal, data`) onto the canvas.
    /// `clip` is the `(w, h)` window the caller asked for; `None` keeps the
    /// bitmap's own size, which is what the five-argument `+0x134` form does.
    fn draw_bitmap(
        &mut self,
        slot: u32,
        bmp_ptr: u32,
        clip: Option<(i32, i32)>,
        dx: i32,
        dy: i32,
    ) -> Result<(), MachineError> {
        if bmp_ptr <= 0x1000 {
            return Ok(());
        }
        let mut hdr = [0u8; 0x14];
        if self.mem.read(bmp_ptr, &mut hdr).is_err() {
            return Ok(());
        }
        let sw = u32::from_le_bytes([hdr[0], hdr[1], hdr[2], hdr[3]]) as i32;
        let sh = u32::from_le_bytes([hdr[4], hdr[5], hdr[6], hdr[7]]) as i32;
        let key = u32::from_le_bytes([hdr[8], hdr[9], hdr[10], hdr[11]]);
        let pal = u32::from_le_bytes([hdr[12], hdr[13], hdr[14], hdr[15]]);
        let data = u32::from_le_bytes([hdr[16], hdr[17], hdr[18], hdr[19]]);
        let (w, h) = clip.unwrap_or((sw, sh));
        if self.trace {
            debug!(
                "[gdi]+0x{slot:X} blit obj={:#010x} bmp@{bmp_ptr:#x} sw={sw} sh={sh} key={key:#x} pal={pal:#x} data={data:#x} clip={w}x{h} @({dx},{dy})",
                self.cpu.reg_get(0)
            );
        }
        if w <= 0 || h <= 0 || w > 0x400 || h > 0x400 {
            return Ok(());
        }
        let data = if data > 0x1000 && data <= 0x7F_FFFF {
            data
        } else {
            self.mem.read_u32(bmp_ptr.wrapping_add(0x14))?
        };
        let pal_mode = pal > 0x1000 && pal <= 0x7F_FFFF;
        let nbytes = if pal_mode {
            (sw * sh) as usize
        } else {
            (sw * sh * 2) as usize
        };
        if sw <= 0 || sh <= 0 || nbytes == 0 || nbytes > 0x80000 {
            return Ok(());
        }
        let mut buf = vec![0u8; nbytes];
        if self.mem.read(data, &mut buf).is_err() {
            return Ok(());
        }
        if self.trace {
            let pal_head: Vec<u16> = (0..8u32)
                .filter_map(|i| self.mem.read_u16(pal.wrapping_add(i * 2)).ok())
                .collect();
            debug!(
                "[gdi] blit input pal8={pal_head:x?} first={:02x?}",
                &buf[..8.min(buf.len())]
            );
        }
        self.gdi.op_draw(
            &mut self.mem,
            &buf,
            sw,
            sh,
            key,
            pal_mode,
            w,
            h,
            dx,
            dy,
            pal,
        );
        Ok(())
    }

    /// `fopen(utf16_path, flags)` — refuses platform vfiles, otherwise
    /// serves container bytes (or the sibling resource file for `res`/
    /// `.pak` paths). Handles are 1..4.
    ///
    /// The account files (`__sfssuid` and `ssu<id>`) are the SDK's own
    /// registration record: it probes every drive root for them at boot and
    /// drops to the registration page when none answers. Free play backs them
    /// with the in-memory service store, so a record the guest writes is read
    /// back on the next boot instead of being refused.
    fn hook_fopen(&mut self, path_ptr: u32, flags: u32) -> Result<u32, MachineError> {
        let path = if path_ptr > 0x1000 {
            guest_utf16_to_ascii(&self.mem, path_ptr, 96)?
        } else {
            String::new()
        };
        let free = self.file_slots.iter().position(|s| !s.in_use);
        let base = basename(&path);
        let kind = vfile_kind(base);
        let account = self.free_play && matches!(kind, Some(0) | Some(1) | Some(2) | Some(4));
        if let Some(kind) = kind {
            if !account {
                debug!("[plat] +0x58 fopen \"{path}\" flags={flags} -> 0 (vfile/{kind}, refused)");
                return Ok(0);
            }
        }
        let Some(idx) = free else {
            debug!("[plat] +0x58 fopen \"{path}\" flags={flags} -> 0 (table full)");
            return Ok(0);
        };
        let lower = path.to_ascii_lowercase();
        // A resource pack the guest asks for by name: the publisher's
        // container carries that pack, so the guest is served the pack
        // itself rather than the container that holds it.
        let pack = self
            .res_packs
            .iter()
            .position(|p| p.name.eq_ignore_ascii_case(base));
        let special = !self.res_containers.is_empty()
            && !path.is_empty()
            && (lower.contains(".pak") || lower.contains("res"));
        // The SMS-billing service keeps its request/response records in
        // guest-visible files (`__`-prefixed names: __sfsms* request
        // records, __dcsmsinfo_ the response inbox, __sfgpsav saves): back
        // them all with in-memory data so the guest reads back what it
        // wrote instead of the container bytes.
        let base_lower = basename(&path).to_ascii_lowercase();
        let writable = basename(&path).starts_with("__")
            || account
            || base_lower.ends_with(".sav")
            || (base_lower.ends_with(".dat") && base_lower != "game.dat");
        // A write-intent open (flags 5 = read/write) must land in memory,
        // whatever the path looks like. The guest downloads a resource
        // package, writes it out and then re-opens it to check the size;
        // if the write is dropped the file it reads back is the wrong one
        // and the size check fails.
        let write_intent = flags & 1 != 0;
        // A file the guest already wrote is served back from memory, not
        // from the container: the guest re-opens the resource package it
        // just downloaded to check its size, and the container bytes would
        // answer with the wrong length.
        let written = !write_intent && self.named_files.get(&path).is_some_and(|d| !d.is_empty());
        if writable || write_intent || written {
            let stored = self.named_files.get(&path).cloned().unwrap_or_default();
            // The pay-info file is a tag-stream log the guest builds
            // itself; pre-creating a synthetic record breaks the stream
            // parse. Start empty and let the guest's own registration
            // flow fill it (the write hook forces the leading dword
            // positive so the gate check passes).
            // The response inbox (__dcsmsinfo_) is what the carrier would
            // write back; SF_DCSMS injects its bytes so the send handler's
            // parse-and-compare gate can pass offline. The injection
            // happens once per path: after the guest consumes the record
            // the inbox stays empty so the no-match path (which marks the
            // flow processed) can run instead of an infinite match cycle.
            let mut stored = stored;
            if stored.is_empty()
                && basename(&path).starts_with("__dcsmsinfo")
                && !self.dcsms_injected.contains(&path)
            {
                if let Ok(hex) = std::env::var("SF_DCSMS") {
                    let bytes: Vec<u8> = hex
                        .split(',')
                        .filter_map(|b| u8::from_str_radix(b, 16).ok())
                        .collect();
                    debug!("[plat] injecting {} response bytes for {path}", bytes.len());
                    stored = bytes;
                    self.dcsms_injected.insert(path.clone());
                }
            }
            let size = stored.len();
            // flags bit 0 = write intent: start an empty record; otherwise
            // serve the previously written contents for reading.
            let data = if write_intent { Vec::new() } else { stored };
            debug!(
                "[plat] +0x58 fopen \"{path}\" flags={flags} -> h={} (writable, {} bytes; store={})",
                idx + 1,
                data.len(),
                self.named_files.get(&path).map(|d| d.len()).unwrap_or(0)
            );
            self.file_slots[idx] = FileSlot {
                in_use: true,
                offset: 0,
                size,
                name: path.clone(),
                data,
                backing: SlotBacking::Memory,
            };
            return Ok(idx as u32 + 1);
        }
        // A path the platform does not know is the guest's own file: the
        // save slot it probes on every drive root before it starts a new
        // game. The container bytes are *not* that file's content, and
        // serving them makes the guest validate the package image as a
        // save record and report "invalid save" (无效存档). Such a file
        // starts empty and is filled in by the guest's own writes.
        let (backing, size) = match pack {
            Some(index) => (SlotBacking::Pack(index), self.res_packs[index].bytes.len()),
            None if special => (SlotBacking::Res, self.res_containers[0].len()),
            None => (SlotBacking::Memory, 0),
        };
        self.file_slots[idx] = FileSlot {
            in_use: true,
            offset: 0,
            size,
            name: if path.is_empty() {
                "(nil)".to_string()
            } else {
                path.clone()
            },
            data: Vec::new(),
            backing,
        };
        debug!(
            "[plat] +0x58 fopen \"{path}\" flags={flags} -> h={} (size={size}{})",
            idx + 1,
            if backing == SlotBacking::Res {
                " RES"
            } else {
                ""
            }
        );
        Ok(idx as u32 + 1)
    }

    /// `fread(handle, guest_buf, len)` — copies container bytes.
    /// `+0x48 fetch_sms_info(buffer)` — the SDK's SMS pay-record fetch.
    /// The poll phase checks the record's leading dword for the 200-299
    /// port range and the dword at +0x10 against the confirmation-code
    /// pattern; the offline shim fabricates both.
    fn hook_sms_fetch(&mut self, buf: u32, kind: u32) -> Result<u32, MachineError> {
        // kind 1 = the poll query (buffer, 5 words, type 1); the boot-time
        // info query passes other registers and keeps passthrough
        // semantics so its flow stays untouched.
        if std::env::var_os("SF_FAKE_PAY").is_some()
            && kind == 1
            && buf > 0x1000
            && std::env::var_os("SF_NO_SMS").is_none()
        {
            let rec: [u32; 5] = [0xC8, 0, 0, 0, 0x0002_EAB5];
            let mut bytes = [0u8; 20];
            for (k, v) in rec.iter().enumerate() {
                bytes[k * 4..k * 4 + 4].copy_from_slice(&v.to_le_bytes());
            }
            self.mem.write(buf, &bytes)?;
            debug!("[plat] +0x48 sms fetch -> fabricated record at {buf:#010x}");
        }
        // A zero return routes the caller into its pay-processor callback
        // (the indirect call through the API table), which is what advances
        // the flow; the buffer-pointer return skips it.
        Ok(0)
    }

    /// `fwrite(handle, buf, len)` (slot +0x60) — appends to a writable
    /// service file. The SMS-billing service keeps its request and response
    /// records in these files; the host logs the bytes to make the record
    /// format observable.
    fn hook_fwrite(&mut self, handle: u32, buf: u32, len: u32) -> Result<u32, MachineError> {
        if !(1..=4).contains(&handle) || !self.file_slots[(handle - 1) as usize].in_use {
            debug!("[plat] +0x60 fwrite h={handle} buf={buf:#x} size={len} -> 0 (bad handle)");
            return Ok(0);
        }
        let idx = (handle - 1) as usize;
        let mut chunk = vec![0u8; len as usize];
        if buf > 0x1000 && len > 0 {
            self.mem.read(buf, &mut chunk)?;
        }
        let name = self.file_slots[idx].name.clone();
        debug!(
            "[plat] +0x60 fwrite h={handle} \"{name}\" {} bytes lr={:#010x}: {:02x?}",
            len,
            self.cpu.reg_get(armv4t_emu::reg::LR),
            chunk
        );
        // A write always lands in the in-memory file, whatever the slot's
        // backing: the guest writes the file and then reads it back.
        if self.file_slots[idx].backing == SlotBacking::Memory
            || service_file(&self.file_slots[idx].name)
        {
            let (name, offset) = {
                let slot = &self.file_slots[idx];
                (slot.name.clone(), slot.offset)
            };
            let entry = self
                .named_files
                .entry(name.clone())
                .or_insert_with(|| self.file_slots[idx].data.clone());
            let at = offset.min(entry.len());
            let end = (at + chunk.len()).max(entry.len());
            entry.resize(end, 0);
            entry[at..at + chunk.len()].copy_from_slice(&chunk);
            // Offline billing: the pay-info record's leading big-endian
            // dword is the service status the SDK's gate checks (negative
            // = waiting for the carrier). Force it positive so the
            // confirmation path runs.
            if basename(&name).starts_with("__sfsmspayinfo") && entry.len() >= 4 {
                entry[0..4].copy_from_slice(&1u32.to_be_bytes());
            }
            let slot = &mut self.file_slots[idx];
            slot.data = entry.clone();
            slot.offset = at + chunk.len();
            slot.size = slot.data.len();
        }
        Ok(len)
    }

    fn hook_fread(&mut self, handle: u32, buf: u32, len: u32) -> Result<u32, MachineError> {
        if !(1..=4).contains(&handle) || !self.file_slots[(handle - 1) as usize].in_use {
            debug!("[plat] +0x5C fread h={handle} buf={buf:#x} size={len} -> 0 (bad handle)");
            return Ok(0);
        }
        let idx = (handle - 1) as usize;
        if self.file_slots[idx].backing == SlotBacking::Memory {
            let slot = &self.file_slots[idx];
            let avail = slot.data.len().saturating_sub(slot.offset);
            let n = (len as usize).min(avail);
            if n > 0 && buf > 0x1000 {
                let chunk = slot.data[slot.offset..slot.offset + n].to_vec();
                self.mem.write(buf, &chunk)?;
            }
            self.file_slots[idx].offset += n;
            debug!("[plat] +0x5C fread h={handle} -> {n} (writable file)");
            return Ok(n as u32);
        }
        let avail = self.file_slots[idx]
            .size
            .saturating_sub(self.file_slots[idx].offset);
        let n = (len as usize).min(avail);
        if n > 0 && buf > 0x1000 {
            let src_off = self.file_slots[idx].offset;
            let backing = self.file_slots[idx].backing;
            let data: &[u8] = match backing {
                SlotBacking::Pack(index) => &self.res_packs[index].bytes[src_off..src_off + n],
                SlotBacking::Res => &self.res_containers[0][src_off..src_off + n],
                _ => &self.container[src_off..src_off + n],
            };
            self.mem.write(buf, data)?;
        }
        self.file_slots[idx].offset += n;
        debug!(
            "[plat] +0x5C fread h={handle} buf={buf:#x} size={len} -> {n} (pos={}/{}) lr={:#010x}",
            self.file_slots[idx].offset,
            self.file_slots[idx].size,
            self.cpu.reg_get(armv4t_emu::reg::LR)
        );
        Ok(n as u32)
    }

    /// `fseek(handle, offset, whence)` — 0 SET, 1 CUR, 2 END.
    fn hook_fseek(&mut self, handle: u32, offset: u32, whence: u32) -> Result<u32, MachineError> {
        if !(1..=4).contains(&handle) || !self.file_slots[(handle - 1) as usize].in_use {
            debug!("[plat] +0x64 fseek h={handle} off={offset} whence={whence} -> -1 (bad handle)");
            return Ok(u32::MAX);
        }
        let idx = (handle - 1) as usize;
        let offset = offset as i32;
        let size = self.file_slots[idx].size as i64;
        let pos = self.file_slots[idx].offset as i64;
        let new: i64 = match whence {
            0 => offset as i64,
            1 => pos + offset as i64,
            2 => size + offset as i64,
            _ => -1,
        };
        if (0..=size).contains(&new) {
            self.file_slots[idx].offset = new as usize;
            Ok(new as u32)
        } else {
            Ok(u32::MAX)
        }
    }

    /// `lz(src, src_len, dst, out_len*)` — decompresses one block into the
    /// guest heap window and writes the produced size to `*out`.
    fn hook_lz(&mut self, src: u32, src_len: u32, dst: u32, out: u32) -> Result<u32, MachineError> {
        self.lz_calls += 1;
        debug!("[plat] +0x108 lz src={src:#x} len={src_len} dst={dst:#x} out={out:#x}");
        if src_len == 0 || !(1..=0x400000).contains(&src_len) || src <= 0x1000 || dst <= 0x1000 {
            if out > 0x1000 {
                self.mem.write_u32(out, 0)?;
            }
            return Ok(u32::MAX - 6); // -7 bad args
        }
        let cap = 0x100000usize;
        let mut srcbuf = vec![0u8; src_len as usize];
        self.mem.read(src, &mut srcbuf)?;
        if std::env::var_os("SF_DUMP_BLOCK").is_some() {
            let name = format!("SF_DUMP_BLOCK_{src:08X}_{src_len}.bin");
            let _ = std::fs::write(&name, &srcbuf);
            info!("dumped {name}");
        }
        match lz::decompress(&srcbuf, cap) {
            Ok(data) => {
                let out_len = data.len();
                self.mem.write(dst, &data)?;
                if out > 0x1000 {
                    self.mem.write_u32(out, out_len as u32)?;
                }
                debug!("[plat] +0x108 lz -> ret=0 out_len={out_len}");
                Ok(0)
            }
            Err(e) => {
                warn!("[plat] +0x108 lz failed: {e}");
                if out > 0x1000 {
                    self.mem.write_u32(out, 0)?;
                }
                Ok(match e {
                    lz::LzError::InvalidToken => u32::MAX,
                    lz::LzError::OutputFull => u32::MAX - 1,
                    lz::LzError::BadOffset { .. } => u32::MAX - 5,
                    lz::LzError::SourceOverrun => u32::MAX - 3,
                    lz::LzError::SourceTruncated => u32::MAX - 7,
                    lz::LzError::BadTerminator(_) => u32::MAX,
                })
            }
        }
    }

    /// Boots the guest: init (selector -1, registers the handler), then the
    /// create (1), activate (2) and first resume (3) calls, mirroring the
    /// platform's boot sequence.
    pub fn boot(&mut self, budget: u64) -> Result<(), MachineError> {
        self.app_call(-1, 0, budget)?;
        if self.handler == 0 {
            warn!("boot completed without a registered handler");
        } else {
            info!("guest handler registered at {:#010x}", self.handler);
        }
        self.app_call(1, 0, budget)?;
        // Selector 2 = the app-activate event. The guests' dispatch cases
        // for it complete their boot state; the resume call below follows it
        // so the titles that report it as a pause get un-parked.
        self.app_call(2, 0, budget)?;
        self.app_call(3, 0, budget)?;
        Ok(())
    }

    /// Drives one guest call: init (selector -1) or an event dispatch.
    pub fn app_call(&mut self, selector: i32, param: u32, budget: u64) -> Result<(), MachineError> {
        if self.trace {
            debug!("[app] call selector={selector} param={param:#x}");
        }
        self.mem.write_u8(self.state_addr(), 2)?;
        self.cpu.reg_set(armv4t_emu::reg::SP, STACK_TOP);
        self.cpu.reg_set(armv4t_emu::reg::LR, STOP_ADDRESS);
        self.cpu.reg_set(armv4t_emu::reg::CPSR, 0x10); // user mode, ARM
        if selector == -1 || self.handler == 0 {
            // Init: entry code_base(r0=0, r1=&args slot, r2=selector, r3=0).
            self.cpu.reg_set(0, 0);
            self.cpu.reg_set(1, APP_RECORD_ADDR + 0x1C);
            self.cpu.reg_set(2, selector as u32);
            self.cpu.reg_set(3, 0);
            self.cpu.reg_set(armv4t_emu::reg::PC, self.code_base);
            self.run_until_return(budget)?;
            let registered = self.mem.read_u32(APP_RECORD_ADDR + 0x1C)?;
            if registered != 0 {
                self.handler = registered;
                self.mem.write_u32(APP_RECORD_ADDR, registered)?;
                self.mem
                    .write_u32(registered.wrapping_add(0x40), u32::MAX)?;
                let mut dump = String::new();
                for k in 0..16u32 {
                    let v = self.mem.read_u32(registered + k * 4)?;
                    dump.push_str(&format!("{:08x} ", v));
                }
                debug!("app_root block: {dump}");
            }
        } else {
            // Event: the registered value is a block pointer; the block's
            // first word is the dispatch function. Call it as
            // fn(app_root, selector, param, 0).
            let entry = self.mem.read_u32(self.handler)?;
            if entry == 0 {
                let mut dump = String::new();
                for k in 0..16u32 {
                    let v = self
                        .mem
                        .read_u32(self.handler.wrapping_add(k * 4))
                        .unwrap_or(0);
                    dump.push_str(&format!("{:08x} ", v));
                }
                debug!(
                    "event entry==0: handler={:#010x} block: {dump}",
                    self.handler
                );
            }
            self.cpu.reg_set(0, self.handler);
            self.cpu.reg_set(1, selector as u32);
            self.cpu.reg_set(2, param);
            self.cpu.reg_set(3, 0);
            self.cpu.reg_set(armv4t_emu::reg::PC, entry);
            self.run_until_return(budget)?;
        }
        self.mem.write_u8(self.state_addr(), 3)?;
        Ok(())
    }

    /// Queues a key event (index 0..31, pressed/released).
    pub fn set_key(&mut self, index: u8, pressed: bool) -> Result<(), MachineError> {
        if index > 31 {
            return Err(MachineError::BadKey(index));
        }
        let code = KEY_MAP[index as usize];
        if code != 0 {
            self.events.post(if pressed { 6 } else { 5 }, code as u32);
        }
        Ok(())
    }

    /// Pumps all queued events into the guest.
    pub fn pump(&mut self, budget: u64) -> Result<(), MachineError> {
        while let Some(ev) = self.events.pop() {
            self.app_call(ev.selector, ev.param, budget)?;
        }
        Ok(())
    }

    /// Locates the SDK's delayed-message record for `period`.
    ///
    /// Slot +0xAC only carries `(owner, period, callback)`, but the shared
    /// SDK's callback is a trampoline that dereferences r0 as a record of
    /// `(app block, message id, delay, flag)` and re-dispatches the app
    /// handler with `(selector 10, message id, 0)`. The record lives in the
    /// image's data section and is written before the arm, so the matching
    /// entry — same app block, same delay — identifies it uniquely.
    fn find_message_record(&self, period: u32) -> Option<u32> {
        if self.handler == 0 {
            return None;
        }
        let mut a = self.code_base;
        while a + 16 <= self.image_end {
            if self.mem.read_u32(a).ok() == Some(self.handler) {
                let id = self.mem.read_u32(a + 4).ok()?;
                let delay = self.mem.read_u32(a + 8).ok()?;
                if delay == period && id < 0x100 {
                    return Some(a);
                }
            }
            a += 4;
        }
        None
    }

    /// Fires every UI timer (+0xAC) whose period has elapsed, then advances
    /// the emulated clock by one frame.
    ///
    /// The guest callback runs on its own fresh frame (stack reset to the
    /// caller-visible top, LR parked on the stop sentinel) and returns into
    /// the frame loop, so it can freely reuse the guest stack.
    fn fire_ui_timers(&mut self, budget: u64) -> Result<(), MachineError> {
        if self.ui_timers.is_empty() {
            self.ui_clock_ms += FRAME_MS;
            return Ok(());
        }
        let now = self.ui_clock_ms + FRAME_MS;
        self.ui_clock_ms = now;
        // Iterate by index: the callback may arm or drop timers of its own.
        let mut i = 0;
        while i < self.ui_timers.len() {
            let t = self.ui_timers[i];
            if t.due_ms > now {
                i += 1;
                continue;
            }
            // Re-arm before the call so a callback that overwrites the slot
            // cannot make the pump fire the same period twice.
            let period = t.period_ms.max(1) as u64;
            self.ui_timers[i].due_ms = now + period;
            debug!(
                "[plat] +0xAC fire cb={:#010x} ctx={:#010x} at {now}ms",
                t.callback, t.context
            );
            self.cpu.reg_set(armv4t_emu::reg::SP, STACK_TOP);
            self.cpu.reg_set(armv4t_emu::reg::LR, STOP_ADDRESS);
            self.cpu.reg_set(armv4t_emu::reg::CPSR, 0x10);
            self.cpu.reg_set(0, t.context);
            self.cpu.reg_set(1, 0);
            self.cpu.reg_set(2, 0);
            self.cpu.reg_set(3, 0);
            self.cpu.reg_set(armv4t_emu::reg::PC, t.callback);
            self.run_until_return(budget)?;

            i += 1;
        }
        Ok(())
    }

    /// Completes the pending local HTTP transaction.
    ///
    /// The request descriptor registered through +0x570 carries a status
    /// word, a callback and a context. A real platform's network thread
    /// marks the request done and calls the callback once the response has
    /// been consumed; the guest never polls the descriptor itself. Returns
    /// `(callback, context)` so the frame loop can re-enter the guest.
    fn deliver_http_completion(&mut self) -> Option<(u32, u32)> {
        if !self.http.enabled || self.http.notified || self.http.response.is_empty() {
            return None;
        }
        if self.http.pos < self.http.response.len() {
            return None;
        }
        let req = self.http.request?;
        debug!(
            "[http] deliver req={req:#010x} pos={} len={} status={:#x}",
            self.http.pos,
            self.http.response.len(),
            self.mem.read_u32(req).unwrap_or(0)
        );
        let cb = self.mem.read_u32(req + 4).ok()?;
        let arg = self.mem.read_u32(req + 8).ok()?;
        let _ = self.mem.write_u32(req, 0);
        self.http.notified = true;
        Some((cb, arg))
    }

    /// Advances one frame: pump queued events, deliver the frame tick
    /// (selector 10, the host paces at ~20 fps), then deliver the guest's
    /// registered timer callback (the platform's +0x500 service is a
    /// passthrough on the wire, but a real device fires the callback on the
    /// frame cadence), and count the frame.
    pub fn run_frame(&mut self, budget: u64) -> Result<(), MachineError> {
        // Debug seeding: SF_SEED="addr=value;addr=value" (hex) writes guest
        // words once after boot, before the first frame tick. Lets runtime
        // experiments drive the guest's own UI state machines.
        if self.frame_count == 0 {
            if let Ok(seed) = std::env::var("SF_SEED") {
                for pair in seed.split(';') {
                    let (a, v) = pair.split_once('=').unwrap_or((pair, "0"));
                    if let (Ok(addr), Ok(val)) =
                        (usize::from_str_radix(a, 16), u32::from_str_radix(v, 16))
                    {
                        let _ = self.mem.write_u32(addr as u32, val);
                    }
                }
            }
        }
        // The shared SDK gates its UI state machine (and every repaint) on
        // the app-activate/resume event, selector 3. A foregrounded handset
        // keeps that state active on the frame cadence. Without it the
        // titles draw their boot splash once and then freeze.
        // SF_NO_APP_PUMP=1 (or `set_app_pump(false)`) disables it for
        // experiments; SF_RESUME=1 is the historical spelling of the same
        // forced-on switch.
        if self.app_pump || std::env::var_os("SF_RESUME").is_some() {
            self.events.post(3, 0);
        }
        // A local HTTP transaction that finished during boot has to be
        // reported through the SDK's own completion callback before the
        // frame's UI work runs.
        if let Some((cb, arg)) = self.deliver_http_completion() {
            debug!("[plat] HTTP result callback {cb:#010x} arg={arg:#x}");
            if cb > 0x1000 {
                self.cpu.reg_set(armv4t_emu::reg::SP, STACK_TOP);
                self.cpu.reg_set(armv4t_emu::reg::LR, STOP_ADDRESS);
                self.cpu.reg_set(armv4t_emu::reg::CPSR, 0x10);
                self.cpu.reg_set(0, arg);
                self.cpu.reg_set(1, 0);
                self.cpu.reg_set(2, 0);
                self.cpu.reg_set(armv4t_emu::reg::PC, cb);
                self.run_until_return(budget)?;
            }
        }
        self.pump(budget)?;
        // SF_FORCE="addr:val;..." (hex): write guest words EVERY frame
        // before the tick — for variables the guest's own flow clears but
        // an experiment needs held at a value.
        if let Ok(force) = std::env::var("SF_FORCE") {
            for pair in force.split(';') {
                if let Some((a, v)) = pair.split_once(':') {
                    if let (Ok(addr), Ok(val)) =
                        (u32::from_str_radix(a, 16), u32::from_str_radix(v, 16))
                    {
                        let _ = self.mem.write_u32(addr, val);
                    }
                }
            }
        }
        // Selector 3 is posted at the top of the frame (see above), so it
        // is pumped together with the key events of this frame.
        self.fire_ui_timers(budget)?;
        // The platform also delivers a bare frame tick. The corpus' own
        // pumps gate their selector-10 work on the message id the SDK
        // registered (3, or 1 for the older builds), so this call is what
        // the titles that read the tick directly use, not the main path.
        self.app_call(10, 0, budget)?;
        if let Some(t) = self.timer {
            // The callback receives a platform-allocated result record: the
            // registration's context word is a guest-internal value (hjkg
            // passes the just-read flag byte, i.e. zero), while the callback
            // dereferences r1 as a record with a status byte at +9, a type
            // byte at +4, a payload length halfword at +6, a UTF-16 string
            // at +0xB and a payload buffer at +0x3C. Zeroed = no result.
            let mut job = [0u8; TIMER_JOB_SIZE as usize];
            if std::env::var_os("SF_FAKE_PAY").is_some() {
                // Offline billing receipt. The shared SDK callback renders
                // the record's narrow string field (+0xB) into the match
                // target buffer through its own log formatter, copies the
                // same string into the pattern-item table, and then matches
                // the table against the target — so delivering a record
                // whose narrow string is the confirmation number makes the
                // guest's own code find a match and run its success action
                // (which serves its pay-info file through the file service).
                const RECEIPT: &str = "8";
                let mut text: Vec<u8> = RECEIPT.as_bytes().to_vec();
                text.push(0);
                let t = self.timer.expect("timer captured before delivery");
                if t.item_array != 0 && t.count_addr != 0 {
                    let mut item = [0u8; 0x20];
                    item[..text.len().min(0x20)].copy_from_slice(&text);
                    let _ = self.mem.write(t.item_array, &item);
                    let _ = self.mem.write_u8(t.count_addr, 1);
                }
                // Screen survey: cycle the UI screen index (one 0x14-byte
                // screen-table entry per 20 frames) so every screen renders
                // into the frame dumps. SF_SCREEN_SCAN=2 sweeps the game's
                // own screens (>= 18, past the SDK's dialog handlers). The
                // swept variable is per-binary: SF_SCAN_ADDR overrides.
                if std::env::var_os("SF_SCREEN_SCAN").is_some() {
                    let wide = std::env::var("SF_SCREEN_SCAN")
                        .map(|v| v == "2")
                        .unwrap_or(false);
                    let idx = if wide {
                        (self.frame_count / 20) as u32 % 23 + 18
                    } else {
                        (self.frame_count / 20) as u32 % 17 + 1
                    };
                    let addr = std::env::var("SF_SCAN_ADDR")
                        .ok()
                        .and_then(|a| u32::from_str_radix(&a, 16).ok())
                        .unwrap_or(0x13532C);
                    let _ = self.mem.write_u32(addr, idx);
                }
                // Binary-specific state synthesis (queue head 5 = the
                // pay-flow message type, s1 = the dispatcher state, the
                // pay-flow state 5, the nested-machine trigger, cooldown=1
                // and the send-pattern receipt) — addresses are sanguo's
                // decode; other binaries drive these through SF_SEED.
                if std::env::var_os("SF_PAY_SYNTH").is_some() {
                    // The callback's second match loop (the send-pattern
                    // table this binary registers from its payinfo file)
                    // gates the pay-flow state machine behind a match
                    // against the same target.
                    const SEND_ITEMS: u32 = 0x1353CC;
                    const SEND_COUNT: u32 = 0x1331E0;
                    let mut item = [0u8; 0x20];
                    item[..text.len().min(0x20)].copy_from_slice(&text);
                    let _ = self.mem.write(SEND_ITEMS, &item);
                    let _ = self.mem.write_u8(SEND_COUNT, 1);
                    // The network state machine's send-state counters gate
                    // the menu flow (0x122d68 → the UI setup calls).
                    let _ = self.mem.write_u32(0x1355F0 + 0x34C, 1);
                    let _ = self.mem.write_u32(0x1355F0 + 0x348, 1);
                    // The callback dispatches on the SMS queue head: head=5
                    // is the pay-flow message type whose handler invokes
                    // the message builder — the phase loop driver.
                    let _ = self.mem.write_u32(0x133164, 5);
                    // Seed once: s1 = 1 (the dispatcher state).
                    if self.frame_count == 0 {
                        let _ = self.mem.write_u32(0x1331A0, 1);
                    }
                    // The pay-flow state is only ever loaded from the
                    // payinfo file's tag stream; force state 5 (whose
                    // handler invokes the message builder each delivery).
                    let _ = self.mem.write_u32(0x13533C, 5);
                    // The flag bytes 0x1364B4/0x1364B5 control both the
                    // phase selection and the 0x117b74 nested machine.
                    let _ = self.mem.write_u8(0x1364B4, 1);
                    let _ = self.mem.write_u8(0x1364B5, 1);
                    // [0x133154] set routes the send handler's tail into
                    // the 0x117b74 nested machine (the screen-transition
                    // driver); cooldown=1 makes every 0x118988 gate call
                    // report "expired" so the advance paths fire.
                    let _ = self.mem.write_u8(0x133154, 1);
                    let _ = self.mem.write_u32(0x13498C, 1);
                }
                job[9] = 1; // result ready
                            // The callback's phase gate: the record's phase byte (+0xA)
                            // must be >= the pay-flow state for the match path to run.
                            // 0xFF passes at every state.
                job[0xA] = 0xFF;
                job[4] = 8; // payload carries halfwords
                let be: Vec<u8> = RECEIPT
                    .encode_utf16()
                    .flat_map(|c| [(c >> 8) as u8, (c & 0xFF) as u8])
                    .collect();
                job[6] = (be.len() / 2) as u8;
                job[7] = ((be.len() / 2) >> 8) as u8;
                let so = 0xB;
                job[so..so + text.len()].copy_from_slice(&text);
                let po = 0x3C;
                job[po..po + be.len()].copy_from_slice(&be);
            }
            self.mem.write(TIMER_JOB_ADDR, &job)?;
            if std::env::var_os("SF_PAY_WATCH").is_some() {
                let t = self.timer.expect("timer captured before delivery");
                let count = self.mem.read_u8(t.count_addr).unwrap_or(0xFF);
                let mut tgt = [0u8; 64];
                let _ = self.mem.read(t.target_addr, &mut tgt);
                let to_text = |b: &[u8]| -> String {
                    b.chunks(2)
                        .map(|c| u16::from_le_bytes([c[0], c[1]]))
                        .take_while(|&c| c != 0)
                        .map(|c| char::from_u32(c as u32).unwrap_or('?'))
                        .collect()
                };
                if self.frame_count.is_multiple_of(20) {
                    let mut dump = String::new();
                    for i in 0..count.min(12) {
                        let mut item = [0u8; 0x20];
                        let _ = self.mem.read(t.item_array + i as u32 * 0x20, &mut item);
                        dump.push_str(&format!(" [{}] {:?}", i, to_text(&item)));
                    }
                    debug!(
                        "[plat] timer job: frame={} count={} target={:?} items{}",
                        self.frame_count,
                        count,
                        to_text(&tgt),
                        dump
                    );
                }
            }
            self.cpu.reg_set(armv4t_emu::reg::SP, STACK_TOP);
            self.cpu.reg_set(armv4t_emu::reg::LR, STOP_ADDRESS);
            self.cpu.reg_set(armv4t_emu::reg::CPSR, 0x10);
            self.cpu.reg_set(0, t.id);
            self.cpu.reg_set(1, TIMER_JOB_ADDR);
            self.cpu.reg_set(2, 0);
            self.cpu.reg_set(3, 0);
            self.cpu.reg_set(armv4t_emu::reg::PC, t.entry);
            if std::env::var_os("SF_PAY_WATCH").is_some() {
                let scr = self.mem.read_u32(0x13532C).unwrap_or(0xFF);
                let pay = self.mem.read_u32(0x13533C).unwrap_or(0xFF);
                debug!("[plat] screen state pre-cb: {scr} pay-state: {pay}");
            }
            // Generic variable watch: SF_VARS="addr,addr" (hex) logged per
            // frame around the timer delivery.
            if let Ok(vars) = std::env::var("SF_VARS") {
                let vals: Vec<String> = vars
                    .split(',')
                    .filter_map(|a| u32::from_str_radix(a, 16).ok())
                    .map(|a| format!("{a:#x}={:#x}", self.mem.read_u32(a).unwrap_or(0xFFFF_FFFF)))
                    .collect();
                debug!("[plat] vars pre-cb: {}", vals.join(" "));
            }
            self.run_until_return(budget)?;
            if std::env::var_os("SF_PAY_WATCH").is_some() {
                let scr = self.mem.read_u32(0x13532C).unwrap_or(0xFF);
                let pay = self.mem.read_u32(0x13533C).unwrap_or(0xFF);
                debug!("[plat] screen state post-cb: {scr} pay-state: {pay}");
            }
            if std::env::var_os("SF_PAY_WATCH").is_some() {
                // The SDK's network state machine struct (pointer variable at
                // 0x1355F0 in sanguo): counters 0x348/0x34c gate the send.
                let cooldown = self.mem.read_u32(0x13498C).unwrap_or(0xFFFF);
                let s1 = self.mem.read_u32(0x1331A0).unwrap_or(0xFFFF);
                let s2 = self.mem.read_u8(0x133169).unwrap_or(0xFF);
                let s3 = self.mem.read_u8(0x1364B4).unwrap_or(0xFF);
                let s4 = self.mem.read_u8(0x1364B5).unwrap_or(0xFF);
                let phase = self.mem.read_u32(0x134990).unwrap_or(0xFFFF);
                let gate = self.mem.read_u32(0x133164).unwrap_or(0xFFFF);
                let disp_obj = self.mem.read_u32(0x137214).unwrap_or(0);
                let disp_vt = if disp_obj > 0x1000 {
                    self.mem.read_u32(disp_obj).unwrap_or(0)
                } else {
                    0
                };
                let cfg = self.mem.read_u8(0x133189).unwrap_or(0xFF);
                // The 0x115900 config struct at 0x13873E: 5 signed halfwords
                let hw: Vec<i16> = (0..5)
                    .filter_map(|k| self.mem.read_u16(0x13873E + k * 2).ok())
                    .map(|v| v as i16)
                    .collect();
                let buf5: Vec<u32> = (0..5)
                    .filter_map(|k| self.mem.read_u32(0x3FFF9C + k * 4).ok())
                    .collect();
                debug!("[plat] cfg hw={hw:?} buf5={buf5:?}");
                let expected = self
                    .mem
                    .read_u32(0x1367B8 + cfg as u32 * 4)
                    .unwrap_or(0xFFFF);
                debug!(
                    "[plat] net state: cooldown={cooldown} s1={s1} s2={s2} s3={s3} s4={s4}                      phase={phase:#010x} gate={gate:#010x} cfg={cfg} expect={expected:#010x}                      disp_obj={disp_obj:#010x} disp_vt={disp_vt:#010x}"
                );
                let count = self.mem.read_u8(t.count_addr).unwrap_or(0xFF);
                let mut tgt = [0u8; 64];
                let _ = self.mem.read(t.target_addr, &mut tgt);
                let to_text = |b: &[u8]| -> String {
                    b.chunks(2)
                        .map(|c| u16::from_le_bytes([c[0], c[1]]))
                        .take_while(|&c| c != 0)
                        .map(|c| char::from_u32(c as u32).unwrap_or('?'))
                        .collect()
                };
                debug!(
                    "[plat] timer cb done: frame={} count={} target={:?} lr={:#010x}",
                    self.frame_count,
                    count,
                    to_text(&tgt),
                    self.cpu.reg_get(armv4t_emu::reg::LR)
                );
            }
        }
        // Generic per-frame variable watch, independent of the timer
        // service so guests that never register one are observable too.
        if let Ok(vars) = std::env::var("SF_VARS") {
            let vals: Vec<String> = vars
                .split(',')
                .filter_map(|a| u32::from_str_radix(a, 16).ok())
                .map(|a| format!("{a:#x}={:#x}", self.mem.read_u32(a).unwrap_or(0xFFFF_FFFF)))
                .collect();
            debug!("[plat] frame {} vars: {}", self.frame_count, vals.join(" "));
        }
        self.frame_count += 1;
        Ok(())
    }

    /// The guest's app-state byte.
    pub fn state(&self) -> Result<u8, MachineError> {
        Ok(self.mem.read_u8(self.state_addr())?)
    }

    /// Frame count since boot.
    pub fn frame_count(&self) -> u64 {
        self.frame_count
    }

    /// Copy of the guest framebuffer as RGBA8 (minifb-ready).
    pub fn frame_pixels(&self) -> Vec<u32> {
        self.gdi.frame_pixels(&self.mem)
    }

    /// Whether the display changed since the last presentation.
    pub fn take_dirty(&mut self) -> bool {
        self.gdi.take_dirty()
    }

    /// GDI call counters (fill, text, draw, clear, fill2, flush).
    pub fn gdi_counters(&self) -> [u64; 6] {
        self.gdi.counters()
    }

    /// The guest framebuffer window as RGBA8 (for apps that draw directly
    /// into the plat-region surface instead of using the GDI bridge).
    pub fn guest_fb_pixels(&self) -> Vec<u32> {
        match self.mem.slice(plat::FB_ADDR, plat::FB_SIZE) {
            Ok(raw) => {
                let mut out = Vec::with_capacity((plat::FB_SIZE / 2) as usize);
                for i in 0..raw.len() / 2 {
                    let v = u16::from_le_bytes([raw[i * 2], raw[i * 2 + 1]]);
                    let r = ((v >> 11) & 0x1F) as u32;
                    let g = ((v >> 5) & 0x3F) as u32;
                    let b = (v & 0x1F) as u32;
                    out.push(0xFF00_0000 | (r << 19) | (g << 10) | (b << 3));
                }
                out
            }
            Err(_) => Vec::new(),
        }
    }

    /// Heap base (the guest's scratch window for decompression).
    pub fn heap_base(&self) -> u32 {
        HEAP_BASE
    }

    /// Copies a guest memory range out for inspection (debug tooling).
    pub fn dump_memory(&self, addr: u32, len: u32) -> Option<Vec<u8>> {
        self.mem.slice(addr, len).ok().map(<[u8]>::to_vec)
    }

    /// Content fingerprint over the container and resource bytes; save
    /// states carry it so they are only restored into matching content.
    fn content_fingerprint(&self) -> u64 {
        let containers: Vec<&[u8]> = self.res_containers.iter().map(|c| c.as_slice()).collect();
        crate::save::content_fingerprint(
            std::iter::once(self.container.as_slice()).chain(containers),
        )
    }

    /// Full-state snapshot as a versioned, self-checking blob.
    ///
    /// The blob covers everything the guest can observe across frames
    /// (memory, CPU, handler, file slots, event queue, counters, display
    /// bridge). Load artifacts (container, resource bytes, wrapper table)
    /// stay with the loaded machine and are not duplicated in the snapshot.
    pub fn serialize(&self) -> Vec<u8> {
        let state = crate::save::MachineState {
            mem: self.mem.clone(),
            cpu: self.cpu.clone(),
            handler: self.handler,
            file_slots: self.file_slots.clone(),
            events: self.events.clone(),
            frame_count: self.frame_count,
            lz_calls: self.lz_calls,
            last_pc: self.last_pc,
            branch_log: self.branch_log.clone(),
            gdi: self.gdi.clone(),
            timer: self.timer,
        };
        // Encoding a well-formed snapshot cannot fail; surface any codec
        // fault loudly rather than returning a corrupt state.
        crate::save::encode(&state, self.content_fingerprint())
            .expect("save-state encode of a live machine")
    }

    /// Restores a snapshot in place, keeping the loaded container, resource
    /// bytes, and wrapper table. States from other packages or unknown
    /// formats are rejected; guest memory is copied into the existing
    /// region allocations so pointers handed out before the restore stay
    /// valid.
    pub fn deserialize(&mut self, data: &[u8]) -> Result<(), crate::save::SaveStateError> {
        let state = crate::save::decode(data, self.content_fingerprint())?;
        self.mem
            .copy_regions_from(&state.mem)
            .map_err(|_| crate::save::SaveStateError::ContentMismatch)?;
        self.cpu = state.cpu;
        self.handler = state.handler;
        self.file_slots = state.file_slots;
        self.events = state.events;
        self.frame_count = state.frame_count;
        self.lz_calls = state.lz_calls;
        self.last_pc = state.last_pc;
        self.branch_log = state.branch_log;
        self.gdi = state.gdi;
        self.timer = state.timer;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// RGB565 white as `frame_pixels` expands it (5/6/5 bits spread over
    /// the 8-bit RGB fields, so the high bit of red lands in bit 23).
    const WHITE: u32 = 0xFFF8_FCF8;
    /// RGB565 red (0xF800) as `frame_pixels` expands it.
    const RED: u32 = 0xFFF8_0000;
    /// RGB565 blue (0x001F) as `frame_pixels` expands it.
    const BLUE: u32 = 0xFF00_00F8;

    /// Builds a minimal .app whose "entry" immediately registers a handler
    /// (written to the args slot) and returns to the sentinel, then an
    /// event handler that returns immediately.
    fn build_test_image() -> Vec<u8> {
        // entry (at CODE_BASE):
        //   ldr r0, [r1]          ; keep slot semantics visible
        //   ldr r2, =handler      ; relocated literal
        //   str r2, [r1]          ; register handler at args slot
        //   bx lr
        // handler:
        //   bx lr
        let entry: [u32; 4] = [
            0xE591_0000, // ldr r0, [r1]
            0xE59F_2004, // ldr r2, [pc, #4]  -> literal at +0x10
            0xE581_2000, // str r2, [r1]
            0xE12F_FF1E, // bx lr
        ];
        let handler: [u32; 1] = [0xE12F_FF1E]; // bx lr
        let mut code = Vec::new();
        for w in entry {
            code.extend_from_slice(&w.to_le_bytes());
        }
        // pad to 0x10 so the literal lands right after `bx lr`
        while code.len() < 0x10 {
            code.push(0xEE); // invalid-ish filler never executed
        }
        let handler_off = code.len() as u32;
        for w in handler {
            code.extend_from_slice(&w.to_le_bytes());
        }
        // literal: handler address relative to code_base (relocated)
        code.extend_from_slice(&handler_off.to_le_bytes());
        // relocation entry for the literal at offset 0x10
        let reloc = 0x10u32;

        let mut v = Vec::new();
        v.extend_from_slice(&0x1941_4653u32.to_le_bytes()); // magic
        v.extend_from_slice(&0u32.to_le_bytes()); // ver
        v.extend_from_slice(&0u32.to_le_bytes()); // reserved
        v.extend_from_slice(&0u32.to_le_bytes()); // reserved
        v.extend_from_slice(&0x20u32.to_le_bytes()); // hdr_len
        v.extend_from_slice(&(code.len() as u32).to_le_bytes()); // c0
        v.extend_from_slice(&0u32.to_le_bytes()); // c1
        v.extend_from_slice(&1u32.to_le_bytes()); // reloc count
        v.extend_from_slice(&code);
        v.extend_from_slice(&reloc.to_le_bytes());
        v
    }

    #[test]
    fn boots_and_registers_handler() {
        let container = build_test_image();
        let mut m = SnowFishMachine::load(container, &[]).unwrap();
        m.boot(1000).unwrap();
        assert_ne!(m.handler, 0, "handler registered");
        assert_eq!(m.state().unwrap(), 3);
        // Dispatch an event through the handler; the handler returns at once.
        m.app_call(6, 0x08, 1000).unwrap();
        assert_eq!(m.state().unwrap(), 3);
    }

    #[test]
    fn scan_wrappers_matches_synthetic_stublet() {
        // ldr r3,[r3,#-4]; mov lr,pc; ldr pc,[r3,#0x7C]; pop {pc}
        let code: Vec<u8> = [0xE513_3004u32, 0xE1A0_E00F, 0xE593_F07C, 0xE8BD_8000]
            .iter()
            .flat_map(|w| w.to_le_bytes())
            .collect();
        let w = scan_wrappers(&code);
        assert_eq!(w.len(), 1);
        assert_eq!(w[0].slot, 0x7C);
        assert_eq!(w[0].hook, CODE_BASE);
        assert_eq!(w[0].ret, CODE_BASE + 0xC);
    }

    /// +0x0C answers the screen height: the machine must leave the slot on
    /// the planted value stub instead of serving it as a heap allocator.
    /// Intercepting it made the guests' canvas geometry globals hold heap
    /// pointers, which collapsed every dialog fill to a 0-height rect.
    #[test]
    fn screen_height_slot_runs_the_planted_stub() {
        let m = SnowFishMachine::load(build_test_image(), &[]).unwrap();
        let w = Wrapper {
            slot: 0x0C,
            hook: CODE_BASE,
            ret: CODE_BASE + 0xC,
        };
        assert!(
            m.hooked_slot_ret(&w).is_none(),
            "+0x0C must not be intercepted"
        );
        assert_eq!(
            m.mem.read_u32(plat::ROOT_ADDR + 0x0C).unwrap(),
            plat::STUB_HEIGHT
        );
        assert_eq!(
            m.mem.read_u32(plat::STUB_HEIGHT + 8).unwrap(),
            plat::SCREEN_HEIGHT
        );
    }

    /// Text takes its pen colour from the canvas object at +0x28, and zero
    /// there is a real colour — black. Treating it as "unset" and falling
    /// back to white made every softkey label on a white dialog invisible.
    #[test]
    fn text_keeps_a_black_pen() {
        let mut m = SnowFishMachine::load(build_test_image(), &[]).unwrap();
        let obj = HEAP_BASE;
        let text = HEAP_BASE + 0x100;
        m.mem.write_u32(obj + 0x28, 0).unwrap(); // pen = black
        m.mem.write_u16(text, 0x41).unwrap(); // 'A'
        m.mem.write_u16(text + 2, 0).unwrap();
        m.gdi.fill(&mut m.mem, 0, 0, 240, 320, 0xFFFF);
        m.cpu.reg_set(0, obj);
        m.cpu.reg_set(1, text);
        m.cpu.reg_set(2, 10); // x
        m.cpu.reg_set(3, 10); // y
        m.exec_gdi(0x154, 0).unwrap();
        let px = m.gdi.frame_pixels(&m.mem);
        let ink = (0..8)
            .flat_map(|row| (0..8).map(move |col| (10 + row) * 240 + 10 + col))
            .filter(|&i| px[i] == 0xFF00_0000)
            .count();
        assert!(ink > 0, "'A' should be drawn in black, not white-on-white");
    }

    /// +0x12C commits a rectangle of the surface to the display; it does not
    /// paint. The second stack word is *not* a colour — every title in the
    /// corpus leaves stale caller data there (code pointers, string
    /// pointers, zero) — so treating it as RGB565 scribbled a random colour
    /// over the frame and erased every sprite drawn before the call.
    #[test]
    fn update_rect_leaves_the_surface_alone() {
        let mut m = SnowFishMachine::load(build_test_image(), &[]).unwrap();
        let sp = HEAP_BASE + 0x200;
        m.cpu.reg_set(armv4t_emu::reg::SP, sp);
        m.mem.write_u32(sp + 4, 320).unwrap(); // h (5th argument)
        m.mem.write_u32(sp + 8, 0x4151_7000).unwrap(); // stale word
        m.gdi.fill(&mut m.mem, 0, 0, 240, 320, 0xFFFF);
        m.cpu.reg_set(0, HEAP_BASE);
        m.cpu.reg_set(1, 0);
        m.cpu.reg_set(2, 0);
        m.cpu.reg_set(3, 240);
        m.exec_gdi(0x12C, 0).unwrap();
        let px = m.gdi.frame_pixels(&m.mem);
        let bad = px.iter().filter(|&&p| p != WHITE).count();
        assert_eq!(bad, 0, "committing a rectangle must not repaint it");
    }

    /// +0x134 is the five-argument blit — `draw(obj, bmp, x, y, flags)` — and
    /// the clip window is the bitmap's own size. It used to be a silent
    /// no-op, which left every title that paints through it with nothing but
    /// its background.
    #[test]
    fn blit_draws_the_bitmap_at_its_own_size() {
        let mut m = SnowFishMachine::load(build_test_image(), &[]).unwrap();
        let desc = HEAP_BASE;
        let data = HEAP_BASE + 0x100;
        m.mem.write_u32(desc, 4).unwrap(); // sw
        m.mem.write_u32(desc + 4, 4).unwrap(); // sh
        m.mem.write_u32(desc + 8, 0).unwrap(); // key: none
        m.mem.write_u32(desc + 12, 0).unwrap(); // pal: none
        m.mem.write_u32(desc + 16, data).unwrap(); // pixels
        for i in 0..16u32 {
            m.mem.write_u16(data + i * 2, 0xF800).unwrap(); // red
        }
        m.gdi.fill(&mut m.mem, 0, 0, 240, 320, 0xFFFF);
        m.cpu.reg_set(0, HEAP_BASE + 0x300);
        m.cpu.reg_set(1, desc);
        m.cpu.reg_set(2, 10);
        m.cpu.reg_set(3, 20);
        m.exec_gdi(0x134, 0).unwrap();
        let px = m.gdi.frame_pixels(&m.mem);
        assert_eq!(px[20 * 240 + 10], RED, "first pixel of the blit");
        assert_eq!(px[23 * 240 + 13], RED, "last pixel of the blit");
        assert_eq!(px[20 * 240 + 14], WHITE, "nothing past the bitmap");
    }

    /// A palette blit with key=0 must not key out index 0. The descriptor's
    /// key field only carries a chroma index when the guest set one; keying
    /// zero unconditionally erased the dominant palette colour — 宇宙风暴's
    /// full-screen title bitmap is index 0 for the logo and the star field
    /// with key=0, and the blit painted an all-black frame.
    #[test]
    fn palette_blit_keeps_index_zero_when_key_is_unset() {
        let mut m = SnowFishMachine::load(build_test_image(), &[]).unwrap();
        let desc = HEAP_BASE;
        let pal = HEAP_BASE + 0x40;
        let data = HEAP_BASE + 0x100;
        m.mem.write_u32(desc, 2).unwrap(); // sw
        m.mem.write_u32(desc + 4, 2).unwrap(); // sh
        m.mem.write_u32(desc + 8, 0).unwrap(); // key: none
        m.mem.write_u32(desc + 12, pal).unwrap(); // palette pointer
        m.mem.write_u32(desc + 16, data).unwrap(); // indices
        m.mem.write_u16(pal, 0xF800).unwrap(); // index 0: red
        m.mem.write_u16(pal + 2, 0x001F).unwrap(); // index 1: blue
        // index 0 first (red), index 1 second (blue)
        m.mem.write_u16(data, 0x0100).unwrap(); // bytes [0, 1]
        m.mem.write_u16(data + 2, 0x0100).unwrap();
        m.gdi.fill(&mut m.mem, 0, 0, 240, 320, 0xFFFF);
        m.cpu.reg_set(0, HEAP_BASE + 0x300);
        m.cpu.reg_set(1, desc);
        m.cpu.reg_set(2, 0);
        m.cpu.reg_set(3, 0);
        m.exec_gdi(0x134, 0).unwrap();
        let px = m.gdi.frame_pixels(&m.mem);
        assert_eq!(px[0], RED, "index-0 pixel must draw when key=0");
        assert_eq!(px[1], BLUE, "index-1 pixel draws its palette colour");
    }

    /// Dialog bodies are drawn by `+0x14C`, which carries a character range
    /// plus the text origin instead of a pixel pointer. Skipping the slot
    /// paints the dialog frame and leaves the body blank, which is exactly
    /// how every trial title's notice looked before.
    #[test]
    fn text_runs_land_at_the_guests_origin() {
        let mut m = SnowFishMachine::load(build_test_image(), &[]).unwrap();
        let obj = HEAP_BASE;
        let text = HEAP_BASE + 0x100;
        let sp = HEAP_BASE + 0x400;
        m.mem.write_u32(obj + 0x28, 0x00FF_0000).unwrap(); // red pen
        m.mem.write_u16(obj + 0x2C, 0xF800).unwrap();
        // "AB" — the run starts at index 0 and covers both cells.
        m.mem.write_u16(text, 0x41).unwrap();
        m.mem.write_u16(text + 2, 0x42).unwrap();
        m.mem.write_u16(text + 4, 0).unwrap();
        m.gdi.fill(&mut m.mem, 0, 0, 240, 320, 0xFFFF);
        // Stack args 5/6 are the origin; sp_adjust() reads 0 at ret = 0, so
        // the first argument sits one word above sp.
        m.mem.write_u32(sp + 4, 30).unwrap();
        m.mem.write_u32(sp + 8, 40).unwrap();
        m.cpu.reg_set(armv4t_emu::reg::SP, sp);
        m.cpu.reg_set(0, obj);
        m.cpu.reg_set(1, text);
        m.cpu.reg_set(2, 0);
        m.cpu.reg_set(3, 2);
        m.exec_gdi(0x14C, 0).unwrap();
        let px = m.gdi.frame_pixels(&m.mem);
        let inked = (40..48)
            .flat_map(|y| (30..46).map(move |x| (x, y)))
            .filter(|&(x, y)| px[(y * 240 + x) as usize] == RED)
            .count();
        assert!(inked > 0, "the run must be inked at the guest's origin");
        assert_eq!(px[20 * 240 + 20], WHITE, "nothing above the run");
        assert_eq!(px[60 * 240 + 30], WHITE, "nothing below the run");
    }

    /// Softkey labels are anchored to the surface edges: the right-hand one
    /// lands on `width - 5` and the bottom row on `height + n`, one cell past
    /// the last row. Clipping them away loses every softkey label in the
    /// corpus, so the platform slides a label back inside instead.
    #[test]
    fn labels_are_slid_back_onto_the_surface() {
        let mut m = SnowFishMachine::load(build_test_image(), &[]).unwrap();
        let obj = HEAP_BASE;
        let text = HEAP_BASE + 0x100;
        m.mem.write_u32(obj + 0x28, 0x00FF_FFFF).unwrap(); // white pen
        m.mem.write_u16(text, 0x41).unwrap(); // 'A'
        m.mem.write_u16(text + 2, 0).unwrap();
        m.gdi.fill(&mut m.mem, 0, 0, 240, 320, 0);
        m.cpu.reg_set(0, obj);
        m.cpu.reg_set(1, text);
        m.cpu.reg_set(2, 236); // one cell past the right edge
        m.cpu.reg_set(3, 320); // one cell past the last row
        m.exec_gdi(0x154, 0).unwrap();
        let px = m.gdi.frame_pixels(&m.mem);
        let ink: Vec<(i32, i32)> = (0..320)
            .flat_map(|y| (0..240).map(move |x| (x, y)))
            .filter(|&(x, y)| px[(y * 240 + x) as usize] != 0xFF00_0000)
            .collect();
        assert!(!ink.is_empty(), "the label must land inside the surface");
        assert!(
            ink.iter()
                .all(|&(x, y)| (232..240).contains(&x) && (312..320).contains(&y)),
            "every glyph pixel belongs to the box slid back to (232, 312)"
        );
    }

    /// Builds an image whose entry registers an app-handler block and whose
    /// code at +0x18 is a callback that pushes r0 onto the guest stack and
    /// returns — enough to observe a platform timer delivery.
    fn build_ui_timer_image() -> Vec<u8> {
        // The loader plants the platform root pointer at image end - 8, so
        // the last two words are padding rather than code.
        let words: [u32; 13] = [
            0xE59F_2008, // 0x00: ldr r2, [pc, #8]  -> literal at 0x10
            0xE581_2000, // 0x04: str r2, [r1]      ; register the app block
            0xE12F_FF1E, // 0x08: bx lr
            0xE1A0_0000, // 0x0C: nop
            0x0000_0024, // 0x10: app block offset (relocated)
            0xE12F_FF1E, // 0x14: handler: bx lr
            0xE52D_0004, // 0x18: callback: str r0, [sp, #-4]!
            0xE12F_FF1E, // 0x1C: callback: bx lr
            0xE1A0_0000, // 0x20: nop
            0x0000_0014, // 0x24: app block word 0 -> handler entry (relocated)
            0xE1A0_0000, // 0x28: nop
            0xE1A0_0000, // 0x2C: padding (root pointer planted at end - 8)
            0xE1A0_0000, // 0x30: padding
        ];
        let mut code = Vec::new();
        for w in words {
            code.extend_from_slice(&w.to_le_bytes());
        }
        let mut v = Vec::new();
        v.extend_from_slice(&0x1941_4653u32.to_le_bytes()); // magic
        v.extend_from_slice(&0u32.to_le_bytes()); // ver
        v.extend_from_slice(&0u32.to_le_bytes()); // reserved
        v.extend_from_slice(&0u32.to_le_bytes()); // reserved
        v.extend_from_slice(&0x20u32.to_le_bytes()); // hdr_len
        v.extend_from_slice(&(code.len() as u32).to_le_bytes()); // c0
        v.extend_from_slice(&0u32.to_le_bytes()); // c1
        v.extend_from_slice(&2u32.to_le_bytes()); // reloc count
        v.extend_from_slice(&code);
        v.extend_from_slice(&0x10u32.to_le_bytes()); // literal at 0x10
        v.extend_from_slice(&0x24u32.to_le_bytes()); // block word at 0x24
        v
    }

    /// Slot +0xAC arms the platform's repeating UI timer and the frame loop
    /// must enter the guest callback when its period elapses. Nothing in the
    /// corpus painted past its background while the slot went unanswered —
    /// the SDK re-enters the app handler with selector 10 from inside the
    /// callback, so an unanswered arm leaves every screen stack empty.
    #[test]
    fn ui_timer_callback_runs_on_the_frame_clock() {
        let mut m = SnowFishMachine::load(build_ui_timer_image(), &[]).unwrap();
        m.boot(2000).unwrap();
        let callback = CODE_BASE + 0x18;
        m.ui_timers.push(UiTimer {
            period_ms: 40,
            callback,
            context: 0x1234,
            due_ms: 40,
        });
        // Frame 1 ends at 33 ms — before the 40 ms period.
        m.run_frame(2000).unwrap();
        assert_eq!(m.mem.read_u32(STACK_TOP - 4).unwrap(), 0, "not due yet");
        // Frame 2 ends at 66 ms — the callback must have run with the
        // recorded context in r0.
        m.run_frame(2000).unwrap();
        assert_eq!(m.mem.read_u32(STACK_TOP - 4).unwrap(), 0x1234);
        // The period re-arms, so frame 3 is quiet again.
        m.mem.write_u32(STACK_TOP - 4, 0).unwrap();
        m.run_frame(2000).unwrap();
        assert_eq!(m.mem.read_u32(STACK_TOP - 4).unwrap(), 0, "re-armed");
        m.run_frame(2000).unwrap();
        assert_eq!(m.mem.read_u32(STACK_TOP - 4).unwrap(), 0x1234);
    }

    /// Root slot +0x524 is the service lookup. The guests call through the
    /// object it hands back without checking the result, so the layout must
    /// contain a callable method table — an unwritten out-pointer used to
    /// become a jump to 0 at mota's boot.
    #[test]
    fn service_slot_hands_out_a_callable_object() {
        let mut m = SnowFishMachine::load(build_test_image(), &[]).unwrap();
        let w = Wrapper {
            slot: 0x524,
            hook: CODE_BASE,
            ret: CODE_BASE + 0xC,
        };
        assert!(
            m.hooked_slot_ret(&w).is_some(),
            "+0x524 must be served by the host"
        );

        // Layout: object word -> method table, every entry -> return-0 stub.
        assert_eq!(
            m.mem.read_u32(plat::SERVICE_OBJ_ADDR).unwrap(),
            plat::SERVICE_VTABLE_ADDR
        );
        for i in 0..plat::SERVICE_VTABLE_LEN {
            assert_eq!(
                m.mem.read_u32(plat::SERVICE_VTABLE_ADDR + i * 4).unwrap(),
                plat::STUB_ADDR
            );
        }
        assert_eq!(
            m.mem.read_u32(plat::SERVICE_VTABLE_ADDR + 0x3C).unwrap(),
            plat::STUB_ADDR,
            "the guests call vtable+0x3C"
        );
        assert_eq!(
            m.mem.read_u32(plat::STUB_ADDR).unwrap(),
            0xE3A0_0000,
            "the shared stub is `mov r0,#0`"
        );

        // Typed lookup: the out-pointer receives the object.
        let out = plat::ROOT_ADDR + 0x2000;
        m.cpu.reg_set(0, 0x30);
        m.cpu.reg_set(1, 0x1219);
        m.cpu.reg_set(2, out);
        m.exec_slot(0x524, CODE_BASE).unwrap();
        assert_eq!(m.mem.read_u32(out).unwrap(), plat::SERVICE_OBJ_ADDR);
        assert_eq!(m.cpu.reg_get(0), 1);

        // Untargeted call (register garbage as the type): the historical
        // record-the-pointer behaviour, with the out-pointer left alone.
        let junk_out = plat::ROOT_ADDR + 0x2100;
        m.cpu.reg_set(0, 0x120D_E2BD);
        m.cpu.reg_set(1, 0xCF14_D7B7);
        m.cpu.reg_set(2, junk_out);
        m.exec_slot(0x524, CODE_BASE).unwrap();
        assert_eq!(m.mem.read_u32(junk_out).unwrap(), 0);
        assert_eq!(
            m.mem.read_u32(plat::STATE_PTR_ADDR).unwrap(),
            junk_out,
            "untargeted calls keep the legacy store"
        );
    }

    #[test]
    fn vfile_classification() {
        assert_eq!(vfile_kind("__sfssuid"), Some(0));
        assert_eq!(vfile_kind("ssu112"), Some(1));
        assert_eq!(vfile_kind("ssu321.dat"), Some(1));
        assert_eq!(vfile_kind("__sfnetinfo__"), Some(2));
        assert_eq!(vfile_kind("game_login.dat"), Some(3));
        assert_eq!(vfile_kind("__sfgpsav"), Some(4));
        assert_eq!(vfile_kind("mysave_save_1.dat"), Some(4));
        assert_eq!(vfile_kind("data.bin"), None);
        assert_eq!(basename("E:\\Application\\data.bin"), "data.bin");
    }

    /// `+0x4C` is the pay service's first question: can this handset bill
    /// over the carrier channel? The platform has a SIM, so the honest answer
    /// is yes; a zero makes the service exit before it looks anything up and
    /// leaves most of the corpus parked on its pay page.
    #[test]
    fn billing_capable_slot_answers_for_a_phone() {
        let mut m = SnowFishMachine::load(build_test_image(), &[]).unwrap();
        m.cpu.reg_set(0, 0);
        m.exec_slot(0x4C, CODE_BASE).unwrap();
        assert_eq!(m.cpu.reg_get(0), 1, "+0x4C must report a billing channel");
    }

    /// `+0x524` with the SDK's own hash instead of a small type id is the
    /// subscriber-identity lookup: 0 means "found" and the out-pointer has to
    /// receive an entry the pay service can both call directly and index as
    /// a method table. Off free play the lookup reports "missing" and leaves
    /// the out-pointer alone, which is what parks the titles on the
    /// registration page.
    #[test]
    fn sim_lookup_is_answered_under_free_play() {
        let mut m = SnowFishMachine::load(build_test_image(), &[]).unwrap();
        let out = plat::ROOT_ADDR + 0x2200;
        // type/id as the pay service passes them, out-pointer to a scratch word.
        m.cpu.reg_set(0, 0x120D_E2BD);
        m.cpu.reg_set(1, 0xCF14_D7B7);
        m.cpu.reg_set(2, out);
        m.exec_slot(0x524, CODE_BASE).unwrap();
        assert_eq!(m.cpu.reg_get(0), 1, "no identity without free play");
        assert_eq!(m.mem.read_u32(out).unwrap(), 0, "out-pointer stays alone");

        m.set_free_play(true);
        m.cpu.reg_set(0, 0x120D_E2BD);
        m.cpu.reg_set(1, 0xCF14_D7B7);
        m.cpu.reg_set(2, out);
        m.exec_slot(0x524, CODE_BASE).unwrap();
        assert_eq!(m.cpu.reg_get(0), 0, "free play finds the identity");
        let entry = m.mem.read_u32(out).unwrap();
        assert_eq!(
            entry,
            plat::SERVICE_CALLABLE_ADDR,
            "the out-pointer must hold the dual-shape entry"
        );
        // Shape A: `ldr pc, [entry]` must land on executable code.
        let first = m.mem.read_u32(entry).unwrap();
        assert_eq!(
            first,
            u32::from_le_bytes(plat::STUB_RET0[..4].try_into().unwrap()),
            "direct calls must reach the return-0 trampoline"
        );
        assert_eq!(
            m.mem.read_u32(entry + 4).unwrap(),
            u32::from_le_bytes(plat::STUB_RET0[4..].try_into().unwrap()),
            "the trampoline must actually return"
        );
        // Shape B: `ldr pc, [entry + 0x50]` must resolve to that same code,
        // not to a literal word from the shared stub area. Handing out the
        // bare STUB_ADDR fails here and aborts the whole boot event.
        assert_eq!(
            m.mem.read_u32(entry + 0x50).unwrap(),
            entry,
            "virtual calls must resolve to the callable trampoline"
        );
        for off in [0x18u32, 0x3C, 0x54, 0x7C] {
            assert_eq!(
                m.mem.read_u32(entry + off).unwrap(),
                entry,
                "method offset {off:#x} must resolve too"
            );
        }
    }

    /// The account record (`__sfssuid`, `ssu<id>`) is the SDK's proof that
    /// the handset registered once. Free play backs it with the in-memory
    /// service store so the guest writes a record at boot and reads the same
    /// bytes back; without free play the probe is refused, so every drive
    /// root fails and the title drops to its registration page.
    #[test]
    fn account_record_files_are_served_under_free_play() {
        let mut m = SnowFishMachine::load(build_test_image(), &[]).unwrap();
        let path_at = plat::ROOT_ADDR + 0x3000;
        let write_path = |m: &mut SnowFishMachine, s: &str| {
            let mut bytes = Vec::new();
            for unit in s.encode_utf16() {
                bytes.extend_from_slice(&unit.to_le_bytes());
            }
            bytes.extend_from_slice(&[0, 0]);
            m.mem.write(path_at, &bytes).unwrap();
        };

        write_path(&mut m, "E:\\Application\\__sfssuid");
        m.cpu.reg_set(0, path_at);
        m.cpu.reg_set(1, 0);
        m.exec_slot(0x58, CODE_BASE).unwrap();
        assert_eq!(m.cpu.reg_get(0), 0, "no account file without free play");

        m.set_free_play(true);
        m.cpu.reg_set(0, path_at);
        m.cpu.reg_set(1, 0);
        m.exec_slot(0x58, CODE_BASE).unwrap();
        let handle = m.cpu.reg_get(0);
        assert!(handle != 0, "free play serves the account record");

        // Write a record through the handle, close it, and read it back.
        let payload_at = plat::ROOT_ADDR + 0x3100;
        let payload = [7u8, 0, 0, 0, 0xFC];
        m.mem.write(payload_at, &payload).unwrap();
        m.cpu.reg_set(0, handle);
        m.cpu.reg_set(1, payload_at);
        m.cpu.reg_set(2, payload.len() as u32);
        m.exec_slot(0x60, CODE_BASE).unwrap();
        m.cpu.reg_set(0, handle);
        m.exec_slot(0x68, CODE_BASE).unwrap();

        m.cpu.reg_set(0, path_at);
        m.cpu.reg_set(1, 0);
        m.exec_slot(0x58, CODE_BASE).unwrap();
        let handle = m.cpu.reg_get(0);
        assert!(handle != 0, "the record is still there after a close");
        let read_at = plat::ROOT_ADDR + 0x3200;
        m.cpu.reg_set(0, handle);
        m.cpu.reg_set(1, read_at);
        m.cpu.reg_set(2, payload.len() as u32);
        m.exec_slot(0x5C, CODE_BASE).unwrap();
        let mut back = [0u8; 5];
        m.mem.read(read_at, &mut back).unwrap();
        assert_eq!(back, payload, "the guest reads back what it wrote");
    }

    /// Builds a publisher resource container that declares `name` and
    /// carries one pack of `body_len` bytes behind it.
    ///
    /// The layout is the one the resource generator writes: the UTF-16
    /// file name first, then the pack as a length-prefixed blob whose
    /// header is the fixed `1, 1, 6, count` shape.
    fn build_pack_container(name: &str, entries: u16, body_len: usize) -> Vec<u8> {
        let mut code = Vec::new();
        for unit in name.encode_utf16() {
            code.extend_from_slice(&unit.to_le_bytes());
        }
        code.extend_from_slice(&[0, 0]);
        let mut payload = Vec::new();
        payload.extend_from_slice(&1u16.to_le_bytes());
        payload.extend_from_slice(&1u16.to_le_bytes());
        payload.extend_from_slice(&6u16.to_le_bytes());
        payload.extend_from_slice(&entries.to_le_bytes());
        payload.resize(4 + body_len, 0x5A);
        code.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        code.extend_from_slice(&payload);
        let mut v = Vec::new();
        v.extend_from_slice(&0x1941_4653u32.to_le_bytes()); // magic
        v.extend_from_slice(&0u32.to_le_bytes()); // ver
        v.extend_from_slice(&0u32.to_le_bytes()); // reserved
        v.extend_from_slice(&0u32.to_le_bytes()); // reserved
        v.extend_from_slice(&0x20u32.to_le_bytes()); // hdr_len
        v.extend_from_slice(&(code.len() as u32).to_le_bytes()); // c0
        v.extend_from_slice(&0u32.to_le_bytes()); // c1
        v.extend_from_slice(&0u32.to_le_bytes()); // reloc count
        v.extend_from_slice(&code);
        v
    }

    /// A download request names the pack it wants, and the platform answers
    /// with that pack's own bytes. The guest has two length gates on the
    /// result — the downloaded body and the on-disk size — and both must
    /// see the pack's real length, so the resource load runs without a
    /// carrier round trip.
    #[test]
    fn a_pack_named_by_the_request_is_served_with_its_own_bytes() {
        let res = build_pack_container("FooRes_QVGA.pak", 9, 64);
        let m = SnowFishMachine::load(build_test_image(), &[res]).unwrap();
        let packs = m.resource_packs();
        assert_eq!(packs.len(), 1);
        assert_eq!(packs[0].0, "FooRes_QVGA.pak");
        assert_eq!(packs[0].1, 68, "the pack length excludes the prefix");

        // The request path names the pack; the lookup resolves it by name.
        assert_eq!(
            m.pack_for_request("GET /GM_PAK/FooRes_QVGA.pak HTTP/1.0"),
            Some(0)
        );
        assert_eq!(
            m.pack_for_request("GET /GM_PAK/Other_QVGA.pak HTTP/1.0"),
            None,
            "a pack the container does not carry is not answered"
        );
    }

    /// A guest that opens the pack by name gets the pack, not the container
    /// that holds it: the file's size must be the pack's length, because
    /// the guest compares it against the length it asked for.
    #[test]
    fn a_pack_opened_by_name_has_the_packs_length() {
        let res = build_pack_container("FooRes_QVGA.pak", 9, 64);
        let mut m = SnowFishMachine::load(build_test_image(), &[res]).unwrap();
        let path_at = plat::ROOT_ADDR + 0x3000;
        let mut bytes = Vec::new();
        for unit in "E:\\Application\\FooRes_QVGA.pak".encode_utf16() {
            bytes.extend_from_slice(&unit.to_le_bytes());
        }
        bytes.extend_from_slice(&[0, 0]);
        m.mem.write(path_at, &bytes).unwrap();
        m.cpu.reg_set(0, path_at);
        m.cpu.reg_set(1, 0);
        m.exec_slot(0x58, CODE_BASE).unwrap();
        let handle = m.cpu.reg_get(0);
        assert_ne!(handle, 0, "the pack is opened");
        let slot = &m.file_slots[(handle - 1) as usize];
        assert_eq!(slot.size, 68, "the size is the pack's length");
        assert_eq!(slot.backing, SlotBacking::Pack(0));

        // The bytes read back are the pack's payload.
        let read_at = plat::ROOT_ADDR + 0x3200;
        m.cpu.reg_set(0, handle);
        m.cpu.reg_set(1, read_at);
        m.cpu.reg_set(2, 8);
        m.exec_slot(0x5C, CODE_BASE).unwrap();
        let mut head = [0u8; 8];
        m.mem.read(read_at, &mut head).unwrap();
        assert_eq!(&head, &[1, 0, 1, 0, 6, 0, 9, 0], "the pack header");
    }

    /// A guest path the platform does not know is the guest's own save
    /// slot, and it must start empty. Serving the package image as its
    /// content makes the guest validate the container bytes as a save
    /// record and report "invalid save" (无效存档).
    #[test]
    fn unknown_paths_read_as_empty_guest_files() {
        let mut m = SnowFishMachine::load(build_test_image(), &[]).unwrap();
        let path_at = plat::ROOT_ADDR + 0x3000;
        let mut bytes = Vec::new();
        for unit in "E:////Application////TRUNK_SAV".encode_utf16() {
            bytes.extend_from_slice(&unit.to_le_bytes());
        }
        bytes.extend_from_slice(&[0, 0]);
        m.mem.write(path_at, &bytes).unwrap();
        m.cpu.reg_set(0, path_at);
        m.cpu.reg_set(1, 0);
        m.exec_slot(0x58, CODE_BASE).unwrap();
        let handle = m.cpu.reg_get(0);
        assert!(handle != 0, "the guest's own file is opened");
        assert_eq!(
            m.file_slots[(handle - 1) as usize].size,
            0,
            "an unwritten guest file reads as empty, not as the package image"
        );
    }

    /// The guest's own file must accept writes and hand them back on the
    /// next open, so a game can create and later reload its save slot.
    #[test]
    fn unknown_paths_round_trip_the_guests_own_writes() {
        let mut m = SnowFishMachine::load(build_test_image(), &[]).unwrap();
        let path_at = plat::ROOT_ADDR + 0x3000;
        let mut bytes = Vec::new();
        for unit in "E:////Application////TRUNK_SAV".encode_utf16() {
            bytes.extend_from_slice(&unit.to_le_bytes());
        }
        bytes.extend_from_slice(&[0, 0]);
        m.mem.write(path_at, &bytes).unwrap();
        m.cpu.reg_set(0, path_at);
        m.cpu.reg_set(1, 0);
        m.exec_slot(0x58, CODE_BASE).unwrap();
        let handle = m.cpu.reg_get(0);
        assert!(handle != 0);

        let payload_at = plat::ROOT_ADDR + 0x3100;
        let payload = [3u8, 1, 4, 1, 5];
        m.mem.write(payload_at, &payload).unwrap();
        m.cpu.reg_set(0, handle);
        m.cpu.reg_set(1, payload_at);
        m.cpu.reg_set(2, payload.len() as u32);
        m.exec_slot(0x60, CODE_BASE).unwrap();
        m.cpu.reg_set(0, handle);
        m.exec_slot(0x68, CODE_BASE).unwrap();

        m.cpu.reg_set(0, path_at);
        m.cpu.reg_set(1, 0);
        m.exec_slot(0x58, CODE_BASE).unwrap();
        let handle = m.cpu.reg_get(0);
        assert!(handle != 0);
        let read_at = plat::ROOT_ADDR + 0x3200;
        m.cpu.reg_set(0, handle);
        m.cpu.reg_set(1, read_at);
        m.cpu.reg_set(2, payload.len() as u32);
        m.exec_slot(0x5C, CODE_BASE).unwrap();
        let mut back = [0u8; 5];
        m.mem.read(read_at, &mut back).unwrap();
        assert_eq!(back, payload, "the guest reloads what it saved");
    }

    /// Save states round-trip the observable runtime state: frame count,
    /// handler, and guest memory survive serialize -> deserialize.
    #[test]
    fn save_state_round_trips_runtime_state() {
        let container = build_test_image();
        let mut machine = SnowFishMachine::load(container, &[]).unwrap();
        machine.boot(1000).unwrap();
        machine.app_call(6, 0x08, 1000).unwrap();
        let blob = machine.serialize();

        // Run further ahead, then rewind and confirm the earlier state.
        machine.run_frame(1000).unwrap();
        machine.run_frame(1000).unwrap();
        let ahead_frames = machine.frame_count();
        assert_eq!(ahead_frames, 2);

        machine.deserialize(&blob).unwrap();
        assert!(machine.frame_count() < ahead_frames);
        assert_eq!(machine.state().unwrap(), 3);
        assert_ne!(machine.handler, 0);
        // The restored machine keeps executing guest calls.
        machine.app_call(6, 0x08, 1000).unwrap();
        assert_eq!(machine.state().unwrap(), 3);
    }

    /// A state from other content (different fingerprint) must be rejected
    /// without touching the running machine.
    #[test]
    fn save_state_rejects_foreign_content() {
        let mut machine = SnowFishMachine::load(build_test_image(), &[]).unwrap();
        machine.boot(1000).unwrap();
        let blob = machine.serialize();

        // Flip a container byte to fake a different package.
        let mut other = SnowFishMachine::load(build_test_image(), &[]).unwrap();
        other.container[0x30] ^= 0xFF;
        assert!(matches!(
            other.deserialize(&blob),
            Err(crate::save::SaveStateError::ContentMismatch)
        ));

        // Garbage blobs are rejected as well.
        assert!(matches!(
            machine.deserialize(b"garbage!"),
            Err(crate::save::SaveStateError::BadSignature)
        ));
    }

    /// Socket slot +0xE4 is `write(handle, buffer, length)`. The SDK checks
    /// the result against -1 and otherwise uses it as the byte count, so the
    /// generic return-0 stub silently claims that no request was sent.
    #[test]
    fn socket_write_reports_the_byte_count() {
        let mut m = SnowFishMachine::load(build_test_image(), &[]).unwrap();
        let w = Wrapper {
            slot: 0xE4,
            hook: CODE_BASE,
            ret: CODE_BASE + 0xC,
        };
        assert!(
            m.hooked_slot_ret(&w).is_some(),
            "+0xE4 must be served by the host"
        );
        m.cpu.reg_set(0, 0); // socket handle
        m.cpu.reg_set(1, 0x6143E4); // request buffer
        m.cpu.reg_set(2, 0x11F); // request length
        m.exec_slot(0xE4, CODE_BASE + 0xC).unwrap();
        assert_eq!(m.cpu.reg_get(0), 0x11F, "all bytes must be reported sent");
        assert_eq!(m.cpu.reg_get(15), CODE_BASE + 0xC, "resume at the epilogue");
    }

    /// Builds an image whose app handler accumulates every delivered
    /// selector into a word in the platform region. Two relocations set up
    /// the app block and its handler entry; the recording address is an
    /// absolute platform address and needs no relocation.
    fn build_recording_image() -> Vec<u8> {
        const RECORD_ADDR: u32 = 0x0054_1000;
        let words: [u32; 18] = [
            0xE59F_2008, // 0x00: ldr r2, [pc, #8] -> literal at 0x10
            0xE581_2000, // 0x04: str r2, [r1]     ; register the app block
            0xE12F_FF1E, // 0x08: bx lr
            0xE1A0_0000, // 0x0C: nop
            0x0000_0034, // 0x10: app block offset (relocated)
            0xE59F_2010, // 0x14: handler: ldr r2, [pc, #0x10] -> literal at 0x2C
            0xE592_3000, // 0x18: ldr r3, [r2]
            0xE083_3001, // 0x1C: add r3, r3, r1  ; r1 is the selector
            0xE582_3000, // 0x20: str r3, [r2]
            0xE12F_FF1E, // 0x24: bx lr
            0xE1A0_0000, // 0x28: nop
            RECORD_ADDR, // 0x2C: recording word
            0xE1A0_0000, // 0x30: nop
            0x0000_0014, // 0x34: app block word 0 -> handler (relocated)
            0xE1A0_0000, // 0x38: nop
            0xE1A0_0000, // 0x3C: nop
            0xE1A0_0000, // 0x40: padding (root pointer planted at end - 8)
            0xE1A0_0000, // 0x44: padding
        ];
        let mut code = Vec::new();
        for w in words {
            code.extend_from_slice(&w.to_le_bytes());
        }
        let mut v = Vec::new();
        v.extend_from_slice(&0x1941_4653u32.to_le_bytes());
        v.extend_from_slice(&0u32.to_le_bytes()); // ver
        v.extend_from_slice(&0u32.to_le_bytes()); // reserved
        v.extend_from_slice(&0u32.to_le_bytes()); // reserved
        v.extend_from_slice(&0x20u32.to_le_bytes()); // hdr_len
        v.extend_from_slice(&(code.len() as u32).to_le_bytes()); // c0
        v.extend_from_slice(&0u32.to_le_bytes()); // c1
        v.extend_from_slice(&2u32.to_le_bytes()); // reloc count
        v.extend_from_slice(&code);
        v.extend_from_slice(&0x10u32.to_le_bytes());
        v.extend_from_slice(&0x34u32.to_le_bytes());
        v
    }

    /// The SDK gates its screen state machine on the app-activate/resume
    /// event. Without the per-frame pump a title draws its boot splash once
    /// and never repaints; with it, selector 3 reaches the guest each frame.
    #[test]
    fn frame_pump_delivers_the_resume_event() {
        const PROBE: u32 = 0x0054_1000;
        let mut m = SnowFishMachine::load(build_recording_image(), &[]).unwrap();
        m.boot(2000).unwrap();
        m.mem.write_u32(PROBE, 0).unwrap();
        m.events.events.clear();
        m.run_frame(2000).unwrap();
        // selector 3 (pump) + selector 10 (bare frame tick).
        assert_eq!(m.mem.read_u32(PROBE).unwrap(), 13);

        m.set_app_pump(false);
        m.mem.write_u32(PROBE, 0).unwrap();
        m.events.events.clear();
        m.run_frame(2000).unwrap();
        assert_eq!(m.mem.read_u32(PROBE).unwrap(), 10);
    }

    /// The offline responder must leave every network slot on the generic
    /// stub until it is explicitly enabled.
    #[test]
    fn network_slots_stay_generic_when_the_responder_is_off() {
        let mut m = SnowFishMachine::load(build_test_image(), &[]).unwrap();
        for slot in [0xC8, 0xE0, 0xEC, 0x570] {
            let w = Wrapper {
                slot,
                hook: CODE_BASE,
                ret: CODE_BASE + 0xC,
            };
            assert!(m.hooked_slot_ret(&w).is_none(), "+{slot:#x} stays native");
        }
        m.set_offline_http(true);
        for slot in [0xC8, 0xE0, 0xEC, 0x570] {
            let w = Wrapper {
                slot,
                hook: CODE_BASE,
                ret: CODE_BASE + 0xC,
            };
            assert!(m.hooked_slot_ret(&w).is_some(), "+{slot:#x} is served");
        }
    }

    /// +0xE8 registers the socket object with the asynchronous notifier and
    /// +0xEC looks it back up. Returning zero (the generic stub) made the
    /// SDK's event dispatcher drop every asynchronous socket event.
    #[test]
    fn offline_socket_notifier_looks_up_the_attached_object() {
        let mut m = SnowFishMachine::load(build_test_image(), &[]).unwrap();
        m.set_offline_http(true);
        m.cpu.reg_set(0, 7); // handle
        m.cpu.reg_set(1, 0x0061_429C); // socket object
        m.exec_slot(0xE8, CODE_BASE + 0xC).unwrap();
        m.cpu.reg_set(0, 7);
        m.exec_slot(0xEC, CODE_BASE + 0xC).unwrap();
        assert_eq!(m.cpu.reg_get(0), 0x0061_429C);
        m.cpu.reg_set(0, 8); // a different handle
        m.exec_slot(0xEC, CODE_BASE + 0xC).unwrap();
        assert_eq!(m.cpu.reg_get(0), 0);
    }

    /// The socket read path must return the full HTTP response and its byte
    /// count, then report EOF once the response has been consumed.
    #[test]
    fn offline_http_read_serves_a_complete_response() {
        const BUF: u32 = 0x0054_1200;
        let mut m = SnowFishMachine::load(build_test_image(), &[]).unwrap();
        m.set_offline_http(true);
        m.cpu.reg_set(0, 0);
        m.cpu.reg_set(1, BUF);
        m.cpu.reg_set(2, 0x800);
        m.exec_slot(0xE0, CODE_BASE + 0xC).unwrap();
        let n = m.cpu.reg_get(0) as usize;
        assert!(n > 32);
        let mut bytes = vec![0u8; n];
        m.mem.read(BUF, &mut bytes).unwrap();
        assert!(bytes.starts_with(b"HTTP/1.0 200 OK\r\n"));
        assert!(bytes.windows(4).any(|w| w == b"\r\n\r\n"));
        m.cpu.reg_set(1, BUF);
        m.cpu.reg_set(2, 0x800);
        m.exec_slot(0xE0, CODE_BASE + 0xC).unwrap();
        assert_eq!(m.cpu.reg_get(0), 0, "HTTP/1.0 closes after the response");
    }

    /// +0x570 hands the platform an async request descriptor
    /// `[status, callback, context]`. Once the response has been consumed
    /// the platform marks the request done and returns the guest callback
    /// so the SDK's own result parser runs.
    #[test]
    fn offline_http_completion_notifies_the_registered_callback() {
        const REQ: u32 = 0x0054_1300;
        let mut m = SnowFishMachine::load(build_test_image(), &[]).unwrap();
        m.set_offline_http(true);
        m.mem.write_u32(REQ, u32::MAX).unwrap();
        m.mem.write_u32(REQ + 4, 0x0011_76C0).unwrap();
        m.mem.write_u32(REQ + 8, 0x2A).unwrap();
        m.cpu.reg_set(0, REQ);
        m.exec_slot(0x570, CODE_BASE + 0xC).unwrap();
        assert!(m.deliver_http_completion().is_none(), "no response yet");
        m.cpu.reg_set(1, 0x0054_1400);
        m.cpu.reg_set(2, 0x800);
        m.exec_slot(0xE0, CODE_BASE + 0xC).unwrap();
        assert_eq!(m.deliver_http_completion(), Some((0x0011_76C0, 0x2A)));
        assert_eq!(m.mem.read_u32(REQ).unwrap(), 0, "status is marked done");
        assert!(m.deliver_http_completion().is_none(), "notify only once");
    }

    /// The SDK issues more than one HTTP request per session and reuses the
    /// same descriptor address. Registering a new request after the previous
    /// response was consumed must serve a fresh response and deliver a
    /// second completion instead of an immediate EOF.
    #[test]
    fn offline_http_serves_each_registered_transaction() {
        const REQ: u32 = 0x0054_1300;
        const BUF: u32 = 0x0054_1400;
        let mut m = SnowFishMachine::load(build_test_image(), &[]).unwrap();
        m.set_offline_http(true);
        m.mem.write_u32(REQ, u32::MAX).unwrap();
        m.mem.write_u32(REQ + 4, 0x0011_76C0).unwrap();
        m.mem.write_u32(REQ + 8, 0x2A).unwrap();

        let transact = |m: &mut SnowFishMachine| {
            m.cpu.reg_set(0, REQ);
            m.exec_slot(0x570, CODE_BASE + 0xC).unwrap();
            m.cpu.reg_set(1, BUF);
            m.cpu.reg_set(2, 0x800);
            m.exec_slot(0xE0, CODE_BASE + 0xC).unwrap();
            m.cpu.reg_get(0) as usize
        };

        let first = transact(&mut m);
        assert!(first > 32);
        assert_eq!(m.deliver_http_completion(), Some((0x0011_76C0, 0x2A)));
        m.mem.write_u32(REQ, u32::MAX).unwrap();
        let second = transact(&mut m);
        assert_eq!(second, first, "the second request gets a full response");
        assert_eq!(
            m.deliver_http_completion(),
            Some((0x0011_76C0, 0x2A)),
            "the second transaction reports completion too"
        );
    }

    /// Restoring a state keeps the region allocations stable: pointers
    /// handed out before the restore still address the same guest memory.
    #[test]
    fn save_state_restore_keeps_region_pointers_valid() {
        let mut machine = SnowFishMachine::load(build_test_image(), &[]).unwrap();
        machine.boot(1000).unwrap();
        let ptr = machine
            .mem
            .region_pointer(crate::consts::CODE_REGION_BASE)
            .unwrap();
        let blob = machine.serialize();
        machine.deserialize(&blob).unwrap();
        let ptr_after = machine
            .mem
            .region_pointer(crate::consts::CODE_REGION_BASE)
            .unwrap();
        assert_eq!(ptr, ptr_after);
    }
}
