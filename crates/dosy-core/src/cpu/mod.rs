//! x86 real-mode CPU (8086 / 80186 / common 386 real-mode extensions).
//! Tasks 001 (alu), 002 (state, decode, exec), 007, 025.

pub mod alu;
pub mod decode;
mod exec;
pub mod flags;
pub mod regs;

#[cfg(test)]
mod tests;

pub use decode::{ModRm, Prefixes, Rep, Rm};
pub use exec::condition;
pub use regs::{Regs, Seg, SegRegs};

use crate::bus::{Bus, linear};
use crate::hle::HleId;

/// FLAGS / EFLAGS value; bit masks in [`flags`].
pub type Flags = u32;

/// FLAGS bits that POPF / IRET / SAHF may change (386 real mode, IOPL/NT
/// kept 0). Bits 12..15 read as 0, as on a 386 (an 8086 reads them as 1).
pub const FLAGS_WRITABLE: u32 = flags::CF
    | flags::PF
    | flags::AF
    | flags::ZF
    | flags::SF
    | flags::TF
    | flags::IF
    | flags::DF
    | flags::OF;

/// Outcome of one [`Cpu::step`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    Ok,
    /// HLT executed; `halted` is set until [`Cpu::interrupt`].
    Halt,
    /// Callback opcode `FE 38 lo hi`; IP is past the 4 bytes.
    Callback(HleId),
    /// IP is rewound to the start of the faulting instruction (incl. prefixes).
    Fault(CpuFault),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CpuFault {
    /// Unknown or unimplemented opcode at `cs:ip` (first byte after prefixes).
    InvalidOpcode { cs: u16, ip: u16, opcode: u8 },
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Cpu {
    pub regs: Regs,
    pub sregs: SegRegs,
    /// EIP; always < 0x10000 in 16-bit code.
    pub ip: u32,
    pub flags: Flags,
    pub halted: bool,
    /// Set by STI (when IF was 0), MOV SS and POP SS: the machine must not
    /// take a maskable interrupt before the next instruction.
    pub int_shadow: bool,
    /// IP of the last instruction started (incl. prefixes).
    pub insn_ip: u32,
}

impl Cpu {
    pub fn new() -> Self {
        let mut cpu = Self::default();
        cpu.reset();
        cpu
    }

    /// Power-on state: CS:IP = F000:FFF0, other registers 0.
    pub fn reset(&mut self) {
        *self = Self::default();
        self.sregs.set(Seg::Cs, 0xF000);
        self.ip = 0xFFF0;
        self.flags = flags::RESERVED1;
    }

    /// Real-mode interrupt through the IVT at linear 0: push FLAGS, CS, IP,
    /// clear IF/TF and jump. Also wakes a halted CPU.
    pub fn interrupt<B: Bus>(&mut self, bus: &mut B, vector: u8) {
        self.push16(bus, self.flags as u16);
        self.push16(bus, self.sregs.get(Seg::Cs));
        self.push16(bus, self.ip as u16);
        self.flags &= !(flags::IF | flags::TF);
        let v = vector as u32 * 4;
        let ip = bus.read16(v);
        let cs = bus.read16(v + 2);
        self.far_jump(cs, ip);
        self.halted = false;
    }

    #[inline]
    pub fn far_jump(&mut self, cs: u16, ip: u16) {
        self.sregs.set(Seg::Cs, cs);
        self.ip = ip as u32;
    }

    /// Low 16 bits of FLAGS as POPF/IRET/SAHF-style writes see them.
    pub fn set_flags16(&mut self, v: u16) {
        self.flags = (self.flags & !0xFFFF) | (v as u32 & FLAGS_WRITABLE) | flags::RESERVED1;
    }

    // --- Memory, with offsets wrapping inside the segment. ---

    #[inline]
    pub fn read_mem8<B: Bus>(&mut self, bus: &mut B, seg: Seg, off: u16) -> u8 {
        bus.read8(linear(self.sregs.get(seg), off))
    }
    #[inline]
    pub fn write_mem8<B: Bus>(&mut self, bus: &mut B, seg: Seg, off: u16, v: u8) {
        bus.write8(linear(self.sregs.get(seg), off), v)
    }
    pub fn read_mem16<B: Bus>(&mut self, bus: &mut B, seg: Seg, off: u16) -> u16 {
        let lo = self.read_mem8(bus, seg, off);
        let hi = self.read_mem8(bus, seg, off.wrapping_add(1));
        u16::from_le_bytes([lo, hi])
    }
    pub fn write_mem16<B: Bus>(&mut self, bus: &mut B, seg: Seg, off: u16, v: u16) {
        let [lo, hi] = v.to_le_bytes();
        self.write_mem8(bus, seg, off, lo);
        self.write_mem8(bus, seg, off.wrapping_add(1), hi);
    }

    pub fn read_rm8<B: Bus>(&mut self, bus: &mut B, rm: Rm) -> u8 {
        match rm {
            Rm::Reg(i) => self.regs.get8(i),
            Rm::Mem { seg, off } => self.read_mem8(bus, seg, off),
        }
    }
    pub fn write_rm8<B: Bus>(&mut self, bus: &mut B, rm: Rm, v: u8) {
        match rm {
            Rm::Reg(i) => self.regs.set8(i, v),
            Rm::Mem { seg, off } => self.write_mem8(bus, seg, off, v),
        }
    }
    pub fn read_rm16<B: Bus>(&mut self, bus: &mut B, rm: Rm) -> u16 {
        match rm {
            Rm::Reg(i) => self.regs.get16(i),
            Rm::Mem { seg, off } => self.read_mem16(bus, seg, off),
        }
    }
    pub fn write_rm16<B: Bus>(&mut self, bus: &mut B, rm: Rm, v: u16) {
        match rm {
            Rm::Reg(i) => self.regs.set16(i, v),
            Rm::Mem { seg, off } => self.write_mem16(bus, seg, off, v),
        }
    }

    // --- Stack (SS:SP, 16-bit, wraps at 0). ---

    pub fn push16<B: Bus>(&mut self, bus: &mut B, v: u16) {
        let sp = self.regs.sp().wrapping_sub(2);
        self.regs.set_sp(sp);
        self.write_mem16(bus, Seg::Ss, sp, v);
    }
    pub fn pop16<B: Bus>(&mut self, bus: &mut B) -> u16 {
        let sp = self.regs.sp();
        let v = self.read_mem16(bus, Seg::Ss, sp);
        self.regs.set_sp(sp.wrapping_add(2));
        v
    }
}
