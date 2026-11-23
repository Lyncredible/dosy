//! Post-8086 opcodes (task 025): 80186 additions, the 286 `0F 01` system
//! group in real mode, and `F1` (INT1). Reached from `exec` for opcodes the
//! 8086 table doesn't handle.

use super::alu;
use super::regs::{BP, SP, Seg};
use super::{Cpu, CpuModel, Prefixes, Rm, Step};
use crate::bus::Bus;

/// Interrupt vectors raised by these instructions.
const VEC_INT1: u8 = 1;
const VEC_BOUND: u8 = 5;
const VEC_UD: u8 = 6;

impl Cpu {
    /// `None` = invalid / not implemented (the caller faults).
    pub(super) fn exec_ext<B: Bus>(&mut self, bus: &mut B, op: u8, pfx: &Prefixes) -> Option<Step> {
        match op {
            0x0F => return self.exec_0f(bus, pfx),
            // PUSHA: AX CX DX BX SP(original) BP SI DI.
            0x60 => {
                let sp = self.regs.sp();
                for r in 0..8 {
                    let v = if r == SP { sp } else { self.regs.get16(r) };
                    self.push16(bus, v);
                }
            }
            // POPA: reverse order, the SP slot is discarded.
            0x61 => {
                for r in (0..8).rev() {
                    let v = self.pop16(bus);
                    if r != SP {
                        self.regs.set16(r, v);
                    }
                }
            }
            // BOUND r16, m16&16: INT 5 (return to BOUND) when out of range.
            0x62 => {
                let m = self.decode_modrm(bus, pfx);
                let Rm::Mem { seg, off } = m.rm else {
                    return Some(self.raise_fault(bus, VEC_UD));
                };
                let lo = self.read_mem16(bus, seg, off) as i16;
                let hi = self.read_mem16(bus, seg, off.wrapping_add(2)) as i16;
                let v = self.regs.get16(m.reg) as i16;
                if v < lo || v > hi {
                    return Some(self.raise_fault(bus, VEC_BOUND));
                }
            }
            0x68 => {
                let v = self.fetch16(bus);
                self.push16(bus, v);
            }
            0x6A => {
                let v = self.fetch8(bus) as i8 as u16;
                self.push16(bus, v);
            }
            // IMUL r16, r/m16, imm16 / imm8 (sign-extended).
            0x69 | 0x6B => {
                let m = self.decode_modrm(bus, pfx);
                let a = self.read_rm16(bus, m.rm);
                let b = if op == 0x69 {
                    self.fetch16(bus)
                } else {
                    self.fetch8(bus) as i8 as u16
                };
                let (p, f) = alu::imul16(a, b, self.flags);
                self.flags = f;
                self.regs.set16(m.reg, p as u16);
            }
            // INS / OUTS (string I/O at DX).
            0x6C..=0x6F => self.string_op(bus, op, pfx),
            // Shift group by imm8.
            0xC0 | 0xC1 => {
                let m = self.decode_modrm(bus, pfx);
                if op == 0xC0 {
                    let a = self.read_rm8(bus, m.rm);
                    let count = self.fetch8(bus);
                    let r = self.shift8(m.reg, a, count);
                    self.write_rm8(bus, m.rm, r);
                } else {
                    let a = self.read_rm16(bus, m.rm);
                    let count = self.fetch8(bus);
                    let r = self.shift16(m.reg, a, count);
                    self.write_rm16(bus, m.rm, r);
                }
            }
            0xC8 => {
                let size = self.fetch16(bus);
                let level = self.fetch8(bus) & 0x1F;
                self.enter16(bus, size, level);
            }
            // LEAVE: SP = BP; pop BP.
            0xC9 => {
                self.regs.set_sp(self.regs.get16(BP));
                let v = self.pop16(bus);
                self.regs.set16(BP, v);
            }
            // F1: INT1 / ICEBP on the 386 (a LOCK alias before, see `step`).
            0xF1 => self.interrupt(bus, VEC_INT1),
            _ => return None,
        }
        Some(Step::Ok)
    }

    /// ENTER size, level (16-bit stack frame).
    fn enter16<B: Bus>(&mut self, bus: &mut B, size: u16, level: u8) {
        self.push16(bus, self.regs.get16(BP));
        let frame = self.regs.sp();
        if level > 0 {
            for _ in 1..level {
                let bp = self.regs.get16(BP).wrapping_sub(2);
                self.regs.set16(BP, bp);
                let v = self.read_mem16(bus, Seg::Ss, bp);
                self.push16(bus, v);
            }
            self.push16(bus, frame);
        }
        self.regs.set16(BP, frame);
        self.regs.set_sp(self.regs.sp().wrapping_sub(size));
    }

    /// `0F` escape: POP CS on the 8086, #UD on the 186, two-byte opcodes on
    /// the 386.
    fn exec_0f<B: Bus>(&mut self, bus: &mut B, pfx: &Prefixes) -> Option<Step> {
        match self.model {
            CpuModel::I8086 => {
                let v = self.pop16(bus);
                self.sregs.set(Seg::Cs, v);
                return Some(Step::Ok);
            }
            CpuModel::I186 => return Some(self.raise_fault(bus, VEC_UD)),
            CpuModel::I386 => {}
        }
        let op2 = self.fetch8(bus);
        match op2 {
            0x01 => {
                let m = self.decode_modrm(bus, pfx);
                match (m.reg, m.rm) {
                    // SGDT / SIDT: limit + 24-bit base (+ 0 byte). Real mode:
                    // empty GDT, IVT at 0 with limit 3FFh.
                    (0 | 1, Rm::Mem { seg, off }) => {
                        let limit = if m.reg == 1 { 0x03FF } else { 0 };
                        self.write_mem16(bus, seg, off, limit);
                        self.write_mem16(bus, seg, off.wrapping_add(2), 0);
                        self.write_mem16(bus, seg, off.wrapping_add(4), 0);
                    }
                    // LGDT / LIDT: accepted and ignored (no protected mode;
                    // a moved real-mode IVT is not supported).
                    (2 | 3, Rm::Mem { .. }) => {}
                    // SMSW: MSW with PE=0 and no FPU.
                    (4, rm) => self.write_rm16(bus, rm, 0),
                    // LMSW: entering protected mode is unsupported.
                    (6, rm) => {
                        if self.read_rm16(bus, rm) & 1 != 0 {
                            return None;
                        }
                    }
                    _ => return Some(self.raise_fault(bus, VEC_UD)),
                }
                Some(Step::Ok)
            }
            _ => None,
        }
    }

    /// Fault-class exception: the return address is the faulting
    /// instruction (incl. prefixes).
    pub(super) fn raise_fault<B: Bus>(&mut self, bus: &mut B, vector: u8) -> Step {
        self.ip = self.insn_ip;
        self.interrupt(bus, vector);
        Step::Ok
    }
}
