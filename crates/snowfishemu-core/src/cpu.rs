//! CPU execution loop.
//!
//! Drives the pure-Rust ARMv4T interpreter and completes the ARMv5TE
//! instruction set the applications actually use: interworking `BLX`
//! (immediate ARM form and Thumb register form) and the DSP `SMLAxy`
//! multiply family. Instructions the base interpreter handles go straight
//! through; anything handled here is evaluated with the standard condition
//! code against CPSR first.

use armv4t_emu::{reg, Cpu, Memory, Mode};
use log::debug;

use crate::consts::STOP_ADDRESS;
use crate::mem::{GuestMem, MemError};

/// Bridge between the interpreter and the guest memory map.
///
/// Interpreter memory hooks cannot fail, so unmapped accesses read as zero,
/// drop writes, and are counted for diagnostics.
struct MemAdapter<'a> {
    mem: &'a mut GuestMem,
    read_faults: u64,
    write_faults: u64,
}

impl<'a> MemAdapter<'a> {
    fn new(mem: &'a mut GuestMem) -> Self {
        Self {
            mem,
            read_faults: 0,
            write_faults: 0,
        }
    }
}

impl Memory for MemAdapter<'_> {
    fn r8(&mut self, addr: u32) -> u8 {
        self.mem.read_u8(addr).unwrap_or_else(|e| {
            self.read_faults += 1;
            debug!("cpu read fault @ {addr:#010x}: {e}");
            0
        })
    }
    fn r16(&mut self, addr: u32) -> u16 {
        self.mem.read_u16(addr).unwrap_or_else(|e| {
            self.read_faults += 1;
            debug!("cpu read fault @ {addr:#010x}: {e}");
            0
        })
    }
    fn r32(&mut self, addr: u32) -> u32 {
        self.mem.read_u32(addr).unwrap_or_else(|e| {
            self.read_faults += 1;
            debug!("cpu read fault @ {addr:#010x}: {e}");
            0
        })
    }
    fn w8(&mut self, addr: u32, val: u8) {
        if let Err(MemError::Unmapped(a)) = self.mem.write_u8(addr, val) {
            self.write_faults += 1;
            debug!("cpu write fault @ {a:#010x}");
        }
    }
    fn w16(&mut self, addr: u32, val: u16) {
        if let Err(MemError::Unmapped(a)) = self.mem.write_u16(addr, val) {
            self.write_faults += 1;
            debug!("cpu write fault @ {a:#010x}");
        }
    }
    fn w32(&mut self, addr: u32, val: u32) {
        if let Err(MemError::Unmapped(a)) = self.mem.write_u32(addr, val) {
            self.write_faults += 1;
            debug!("cpu write fault @ {a:#010x}");
        }
    }
}

/// Whether an ARM condition code passes against the standard N/Z/C/V flags.
fn arm_condition_met(condition: u8, cpsr: u32) -> bool {
    let n = cpsr & (1 << 31) != 0;
    let z = cpsr & (1 << 30) != 0;
    let c = cpsr & (1 << 29) != 0;
    let v = cpsr & (1 << 28) != 0;
    match condition {
        0 => z,
        1 => !z,
        2 => c,
        3 => !c,
        4 => n,
        5 => !n,
        6 => v,
        7 => !v,
        8 => c && !z,
        9 => !c || z,
        10 => n == v,
        11 => n != v,
        12 => !z && n == v,
        13 => z || n != v,
        _ => true,
    }
}

/// The result of one step: whether the guest reached the stop sentinel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepOutcome {
    /// Continue stepping.
    Running,
    /// PC reached [`STOP_ADDRESS`]; the guest call returned.
    Returned,
}

/// ARM execution bridge over the guest memory map.
#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct ArmCpu {
    cpu: Cpu,
    read_faults: u64,
    write_faults: u64,
}

impl ArmCpu {
    pub fn new() -> Self {
        Self {
            cpu: Cpu::new(),
            read_faults: 0,
            write_faults: 0,
        }
    }

    /// Reads a guest register (user-bank view).
    pub fn reg_get(&self, r: u8) -> u32 {
        self.cpu.reg_get(Mode::User, r)
    }

    /// Writes a guest register (user-bank view).
    pub fn reg_set(&mut self, r: u8, v: u32) {
        self.cpu.reg_set(Mode::User, r, v);
    }

    /// Whether the core is in Thumb state.
    pub fn thumb_mode(&self) -> bool {
        self.cpu.thumb_mode()
    }

    /// Address of the instruction about to execute.
    pub fn pc(&self) -> u32 {
        self.reg_get(reg::PC)
    }

    /// Executes a single instruction (with ARMv5TE pre-decode). Returns
    /// false when the core halted (undefined instruction).
    pub fn step_once(&mut self, mem: &mut GuestMem) -> bool {
        let pc = self.pc();
        let aligned = if self.thumb_mode() { pc & !1 } else { pc & !3 };
        if aligned != pc {
            self.reg_set(reg::PC, aligned);
        }
        let mut adapter = MemAdapter::new(mem);
        let ok = if self.step_predecoded(&mut adapter, aligned) {
            true
        } else {
            self.cpu.step(&mut adapter)
        };
        self.read_faults += adapter.read_faults;
        self.write_faults += adapter.write_faults;
        ok
    }

    /// Runs until the guest returns to [`STOP_ADDRESS`] or the instruction
    /// budget runs out. Returns the number of instructions executed.
    pub fn run(&mut self, mem: &mut GuestMem, budget: u64) -> (u64, StepOutcome) {
        let mut adapter = MemAdapter::new(mem);
        let mut executed: u64 = 0;
        let outcome = loop {
            if executed >= budget {
                break StepOutcome::Running;
            }
            let pc = self.pc();
            let aligned = if self.thumb_mode() { pc & !1 } else { pc & !3 };
            if aligned != pc {
                self.reg_set(reg::PC, aligned);
            }
            if aligned == STOP_ADDRESS {
                break StepOutcome::Returned;
            }
            if self.step_predecoded(&mut adapter, aligned) || self.cpu.step(&mut adapter) {
                executed += 1;
            } else {
                // Interpreter-level halt; treat as a stop so callers can
                // surface diagnostics instead of spinning.
                break StepOutcome::Returned;
            }
        };
        self.read_faults += adapter.read_faults;
        self.write_faults += adapter.write_faults;
        (executed, outcome)
    }

    /// Executes instructions the base ARMv4T interpreter lacks. Returns true
    /// when `pc` held one of them.
    fn step_predecoded(&mut self, mem: &mut MemAdapter<'_>, pc: u32) -> bool {
        if self.thumb_mode() {
            // Thumb BLX register (0100 1111 1 rm 000).
            let insn = mem.r16(pc);
            if insn & 0xFF87 == 0x4780 {
                let rm = ((insn >> 3) & 0x0F) as u8;
                let target = self.reg_get(rm);
                self.reg_set(reg::LR, pc.wrapping_add(3));
                self.set_interworked_target(target);
                return true;
            }
            // Thumb BLX immediate (F000 E800 prefix/suffix pair) switches to
            // ARM at pc+4+offset.
            let prefix = insn;
            if prefix & 0xF800 == 0xF000 {
                let suffix = mem.r16(pc + 2);
                if suffix & 0xF800 == 0xE800 {
                    let raw = (((prefix & 0x07FF) as u32) << 12) | ((suffix & 0x07FF) as u32) << 1;
                    let offset = ((raw << 9) as i32 >> 9) as u32;
                    let target = pc.wrapping_add(4).wrapping_add(offset) & !3;
                    self.reg_set(reg::LR, pc.wrapping_add(5));
                    self.reg_set(reg::PC, target);
                    let cpsr = self.reg_get(reg::CPSR) & !(1 << 5);
                    self.reg_set(reg::CPSR, cpsr);
                    return true;
                }
            }
            return false;
        }
        let insn = mem.r32(pc);
        if insn & 0x0FFF_FFF0 == 0x012F_FF30 {
            // ARM BLX register.
            let rm = (insn & 0x0F) as u8;
            let target = self.reg_get(rm);
            if arm_condition_met(((insn >> 28) & 0xF) as u8, self.reg_get(reg::CPSR)) {
                self.reg_set(reg::LR, pc.wrapping_add(4));
                self.set_interworked_target(target);
            } else {
                self.reg_set(reg::PC, pc.wrapping_add(4));
            }
            return true;
        }
        if insn & 0xFE00_0000 == 0xFA00_0000 {
            // ARM BLX immediate: target = pc + 8 + sext24*4 + H*2, Thumb.
            let imm24 = insn & 0x00FF_FFFF;
            let offset = (((imm24 << 8) as i32 >> 8) as u32) << 2;
            let h = (insn >> 24) & 1;
            let target = pc.wrapping_add(8).wrapping_add(offset).wrapping_add(h << 1);
            self.reg_set(reg::LR, pc.wrapping_add(4));
            self.reg_set(reg::PC, target & !1);
            let cpsr = self.reg_get(reg::CPSR) | (1 << 5);
            self.reg_set(reg::CPSR, cpsr);
            return true;
        }
        if self.step_dsp_multiply(pc, insn) {
            return true;
        }
        false
    }

    /// Applies a branch target with interworking (state bit from address).
    fn set_interworked_target(&mut self, target: u32) {
        self.reg_set(reg::PC, target & !1);
        let mut cpsr = self.reg_get(reg::CPSR);
        if target & 1 != 0 {
            cpsr |= 1 << 5;
        } else {
            cpsr &= !(1 << 5);
        }
        self.reg_set(reg::CPSR, cpsr);
    }

    /// Handles the ARMv5TE DSP multiply family via the unarm reference
    /// decoder. Returns true when `pc` held one of these instructions.
    fn step_dsp_multiply(&mut self, pc: u32, insn: u32) -> bool {
        let ins = unarm::parse_arm(insn, pc, &unarm::Options::default());
        let handled = matches!(
            ins,
            unarm::Ins::Smla { .. }
                | unarm::Ins::Smul { .. }
                | unarm::Ins::Smlaw { .. }
                | unarm::Ins::Smulw { .. }
                | unarm::Ins::SmlalHalf { .. }
        );
        if !handled {
            return false;
        }
        if !arm_condition_met(((insn >> 28) & 0xF) as u8, self.reg_get(reg::CPSR)) {
            self.reg_set(reg::PC, pc.wrapping_add(4));
            return true;
        }
        let (rd, value) = match ins {
            unarm::Ins::Smla {
                rd,
                rn,
                rn_side,
                rm,
                rm_side,
                ra,
                ..
            } => {
                let prod = self.half(rn, rn_side) as i64 * self.half(rm, rm_side) as i64;
                (rd as u8, (prod + self.reg_get(ra as u8) as i64) as u32)
            }
            unarm::Ins::Smul {
                rd,
                rn,
                rn_side,
                rm,
                rm_side,
                ..
            } => {
                let prod = self.half(rn, rn_side) as i64 * self.half(rm, rm_side) as i64;
                (rd as u8, prod as u32)
            }
            unarm::Ins::Smlaw {
                rd,
                rn,
                rm,
                rm_side,
                ra,
                ..
            } => {
                let shifted =
                    (self.reg_get(rn as u8) as i32 as i64 * self.half(rm, rm_side) as i64) >> 16;
                (rd as u8, (self.reg_get(ra as u8) as i64 + shifted) as u32)
            }
            unarm::Ins::Smulw {
                rd,
                rn,
                rm,
                rm_side,
                ..
            } => {
                let shifted =
                    (self.reg_get(rn as u8) as i32 as i64 * self.half(rm, rm_side) as i64) >> 16;
                (rd as u8, shifted as i32 as u32)
            }
            unarm::Ins::SmlalHalf {
                rd_lo,
                rd_hi,
                rn,
                rn_side,
                rm,
                rm_side,
                ..
            } => {
                let acc =
                    ((self.reg_get(rd_hi as u8) as u64) << 32) | self.reg_get(rd_lo as u8) as u64;
                let prod = self.half(rn, rn_side) as i64 * self.half(rm, rm_side) as i64;
                let sum = (acc as i64).wrapping_add(prod) as u64;
                self.reg_set(rd_hi as u8, (sum >> 32) as u32);
                self.reg_set(rd_lo as u8, sum as u32);
                self.reg_set(reg::PC, pc.wrapping_add(4));
                return true;
            }
            _ => return false,
        };
        self.reg_set(rd, value);
        self.reg_set(reg::PC, pc.wrapping_add(4));
        true
    }

    /// Signed halfword of a register selected by `side`.
    fn half(&self, r: unarm::Reg, side: unarm::RegSide) -> i32 {
        let v = self.reg_get(r as u8);
        let h = match side {
            unarm::RegSide::Bottom => v as u16,
            unarm::RegSide::Top => (v >> 16) as u16,
        };
        (h as i16) as i32
    }
}

impl Default for ArmCpu {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::consts::CODE_BASE;

    /// Writes a tiny program at CODE_BASE and drives the bridge directly.
    fn run_program(words: &[u32], budget: u64) -> (GuestMem, u64, StepOutcome) {
        let mut mem = GuestMem::new(0x1000);
        let mut buf = Vec::with_capacity(words.len() * 4);
        for w in words {
            buf.extend_from_slice(&w.to_le_bytes());
        }
        mem.write(CODE_BASE, &buf).unwrap();
        let mut cpu = ArmCpu::new();
        cpu.reg_set(reg::PC, CODE_BASE);
        cpu.reg_set(reg::SP, crate::consts::STACK_TOP);
        cpu.reg_set(reg::LR, STOP_ADDRESS);
        cpu.reg_set(reg::CPSR, 0x10); // User mode, ARM, T=0: Mode::User bank is current
        let (n, outcome) = cpu.run(&mut mem, budget);
        (mem, n, outcome)
    }

    #[test]
    fn runs_until_stop_sentinel() {
        // mov r0,#42; bx lr
        let (_, n, outcome) = run_program(&[0xE3A0_002A, 0xE12F_FF1E], 100);
        assert_eq!(outcome, StepOutcome::Returned);
        assert_eq!(n, 2);
    }

    #[test]
    fn respects_instruction_budget() {
        // Infinite loop: branch to self.
        let (_, n, outcome) = run_program(&[0xEAFF_FFFE], 500);
        assert_eq!(outcome, StepOutcome::Running);
        assert_eq!(n, 500);
    }

    #[test]
    fn blx_immediate_enters_thumb_and_links() {
        // blx with imm24=1 targets pc+8+4 in Thumb mode and links pc+4.
        let blx = 0xFA00_0001;
        let words = [blx, 0xE1A0_0000, 0xE1A0_0000, 0xE1A0_0000];
        let (mem, _, _) = run_program(&words, 1);
        let mut cpu = ArmCpu::new();
        cpu.reg_set(reg::PC, CODE_BASE);
        cpu.reg_set(reg::LR, STOP_ADDRESS);
        cpu.reg_set(reg::CPSR, 0x10);
        let mut mem = mem;
        cpu.run(&mut mem, 1);
        assert!(cpu.thumb_mode());
        assert_eq!(cpu.pc(), CODE_BASE + 0xC);
        assert_eq!(cpu.reg_get(reg::LR), CODE_BASE + 4);
    }

    #[test]
    fn dsp_multiplies_match_semantics() {
        // Encodings follow the unarm reference layout: rd=[19:16],
        // ra=[15:12] (0 for SMUL forms), rn=[3:0] with side bit5,
        // rm=[11:8] with side bit6, opcode [27:20] with bit7=1 bit4=0.
        struct Case {
            insn: u32,
            regs: [(u8, u32); 3],
            expect: u32,
            name: &'static str,
        }
        let cases = vec![
            // SMULBB r0, r1, r2 = r1.b * r2.b
            Case {
                insn: 0xE160_0281,
                regs: [(0, 0), (1, 0x0003_0002), (2, 0x0005_0004)],
                expect: 2 * 4,
                name: "smulbb",
            },
            // SMLATT r0, r1, r2, r3 = r3 + r1.t * r2.t
            Case {
                insn: 0xE100_32E1,
                regs: [(1, 0x0003_0002), (2, 0x0005_0004), (3, 100)],
                expect: 100 + 3 * 5,
                name: "smlatt",
            },
            // SMULWT r0, r1, r2 = (r1 * r2.t) >> 16
            Case {
                insn: 0xE120_02C1,
                regs: [(1, 0x0002_0000), (2, 0x0008_0004), (0, 0)],
                expect: ((0x20000i64 * 8) >> 16) as u32,
                name: "smulwt",
            },
        ];
        for c in cases {
            let (mem, _, _) = run_program(&[c.insn, 0xE12F_FF1E], 10);
            let mut cpu = ArmCpu::new();
            cpu.reg_set(reg::PC, CODE_BASE);
            for (r, v) in c.regs {
                cpu.reg_set(r, v);
            }
            cpu.reg_set(reg::LR, STOP_ADDRESS);
            cpu.reg_set(reg::CPSR, 0x10);
            let mut mem = mem;
            cpu.run(&mut mem, 10);
            assert_eq!(cpu.reg_get(0), c.expect, "case {}", c.name);
        }
    }

    #[test]
    fn smlal_accumulates_into_pair() {
        // SMLALBB r2(hi):r3(lo) += r1.b * r4.b; then bx lr.
        // unarm layout: rd_hi=[19:16], rd_lo=[15:12], rn=[3:0] side bit5,
        // rm=[11:8] side bit6, opcode [27:20]=0x14 with bit7=1.
        let smlal = 0xE142_3481; // rd_hi=2, rd_lo=3, rm=4, rn=1, sides bottom
        let words = [smlal, 0xE12F_FF1E];
        let (mem, _, _) = run_program(&words, 10);
        let mut cpu = ArmCpu::new();
        cpu.reg_set(reg::PC, CODE_BASE);
        cpu.reg_set(1, 0x0003_0002); // rn low halfword = 2
        cpu.reg_set(4, 0x0005_0004); // rm low halfword = 4
        cpu.reg_set(2, 0x0000_0001); // RdHi = 1
        cpu.reg_set(3, 0xFFFF_FFFF); // RdLo = -1 (64-bit accumulator low)
                                     // Accumulator = 0x00000001_FFFFFFFF; product = 2 * 4 = 8.
        cpu.reg_set(reg::LR, STOP_ADDRESS);
        cpu.reg_set(reg::CPSR, 0x10);
        let mut mem = mem;
        cpu.run(&mut mem, 10);
        let acc = ((cpu.reg_get(2) as u64) << 32) | cpu.reg_get(3) as u64;
        assert_eq!(acc, (0x0000_0001_FFFF_FFFFu64).wrapping_add(8));
    }
}

#[cfg(test)]
mod disasm_probe {
    use unarm::{parse_arm, Options};

    #[test]
    fn probe_dsp_encodings() {
        for w in [
            0x3160_3aa8u32, // guest SMULxy mask match
            0x5162_aaa4,    // guest SMULxy mask match, [19:16]=2
            0x912a_4380,    // guest SMLAWy
            0x5145_7684,    // guest SMLALxy
        ] {
            let ins = parse_arm(w, 0, &Options::default());
            println!("{w:08x}: {:?}", ins);
        }
    }
}
