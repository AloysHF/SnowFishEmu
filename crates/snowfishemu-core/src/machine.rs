//! Emulator machine: package loading, wrapper dispatch, and the guest ABI.
//!
//! The guest reaches platform services through compiler wrapper stublets
//! scanned out of the code image (see [`scan_wrappers`]); hooked slots run
//! natively here and resume at the wrapper epilogue, everything else
//! executes through the planted root-table stubs unchanged.

use crate::consts::{CODE_BASE, HEAP_BASE, STACK_TOP, STOP_ADDRESS};
use crate::cpu::ArmCpu;
use crate::gdi::Gdi;
use crate::lz;
use crate::mem::GuestMem;
use crate::plat::{self, APP_RECORD_ADDR};
use crate::sfa::SfaImage;
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
        if out.iter().any(|e: &Wrapper| e.slot == slot) {
            continue;
        }
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

/// One open file slot served from container bytes.
#[derive(Debug, Default, Clone, serde::Serialize, serde::Deserialize)]
pub(crate) struct FileSlot {
    pub(crate) in_use: bool,
    pub(crate) offset: usize,
    pub(crate) size: usize,
    pub(crate) name: String,
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
    wrappers: Vec<Wrapper>,
    handler: u32,
    file_slots: Vec<FileSlot>,
    container: Vec<u8>,
    res_data: Option<Vec<u8>>,
    events: EventQueue,
    frame_count: u64,
    lz_calls: u64,
    trace: bool,
    last_pc: u32,
    branch_log: Vec<(u32, u32)>,
    gdi: Gdi,
}

impl SnowFishMachine {
    /// Loads a package (the whole `.app` container) and prepares the guest.
    pub fn load(container: Vec<u8>, res_data: Option<Vec<u8>>) -> Result<Self, MachineError> {
        let image = SfaImage::parse(&container)?;
        let mut mem = GuestMem::new(image.header.image_size());
        let image_size = image.header.image_size();
        mem.write(CODE_BASE, &image.code)?;
        plat::layout(&mut mem, CODE_BASE, image_size);
        let wrappers = scan_wrappers(&image.code);
        info!(
            "loaded .app: image 0x{image_size:X}, {} wrappers: {:?}",
            wrappers.len(),
            wrappers
                .iter()
                .map(|w| format!("+0x{:X}", w.slot))
                .collect::<Vec<_>>()
        );
        let trace = std::env::var_os("SF_TRACE").is_some();
        Ok(Self {
            mem,
            cpu: ArmCpu::new(),
            code_base: CODE_BASE,
            wrappers,
            handler: 0,
            file_slots: vec![FileSlot::default(); 4],
            container,
            res_data,
            events: EventQueue::default(),
            frame_count: 0,
            lz_calls: 0,
            trace,
            last_pc: 0,
            branch_log: Vec::new(),
            gdi: Gdi::new(),
        })
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
        match w.slot {
            0x7C | 0x58 | 0x5C | 0x64 | 0x68 | 0x108 => Some(w.ret),
            0x12C | 0x154 | 0x198 | 0x194 | 0x184 | 0x15C | 0x010 => Some(w.ret),
            _ => None,
        }
    }

    /// Runs the guest from the current PC until the stop sentinel, the
    /// instruction budget, or a fault. Handles wrapper interception.
    fn run_until_return(&mut self, budget: u64) -> Result<u64, MachineError> {
        let mut executed = 0u64;
        while executed < budget {
            let pc = self.cpu.pc();
            let aligned = if self.cpu.thumb_mode() {
                pc & !1
            } else {
                pc & !3
            };
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
                if self.trace {
                    debug!(
                        "[plat] native slot +0x{:X} lr={:#010x} r0={:08x} r1={:08x}",
                        w.slot,
                        self.cpu.reg_get(armv4t_emu::reg::LR),
                        self.cpu.reg_get(0),
                        self.cpu.reg_get(1)
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
            executed += 1;
        }
        Ok(executed)
    }

    /// Executes one hooked platform slot natively and resumes at `ret`.
    fn exec_slot(&mut self, slot: u32, ret: u32) -> Result<(), MachineError> {
        let r = |i: u8| self.cpu.reg_get(i);
        let result = match slot {
            0x7C => {
                let msg = if r(0) > 0x1000 {
                    guest_read_string(&self.mem, r(0), 128)?
                } else {
                    format!("fmt@0x{:X}", r(0))
                };
                if self.trace {
                    info!("[plat] +0x7C logfmt=\"{msg}\" -> 0");
                } else {
                    debug!("[plat] +0x7C logfmt=\"{msg}\" -> 0");
                }
                Some(0u32)
            }
            0x58 => Some(self.hook_fopen(r(0), r(1))?),
            0x5C => Some(self.hook_fread(r(0), r(1), r(2))?),
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
            0x12C | 0x154 | 0x198 | 0x194 | 0x184 | 0x15C | 0x010 => {
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

    /// Executes one hooked GDI slot. Stack args follow the reference's
    /// `SP + adj + 4*n` layout; r0 stays untouched.
    fn exec_gdi(&mut self, slot: u32, ret: u32) -> Result<(), MachineError> {
        let sp = self.cpu.reg_get(armv4t_emu::reg::SP);
        let r = |i: u8| self.cpu.reg_get(i);
        match slot {
            0x12C => {
                // fill(obj, x, y, w, h, color)
                let adj = self.sp_adjust(ret);
                let x = r(1) as i32;
                let y = r(2) as i32;
                let w = r(3) as i32;
                let h = self.stack_arg(sp, adj, 1)? as i32;
                let color = self.stack_arg(sp, adj, 2)?;
                if self.trace {
                    debug!("[gdi]+0x{slot:X} fill @{x},{y} {w}x{h} c={color:#010x}");
                }
                self.gdi.op_fill(&mut self.mem, x, y, w, h, color);
            }
            0x154 => {
                // text(obj, str u16le, x, y, font) — the font selector is
                // ignored (single embedded font set).
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
                let mut rgb24: u32 = 0xFFFFFF;
                if r(0) > 0x1000 {
                    let mut b3 = [0u8; 3];
                    if self.mem.read(r(0) + 0x28, &mut b3).is_ok() {
                        let v = (b3[0] as u32) << 16 | (b3[1] as u32) << 8 | b3[2] as u32;
                        if v != 0 {
                            rgb24 = v;
                        }
                    }
                }
                let color = crate::gdi::rgb888_to_565(rgb24);
                if self.trace {
                    let text: String = chars
                        .iter()
                        .take_while(|&&c| c != 0)
                        .filter_map(|&c| char::from_u32(c as u32))
                        .collect();
                    debug!(
                        "[gdi]+0x{slot:X} text @{x},{y} c={color:#06x} len={} \"{text}\"",
                        chars.len()
                    );
                }
                self.gdi.op_text(&mut self.mem, &chars, x, y, color);
            }
            0x198 => {
                // draw(obj, bmp, w, h, _, dx, dy)
                let adj = self.sp_adjust(ret);
                let bmp_ptr = r(1);
                let w = self.stack_arg(sp, adj, 1)? as i32;
                let h = self.stack_arg(sp, adj, 2)? as i32;
                let dx = self.stack_arg(sp, adj, 4)? as i32;
                let dy = self.stack_arg(sp, adj, 5)? as i32;
                if self.trace {
                    debug!("[gdi]+0x198 draw bmp@{bmp_ptr:#x} clip {w}x{h} @({dx},{dy})");
                }
                if bmp_ptr <= 0x1000 || w == 0 || h == 0 || w > 0x400 || h > 0x400 {
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
                if self.trace {
                    debug!(
                        "[gdi] draw desc {bmp_ptr:#x}: sw={sw} sh={sh} key={key:#x} pal={pal:#x} data={data:#x}"
                    );
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
            0x15C => {
                // fill2(obj, x, y, w, h) — color from the canvas object.
                let adj = self.sp_adjust(ret);
                let x = r(1) as i32;
                let y = r(2) as i32;
                let w = r(3) as i32;
                let h = self.stack_arg(sp, adj, 1)? as i32;
                let mut color = self.gdi.fill_color();
                if r(0) > 0x1000 {
                    match self.mem.read_u16(r(0) + 0x2C) {
                        Ok(c16) if c16 != 0 => color = c16,
                        _ => {
                            let mut b3 = [0u8; 3];
                            if self.mem.read(r(0) + 0x28, &mut b3).is_ok() {
                                let v = (b3[0] as u32) << 16 | (b3[1] as u32) << 8 | b3[2] as u32;
                                if v != 0 {
                                    color = crate::gdi::rgb888_to_565(v);
                                }
                            }
                        }
                    }
                }
                if self.trace {
                    debug!("[gdi]+0x{slot:X} fill2 @{x},{y} {w}x{h} c={color:#06x}");
                }
                self.gdi.op_fill2(&mut self.mem, x, y, w, h, color);
            }
            0x010 => {
                self.gdi.op_flush(&self.mem);
            }
            _ => {}
        }
        Ok(())
    }

    /// `fopen(utf16_path, flags)` — refuses platform vfiles, otherwise
    /// serves container bytes (or the sibling resource file for `res`/
    /// `.pak` paths). Handles are 1..4.
    fn hook_fopen(&mut self, path_ptr: u32, flags: u32) -> Result<u32, MachineError> {
        let path = if path_ptr > 0x1000 {
            guest_utf16_to_ascii(&self.mem, path_ptr, 96)?
        } else {
            String::new()
        };
        let free = self.file_slots.iter().position(|s| !s.in_use);
        if let Some(kind) = vfile_kind(basename(&path)) {
            debug!("[plat] +0x58 fopen \"{path}\" flags={flags} -> 0 (vfile/{kind}, refused)");
            return Ok(0);
        }
        let Some(idx) = free else {
            debug!("[plat] +0x58 fopen \"{path}\" flags={flags} -> 0 (table full)");
            return Ok(0);
        };
        let lower = path.to_ascii_lowercase();
        let special = self
            .res_data
            .as_ref()
            .is_some_and(|_| !path.is_empty() && (lower.contains(".pak") || lower.contains("res")));
        let (data, size) = match (&self.res_data, special) {
            (Some(res), true) => (0u8, res.len()),
            _ => (1u8, self.container.len()),
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
        };
        debug!(
            "[plat] +0x58 fopen \"{path}\" flags={flags} -> h={} (size={size}{})",
            idx + 1,
            if data == 0 { " RES" } else { "" }
        );
        Ok(idx as u32 + 1)
    }

    /// `fread(handle, guest_buf, len)` — copies container bytes.
    fn hook_fread(&mut self, handle: u32, buf: u32, len: u32) -> Result<u32, MachineError> {
        if !(1..=4).contains(&handle) || !self.file_slots[(handle - 1) as usize].in_use {
            debug!("[plat] +0x5C fread h={handle} buf={buf:#x} size={len} -> 0 (bad handle)");
            return Ok(0);
        }
        let idx = (handle - 1) as usize;
        let avail = self.file_slots[idx]
            .size
            .saturating_sub(self.file_slots[idx].offset);
        let n = (len as usize).min(avail);
        if n > 0 && buf > 0x1000 {
            let src_off = self.file_slots[idx].offset;
            let uses_res = self.res_data.is_some()
                && self.file_slots[idx]
                    .name
                    .to_ascii_lowercase()
                    .contains("res");
            let data: &[u8] = if uses_res {
                &self.res_data.as_deref().unwrap()[src_off..src_off + n]
            } else {
                &self.container[src_off..src_off + n]
            };
            self.mem.write(buf, data)?;
        }
        self.file_slots[idx].offset += n;
        debug!(
            "[plat] +0x5C fread h={handle} buf={buf:#x} size={len} -> {n} (pos={}/{})",
            self.file_slots[idx].offset, self.file_slots[idx].size
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
    /// start (1) and first-tick (3) calls, mirroring the platform's boot
    /// sequence.
    pub fn boot(&mut self, budget: u64) -> Result<(), MachineError> {
        self.app_call(-1, 0, budget)?;
        if self.handler == 0 {
            warn!("boot completed without a registered handler");
        } else {
            info!("guest handler registered at {:#010x}", self.handler);
        }
        self.app_call(1, 0, budget)?;
        self.app_call(3, 0, budget)?;
        Ok(())
    }

    /// Drives one guest call: init (selector -1) or an event dispatch.
    pub fn app_call(&mut self, selector: i32, param: u32, budget: u64) -> Result<(), MachineError> {
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

    /// Advances one frame: pump queued events, deliver the frame tick
    /// (selector 10, the host paces at ~20 fps), count the frame.
    ///
    /// The platform exposes no timer service: slots like +0x500 are
    /// passthrough stubs and the guest is only driven by the host's tick
    /// and key events (matching the reference session logs).
    pub fn run_frame(&mut self, budget: u64) -> Result<(), MachineError> {
        self.pump(budget)?;
        self.app_call(10, 0, budget)?;
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

    /// Content fingerprint over the container and resource bytes; save
    /// states carry it so they are only restored into matching content.
    fn content_fingerprint(&self) -> u64 {
        let res: &[u8] = self.res_data.as_deref().unwrap_or(&[]);
        crate::save::content_fingerprint([&self.container, res])
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
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let mut m = SnowFishMachine::load(container, None).unwrap();
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

    /// Save states round-trip the observable runtime state: frame count,
    /// handler, and guest memory survive serialize -> deserialize.
    #[test]
    fn save_state_round_trips_runtime_state() {
        let container = build_test_image();
        let mut machine = SnowFishMachine::load(container, None).unwrap();
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
        let mut machine = SnowFishMachine::load(build_test_image(), None).unwrap();
        machine.boot(1000).unwrap();
        let blob = machine.serialize();

        // Flip a container byte to fake a different package.
        let mut other = SnowFishMachine::load(build_test_image(), None).unwrap();
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

    /// Restoring a state keeps the region allocations stable: pointers
    /// handed out before the restore still address the same guest memory.
    #[test]
    fn save_state_restore_keeps_region_pointers_valid() {
        let mut machine = SnowFishMachine::load(build_test_image(), None).unwrap();
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
