//! 386 two-byte opcodes (`0F xx`) in real mode.

use super::alu;
use super::exec::condition;
use super::flags::{CF, OF, ZF};
use super::regs::Seg;
use super::{Cpu, Prefixes, Rm, Step};
use crate::bus::Bus;

/// BT / BTS / BTR / BTC.
#[derive(Clone, Copy)]
enum BitOp {
    Test,
    Set,
    Reset,
    Complement,
}

impl Cpu {
    /// `op2` is the byte after `0F`. `None` = invalid / not implemented.
    pub(super) fn exec_0f_386<B: Bus>(
        &mut self,
        bus: &mut B,
        op2: u8,
        pfx: &Prefixes,
    ) -> Option<Step> {
        let wide = pfx.opsize;
        match op2 {
            // CLTS: no task switching in real mode.
            0x06 => {}
            // MOV r32, CRn: CR0 reads PE=0, no FPU; CR2/CR3 read 0.
            0x20 => {
                let m = self.decode_modrm(bus, pfx);
                let Rm::Reg(r) = m.rm else { return None };
                if !matches!(m.reg, 0 | 2 | 3) {
                    return None;
                }
                self.regs.set32(r, 0);
            }
            // MOV CRn, r32: only a CR0 write without PE is accepted.
            0x22 => {
                let m = self.decode_modrm(bus, pfx);
                let Rm::Reg(r) = m.rm else { return None };
                if m.reg == 0 && self.regs.get32(r) & 1 != 0 {
                    return None;
                }
                if !matches!(m.reg, 0 | 2 | 3) {
                    return None;
                }
            }
            // Jcc rel16 / rel32.
            0x80..=0x8F => {
                let d = if wide {
                    self.fetch32(bus)
                } else {
                    self.fetch16(bus) as i16 as u32
                };
                if condition(self.flags, op2 & 0xF) {
                    self.ip = self.ip.wrapping_add(d) & 0xFFFF;
                }
            }
            // SETcc r/m8.
            0x90..=0x9F => {
                let m = self.decode_modrm(bus, pfx);
                let v = condition(self.flags, op2 & 0xF) as u8;
                self.write_rm8(bus, m.rm, v);
            }
            // PUSH / POP FS, GS.
            0xA0 | 0xA8 => {
                let s = if op2 == 0xA0 { Seg::Fs } else { Seg::Gs };
                let v = self.sregs.get(s);
                if wide {
                    self.push32(bus, v as u32);
                } else {
                    self.push16(bus, v);
                }
            }
            0xA1 | 0xA9 => {
                let s = if op2 == 0xA1 { Seg::Fs } else { Seg::Gs };
                let v = if wide {
                    self.pop32(bus) as u16
                } else {
                    self.pop16(bus)
                };
                self.sregs.set(s, v);
            }
            // BT/BTS/BTR/BTC r/m, r (memory forms address beyond the operand).
            0xA3 | 0xAB | 0xB3 | 0xBB => {
                let m = self.decode_modrm(bus, pfx);
                let op = match op2 {
                    0xA3 => BitOp::Test,
                    0xAB => BitOp::Set,
                    0xB3 => BitOp::Reset,
                    _ => BitOp::Complement,
                };
                let bit = self.regs.get32(m.reg);
                self.bit_op(bus, m.rm, bit, true, wide, op);
            }
            // BT/BTS/BTR/BTC r/m, imm8.
            0xBA => {
                let m = self.decode_modrm(bus, pfx);
                let bit = self.fetch8(bus) as u32;
                let op = match m.reg {
                    4 => BitOp::Test,
                    5 => BitOp::Set,
                    6 => BitOp::Reset,
                    7 => BitOp::Complement,
                    _ => return None,
                };
                self.bit_op(bus, m.rm, bit, false, wide, op);
            }
            // SHLD / SHRD by imm8 or CL.
            0xA4 | 0xA5 | 0xAC | 0xAD => {
                let m = self.decode_modrm(bus, pfx);
                let count = if op2 & 1 == 0 {
                    self.fetch8(bus)
                } else {
                    self.regs.cl()
                };
                let left = op2 < 0xAC;
                let bits = if wide { 32 } else { 16 };
                let dst = if wide {
                    self.read_rm32(bus, m.rm)
                } else {
                    self.read_rm16(bus, m.rm) as u32
                };
                let src = if wide {
                    self.regs.get32(m.reg)
                } else {
                    self.regs.get16(m.reg) as u32
                };
                let Some(r) = self.double_shift(dst, src, count, bits, left) else {
                    return Some(Step::Ok);
                };
                if wide {
                    self.write_rm32(bus, m.rm, r);
                } else {
                    self.write_rm16(bus, m.rm, r as u16);
                }
            }
            // IMUL r, r/m.
            0xAF => {
                let m = self.decode_modrm(bus, pfx);
                if wide {
                    let b = self.read_rm32(bus, m.rm);
                    let (p, f) = alu::imul32(self.regs.get32(m.reg), b, self.flags);
                    self.flags = f;
                    self.regs.set32(m.reg, p as u32);
                } else {
                    let b = self.read_rm16(bus, m.rm);
                    let (p, f) = alu::imul16(self.regs.get16(m.reg), b, self.flags);
                    self.flags = f;
                    self.regs.set16(m.reg, p as u16);
                }
            }
            // LSS / LFS / LGS.
            0xB2 | 0xB4 | 0xB5 => {
                let m = self.decode_modrm(bus, pfx);
                let s = match op2 {
                    0xB2 => Seg::Ss,
                    0xB4 => Seg::Fs,
                    _ => Seg::Gs,
                };
                if wide {
                    return self.load_far32(bus, m.reg, m.rm, s);
                }
                let Rm::Mem { seg, off } = m.rm else {
                    return None;
                };
                let v = self.read_mem16(bus, seg, off);
                let sel = self.read_mem16(bus, seg, off.wrapping_add(2));
                self.regs.set16(m.reg, v);
                self.sregs.set(s, sel);
                if s == Seg::Ss {
                    self.int_shadow = true;
                }
            }
            // MOVZX / MOVSX from r/m8 or r/m16.
            0xB6 | 0xB7 | 0xBE | 0xBF => {
                let m = self.decode_modrm(bus, pfx);
                let signed = op2 >= 0xBE;
                let v = if op2 & 1 == 0 {
                    let b = self.read_rm8(bus, m.rm);
                    if signed { b as i8 as u32 } else { b as u32 }
                } else {
                    let w = self.read_rm16(bus, m.rm);
                    if signed { w as i16 as u32 } else { w as u32 }
                };
                if wide {
                    self.regs.set32(m.reg, v);
                } else {
                    self.regs.set16(m.reg, v as u16);
                }
            }
            // BSF / BSR: ZF=1 and destination unchanged for a zero source.
            0xBC | 0xBD => {
                let m = self.decode_modrm(bus, pfx);
                let v = if wide {
                    self.read_rm32(bus, m.rm)
                } else {
                    self.read_rm16(bus, m.rm) as u32
                };
                if v == 0 {
                    self.flags |= ZF;
                } else {
                    self.flags &= !ZF;
                    let idx = if op2 == 0xBC {
                        v.trailing_zeros()
                    } else {
                        31 - v.leading_zeros()
                    };
                    if wide {
                        self.regs.set32(m.reg, idx);
                    } else {
                        self.regs.set16(m.reg, idx as u16);
                    }
                }
            }
            _ => return None,
        }
        Some(Step::Ok)
    }

    /// Bit-test family. `reg_offset`: the bit index came from a register,
    /// so for memory operands it may select another word/dword.
    fn bit_op<B: Bus>(
        &mut self,
        bus: &mut B,
        rm: Rm,
        bit: u32,
        reg_offset: bool,
        wide: bool,
        op: BitOp,
    ) {
        let bits: u32 = if wide { 32 } else { 16 };
        let rm = match rm {
            Rm::Mem { seg, off } if reg_offset => {
                let delta = if wide {
                    ((bit as i32) >> 5) * 4
                } else {
                    ((bit as u16 as i16 as i32) >> 4) * 2
                };
                Rm::Mem {
                    seg,
                    off: off.wrapping_add(delta as u16),
                }
            }
            rm => rm,
        };
        let mask = 1u32 << (bit & (bits - 1));
        let v = if wide {
            self.read_rm32(bus, rm)
        } else {
            self.read_rm16(bus, rm) as u32
        };
        self.flags = if v & mask != 0 {
            self.flags | CF
        } else {
            self.flags & !CF
        };
        let r = match op {
            BitOp::Test => return,
            BitOp::Set => v | mask,
            BitOp::Reset => v & !mask,
            BitOp::Complement => v ^ mask,
        };
        if wide {
            self.write_rm32(bus, rm, r);
        } else {
            self.write_rm16(bus, rm, r as u16);
        }
    }

    /// SHLD (`left`) / SHRD on `bits`-wide `dst` with fill from `src`.
    /// Count is masked to 5 bits; `None` for a zero count (no change).
    /// Flags: SZP from the result, CF = last bit out, OF = sign change,
    /// AF cleared. 16-bit counts above 16 follow the 32-bit concatenation.
    fn double_shift(
        &mut self,
        dst: u32,
        src: u32,
        count: u8,
        bits: u32,
        left: bool,
    ) -> Option<u32> {
        let c = (count & 0x1F) as u32;
        if c == 0 {
            return None;
        }
        let mask = if bits == 32 { u32::MAX } else { 0xFFFF };
        let (res, cf) = if left {
            let v = (dst as u64) << bits | src as u64;
            (
                ((v << c) >> bits) as u32 & mask,
                (v >> (2 * bits - c)) & 1 != 0,
            )
        } else {
            let v = (src as u64) << bits | dst as u64;
            ((v >> c) as u32 & mask, (v >> (c - 1)) & 1 != 0)
        };
        let msb = 1u32 << (bits - 1);
        let mut f = if bits == 32 {
            alu::or32(res, 0, self.flags).1
        } else {
            alu::or16(res as u16, 0, self.flags).1
        };
        if cf {
            f |= CF;
        }
        if (res ^ dst) & msb != 0 {
            f |= OF;
        }
        self.flags = f;
        Some(res)
    }
}
