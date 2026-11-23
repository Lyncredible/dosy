//! `Cpu::step`: prefix loop and the opcode dispatch table.
//!
//! One `match` arm per opcode (or opcode range); bulky groups call helpers
//! in `ops.rs`. 386 extensions (025) slot in as new arms; anything unhandled
//! faults.

use super::alu;
use super::flags::{self, AF, CF, DF, IF, OF, PF, SF, ZF};
use super::regs::{AX, BX, CX, DX, Seg};
use super::{Cpu, CpuFault, CpuModel, Flags, Prefixes, Rep, Rm, Step};
use crate::bus::Bus;
use crate::hle::HleId;

/// Longest legal instruction; more prefixes than this fault.
const MAX_INSN_LEN: u32 = 15;

/// Jcc / SETcc / CMOVcc condition `cc` (low nibble of `70+cc`).
pub fn condition(f: Flags, cc: u8) -> bool {
    let set = |m: u32| f & m != 0;
    let r = match cc >> 1 {
        0 => set(OF),
        1 => set(CF),
        2 => set(ZF),
        3 => set(CF) || set(ZF),
        4 => set(SF),
        5 => set(PF),
        6 => set(SF) != set(OF),
        _ => set(ZF) || (set(SF) != set(OF)),
    };
    r != (cc & 1 != 0)
}

impl Cpu {
    /// Execute one instruction. Returns the outcome and its cost in cycles.
    pub fn step<B: Bus>(&mut self, bus: &mut B) -> (Step, u32) {
        self.int_shadow = false;
        if self.halted {
            return (Step::Halt, 1);
        }
        let start = self.ip;
        self.insn_ip = start;
        let mut pfx = Prefixes::default();
        let op = loop {
            if (self.ip.wrapping_sub(start) & 0xFFFF) >= MAX_INSN_LEN {
                return (self.fault(start, 0), 1);
            }
            match self.fetch8(bus) {
                0x26 => pfx.seg = Some(Seg::Es),
                0x2E => pfx.seg = Some(Seg::Cs),
                0x36 => pfx.seg = Some(Seg::Ss),
                0x3E => pfx.seg = Some(Seg::Ds),
                0x64 => pfx.seg = Some(Seg::Fs),
                0x65 => pfx.seg = Some(Seg::Gs),
                0xF0 => pfx.lock = true,
                // Pre-386, F1 decodes as another LOCK.
                0xF1 if self.model < CpuModel::I386 => pfx.lock = true,
                0xF2 => pfx.rep = Rep::Repne,
                0xF3 => pfx.rep = Rep::Rep,
                op => break op,
            }
        };
        match self.exec(bus, op, &pfx) {
            Some(step) => (step, 1),
            None => (self.fault(start, op), 1),
        }
    }

    fn fault(&mut self, start: u32, opcode: u8) -> Step {
        self.ip = start;
        Step::Fault(CpuFault::InvalidOpcode {
            cs: self.sregs.get(Seg::Cs),
            ip: start as u16,
            opcode,
        })
    }

    #[inline]
    fn jump_rel(&mut self, d: u16) {
        self.ip = (self.ip as u16).wrapping_add(d) as u32;
    }

    #[inline]
    fn fetch_rel8<B: Bus>(&mut self, bus: &mut B) -> u16 {
        self.fetch8(bus) as i8 as u16
    }

    fn set_sreg(&mut self, s: Seg, v: u16) {
        self.sregs.set(s, v);
        if s == Seg::Ss {
            self.int_shadow = true;
        }
    }

    /// Decoded opcode `op`; `None` = invalid / not yet implemented.
    fn exec<B: Bus>(&mut self, bus: &mut B, op: u8, pfx: &Prefixes) -> Option<Step> {
        match op {
            // --- MOV ---
            0x88 => {
                let m = self.decode_modrm(bus, pfx);
                let v = self.regs.get8(m.reg);
                self.write_rm8(bus, m.rm, v);
            }
            0x89 => {
                let m = self.decode_modrm(bus, pfx);
                let v = self.regs.get16(m.reg);
                self.write_rm16(bus, m.rm, v);
            }
            0x8A => {
                let m = self.decode_modrm(bus, pfx);
                let v = self.read_rm8(bus, m.rm);
                self.regs.set8(m.reg, v);
            }
            0x8B => {
                let m = self.decode_modrm(bus, pfx);
                let v = self.read_rm16(bus, m.rm);
                self.regs.set16(m.reg, v);
            }
            0x8C => {
                let m = self.decode_modrm(bus, pfx);
                let v = self.sregs.get(Seg::from_index(m.reg)?);
                self.write_rm16(bus, m.rm, v);
            }
            0x8E => {
                let m = self.decode_modrm(bus, pfx);
                let s = Seg::from_index(m.reg).filter(|&s| s != Seg::Cs)?;
                let v = self.read_rm16(bus, m.rm);
                self.set_sreg(s, v);
            }
            0xA0..=0xA3 => {
                let off = self.fetch16(bus);
                let seg = pfx.seg.unwrap_or(Seg::Ds);
                match op {
                    0xA0 => {
                        let v = self.read_mem8(bus, seg, off);
                        self.regs.set_al(v);
                    }
                    0xA1 => {
                        let v = self.read_mem16(bus, seg, off);
                        self.regs.set_ax(v);
                    }
                    0xA2 => self.write_mem8(bus, seg, off, self.regs.al()),
                    _ => self.write_mem16(bus, seg, off, self.regs.ax()),
                }
            }
            0xB0..=0xB7 => {
                let v = self.fetch8(bus);
                self.regs.set8((op & 7) as usize, v);
            }
            0xB8..=0xBF => {
                let v = self.fetch16(bus);
                self.regs.set16((op & 7) as usize, v);
            }
            0xC6 => {
                let m = self.decode_modrm(bus, pfx);
                let v = self.fetch8(bus);
                if m.reg != 0 {
                    return None;
                }
                self.write_rm8(bus, m.rm, v);
            }
            0xC7 => {
                let m = self.decode_modrm(bus, pfx);
                let v = self.fetch16(bus);
                if m.reg != 0 {
                    return None;
                }
                self.write_rm16(bus, m.rm, v);
            }

            // --- Stack ---
            // PUSH SP pushes the value before the decrement (286+); the
            // 8086 pushes the decremented value.
            0x54 if self.model == CpuModel::I8086 => {
                let v = self.regs.sp().wrapping_sub(2);
                self.push16(bus, v);
            }
            0x50..=0x57 => self.push16(bus, self.regs.get16((op & 7) as usize)),
            0x58..=0x5F => {
                let v = self.pop16(bus);
                self.regs.set16((op & 7) as usize, v);
            }
            0x06 | 0x0E | 0x16 | 0x1E => {
                let s = Seg::from_index((op >> 3) as usize)?;
                self.push16(bus, self.sregs.get(s));
            }
            0x07 | 0x17 | 0x1F => {
                let s = Seg::from_index((op >> 3) as usize)?;
                let v = self.pop16(bus);
                self.set_sreg(s, v);
            }
            0x8F => {
                let m = self.decode_modrm(bus, pfx);
                if m.reg != 0 {
                    return None;
                }
                let v = self.pop16(bus);
                self.write_rm16(bus, m.rm, v);
            }
            0x9C => self.push16(bus, self.flags16()),
            0x9D => {
                let v = self.pop16(bus);
                self.set_flags16(v);
            }

            // --- XCHG, LEA, LDS/LES ---
            0x86 => {
                let m = self.decode_modrm(bus, pfx);
                let a = self.regs.get8(m.reg);
                let b = self.read_rm8(bus, m.rm);
                self.write_rm8(bus, m.rm, a);
                self.regs.set8(m.reg, b);
            }
            0x87 => {
                let m = self.decode_modrm(bus, pfx);
                let a = self.regs.get16(m.reg);
                let b = self.read_rm16(bus, m.rm);
                self.write_rm16(bus, m.rm, a);
                self.regs.set16(m.reg, b);
            }
            0x90 => {} // NOP (also PAUSE with F3)
            0x91..=0x97 => {
                let r = (op & 7) as usize;
                let (a, b) = (self.regs.get16(AX), self.regs.get16(r));
                self.regs.set16(AX, b);
                self.regs.set16(r, a);
            }
            0x8D => {
                let m = self.decode_modrm(bus, pfx);
                let Rm::Mem { off, .. } = m.rm else {
                    return None;
                };
                self.regs.set16(m.reg, off);
            }
            0xC4 | 0xC5 => {
                let m = self.decode_modrm(bus, pfx);
                let Rm::Mem { seg, off } = m.rm else {
                    return None;
                };
                let v = self.read_mem16(bus, seg, off);
                let s = self.read_mem16(bus, seg, off.wrapping_add(2));
                self.regs.set16(m.reg, v);
                self.sregs
                    .set(if op == 0xC4 { Seg::Es } else { Seg::Ds }, s);
            }

            // --- Flags ---
            0x9E => {
                const M: u32 = SF | ZF | AF | PF | CF;
                self.flags = (self.flags & !M) | (self.regs.ah() as u32 & M);
            }
            0x9F => {
                let v = (self.flags & (SF | ZF | AF | PF | CF)) | flags::RESERVED1;
                self.regs.set_ah(v as u8);
            }
            0xF5 => self.flags ^= CF,
            0xF8 => self.flags &= !CF,
            0xF9 => self.flags |= CF,
            0xFA => self.flags &= !IF,
            0xFB => {
                if self.flags & IF == 0 {
                    self.int_shadow = true;
                }
                self.flags |= IF;
            }
            0xFC => self.flags &= !DF,
            0xFD => self.flags |= DF,
            0xF4 => {
                self.halted = true;
                return Some(Step::Halt);
            }

            // --- Jumps, calls, returns ---
            0xEB => {
                let d = self.fetch_rel8(bus);
                self.jump_rel(d);
            }
            0xE9 => {
                let d = self.fetch16(bus);
                self.jump_rel(d);
            }
            0xEA => {
                let ip = self.fetch16(bus);
                let cs = self.fetch16(bus);
                self.far_jump(cs, ip);
            }
            0xE8 => {
                let d = self.fetch16(bus);
                self.push16(bus, self.ip as u16);
                self.jump_rel(d);
            }
            0x9A => {
                let ip = self.fetch16(bus);
                let cs = self.fetch16(bus);
                self.push16(bus, self.sregs.get(Seg::Cs));
                self.push16(bus, self.ip as u16);
                self.far_jump(cs, ip);
            }
            0xC2 | 0xC3 => {
                let n = if op == 0xC2 { self.fetch16(bus) } else { 0 };
                self.ip = self.pop16(bus) as u32;
                self.regs.set_sp(self.regs.sp().wrapping_add(n));
            }
            0xCA | 0xCB => {
                let n = if op == 0xCA { self.fetch16(bus) } else { 0 };
                let ip = self.pop16(bus);
                let cs = self.pop16(bus);
                self.far_jump(cs, ip);
                self.regs.set_sp(self.regs.sp().wrapping_add(n));
            }
            0x70..=0x7F => {
                let d = self.fetch_rel8(bus);
                if condition(self.flags, op & 0xF) {
                    self.jump_rel(d);
                }
            }
            0xE0..=0xE3 => {
                let d = self.fetch_rel8(bus);
                let cx = if op == 0xE3 {
                    self.regs.get16(CX)
                } else {
                    let cx = self.regs.get16(CX).wrapping_sub(1);
                    self.regs.set16(CX, cx);
                    cx
                };
                let zf = self.flags & ZF != 0;
                let taken = match op {
                    0xE0 => cx != 0 && !zf, // LOOPNE
                    0xE1 => cx != 0 && zf,  // LOOPE
                    0xE2 => cx != 0,        // LOOP
                    _ => cx == 0,           // JCXZ
                };
                if taken {
                    self.jump_rel(d);
                }
            }

            // --- Interrupts ---
            0xCC => self.interrupt(bus, 3),
            0xCD => {
                let n = self.fetch8(bus);
                self.interrupt(bus, n);
            }
            0xCE => {
                if self.flags & OF != 0 {
                    self.interrupt(bus, 4);
                }
            }
            0xCF => {
                let ip = self.pop16(bus);
                let cs = self.pop16(bus);
                let f = self.pop16(bus);
                self.far_jump(cs, ip);
                self.set_flags16(f);
            }

            // --- Groups ---
            0xFE => {
                let b = self.fetch8(bus);
                if b == 0x38 {
                    let id = self.fetch16(bus);
                    return Some(Step::Callback(HleId(id)));
                }
                let m = self.decode_modrm_byte(bus, pfx, b);
                let a = self.read_rm8(bus, m.rm);
                let (r, f) = match m.reg {
                    0 => alu::inc8(a, self.flags),
                    1 => alu::dec8(a, self.flags),
                    _ => return None,
                };
                self.flags = f;
                self.write_rm8(bus, m.rm, r);
            }
            0xFF => {
                let m = self.decode_modrm(bus, pfx);
                match m.reg {
                    0 | 1 => {
                        let a = self.read_rm16(bus, m.rm);
                        let (r, f) = if m.reg == 0 {
                            alu::inc16(a, self.flags)
                        } else {
                            alu::dec16(a, self.flags)
                        };
                        self.flags = f;
                        self.write_rm16(bus, m.rm, r);
                    }
                    2 => {
                        let t = self.read_rm16(bus, m.rm);
                        self.push16(bus, self.ip as u16);
                        self.ip = t as u32;
                    }
                    3 | 5 => {
                        let Rm::Mem { seg, off } = m.rm else {
                            return None;
                        };
                        let ip = self.read_mem16(bus, seg, off);
                        let cs = self.read_mem16(bus, seg, off.wrapping_add(2));
                        if m.reg == 3 {
                            self.push16(bus, self.sregs.get(Seg::Cs));
                            self.push16(bus, self.ip as u16);
                        }
                        self.far_jump(cs, ip);
                    }
                    4 => self.ip = self.read_rm16(bus, m.rm) as u32,
                    6 => {
                        let v = self.read_rm16(bus, m.rm);
                        self.push16(bus, v);
                    }
                    _ => return None,
                }
            }

            // --- ALU ---
            // 00..3F with low 3 bits 0..5: r/m8,r8 | r/m16,r16 | r8,r/m8 |
            // r16,r/m16 | AL,imm8 | AX,imm16; kind = op >> 3.
            0x00..=0x3F if op & 7 < 6 => {
                let kind = (op >> 3) as usize;
                match op & 7 {
                    0 => {
                        let m = self.decode_modrm(bus, pfx);
                        let (a, b) = (self.read_rm8(bus, m.rm), self.regs.get8(m.reg));
                        if let Some(r) = self.alu8(kind, a, b) {
                            self.write_rm8(bus, m.rm, r);
                        }
                    }
                    1 => {
                        let m = self.decode_modrm(bus, pfx);
                        let (a, b) = (self.read_rm16(bus, m.rm), self.regs.get16(m.reg));
                        if let Some(r) = self.alu16(kind, a, b) {
                            self.write_rm16(bus, m.rm, r);
                        }
                    }
                    2 => {
                        let m = self.decode_modrm(bus, pfx);
                        let (a, b) = (self.regs.get8(m.reg), self.read_rm8(bus, m.rm));
                        if let Some(r) = self.alu8(kind, a, b) {
                            self.regs.set8(m.reg, r);
                        }
                    }
                    3 => {
                        let m = self.decode_modrm(bus, pfx);
                        let (a, b) = (self.regs.get16(m.reg), self.read_rm16(bus, m.rm));
                        if let Some(r) = self.alu16(kind, a, b) {
                            self.regs.set16(m.reg, r);
                        }
                    }
                    4 => {
                        let b = self.fetch8(bus);
                        if let Some(r) = self.alu8(kind, self.regs.al(), b) {
                            self.regs.set_al(r);
                        }
                    }
                    _ => {
                        let b = self.fetch16(bus);
                        if let Some(r) = self.alu16(kind, self.regs.ax(), b) {
                            self.regs.set_ax(r);
                        }
                    }
                }
            }
            // 82 is an 8086 alias of 80; 83 sign-extends imm8.
            0x80..=0x83 => {
                let m = self.decode_modrm(bus, pfx);
                if op & 1 == 0 {
                    let a = self.read_rm8(bus, m.rm);
                    let b = self.fetch8(bus);
                    if let Some(r) = self.alu8(m.reg, a, b) {
                        self.write_rm8(bus, m.rm, r);
                    }
                } else {
                    let a = self.read_rm16(bus, m.rm);
                    let b = if op == 0x83 {
                        self.fetch8(bus) as i8 as u16
                    } else {
                        self.fetch16(bus)
                    };
                    if let Some(r) = self.alu16(m.reg, a, b) {
                        self.write_rm16(bus, m.rm, r);
                    }
                }
            }
            0x84 => {
                let m = self.decode_modrm(bus, pfx);
                let a = self.read_rm8(bus, m.rm);
                self.flags = alu::test8(a, self.regs.get8(m.reg), self.flags);
            }
            0x85 => {
                let m = self.decode_modrm(bus, pfx);
                let a = self.read_rm16(bus, m.rm);
                self.flags = alu::test16(a, self.regs.get16(m.reg), self.flags);
            }
            0xA8 => {
                let b = self.fetch8(bus);
                self.flags = alu::test8(self.regs.al(), b, self.flags);
            }
            0xA9 => {
                let b = self.fetch16(bus);
                self.flags = alu::test16(self.regs.ax(), b, self.flags);
            }
            0x40..=0x4F => {
                let r = (op & 7) as usize;
                let a = self.regs.get16(r);
                let (v, f) = if op < 0x48 {
                    alu::inc16(a, self.flags)
                } else {
                    alu::dec16(a, self.flags)
                };
                self.flags = f;
                self.regs.set16(r, v);
            }
            0xF6 => {
                let m = self.decode_modrm(bus, pfx);
                return Some(self.group3_8(bus, m.reg, m.rm));
            }
            0xF7 => {
                let m = self.decode_modrm(bus, pfx);
                return Some(self.group3_16(bus, m.reg, m.rm));
            }
            // D0/D1: count 1; D2/D3: count CL.
            0xD0..=0xD3 => {
                let m = self.decode_modrm(bus, pfx);
                let count = if op & 2 == 0 { 1 } else { self.regs.cl() };
                if op & 1 == 0 {
                    let a = self.read_rm8(bus, m.rm);
                    let r = self.shift8(m.reg, a, count);
                    self.write_rm8(bus, m.rm, r);
                } else {
                    let a = self.read_rm16(bus, m.rm);
                    let r = self.shift16(m.reg, a, count);
                    self.write_rm16(bus, m.rm, r);
                }
            }

            // --- BCD and sign extension ---
            0x27 | 0x2F => {
                let f = if op == 0x27 { alu::daa } else { alu::das };
                let (r, fl) = f(self.regs.al(), self.flags);
                self.flags = fl;
                self.regs.set_al(r);
            }
            0x37 | 0x3F => {
                let f = if op == 0x37 { alu::aaa } else { alu::aas };
                let (r, fl) = f(self.regs.ax(), self.flags);
                self.flags = fl;
                self.regs.set_ax(r);
            }
            0xD4 => {
                let imm = self.fetch8(bus);
                let Ok((r, fl)) = alu::aam(self.regs.al(), imm, self.flags) else {
                    return Some(self.divide_error(bus));
                };
                self.flags = fl;
                self.regs.set_ax(r);
            }
            0xD5 => {
                let imm = self.fetch8(bus);
                let (r, fl) = alu::aad(self.regs.ax(), imm, self.flags);
                self.flags = fl;
                self.regs.set_ax(r);
            }
            0x98 => self.regs.set_ax(alu::cbw(self.regs.al())),
            0x99 => self.regs.set_dx(alu::cwd(self.regs.ax())),
            // SALC (undocumented): AL = CF ? FF : 00.
            0xD6 => self
                .regs
                .set_al(if self.flags & CF != 0 { 0xFF } else { 0 }),

            // --- Strings ---
            0xA4..=0xA7 | 0xAA..=0xAF => self.string_op(bus, op, pfx),

            // --- I/O and misc ---
            0xE4 => {
                let p = self.fetch8(bus) as u16;
                let v = bus.io_in8(p);
                self.regs.set_al(v);
            }
            0xE5 => {
                let p = self.fetch8(bus) as u16;
                let v = bus.io_in16(p);
                self.regs.set_ax(v);
            }
            0xE6 => {
                let p = self.fetch8(bus) as u16;
                bus.io_out8(p, self.regs.al());
            }
            0xE7 => {
                let p = self.fetch8(bus) as u16;
                bus.io_out16(p, self.regs.ax());
            }
            0xEC => {
                let v = bus.io_in8(self.regs.get16(DX));
                self.regs.set_al(v);
            }
            0xED => {
                let v = bus.io_in16(self.regs.get16(DX));
                self.regs.set_ax(v);
            }
            0xEE => bus.io_out8(self.regs.get16(DX), self.regs.al()),
            0xEF => bus.io_out16(self.regs.get16(DX), self.regs.ax()),
            0xD7 => {
                let off = self.regs.get16(BX).wrapping_add(self.regs.al() as u16);
                let v = self.read_mem8(bus, pfx.seg.unwrap_or(Seg::Ds), off);
                self.regs.set_al(v);
            }
            0x9B => {} // WAIT: no FPU
            // ESC: decode the operand and ignore it (no FPU).
            0xD8..=0xDF => {
                self.decode_modrm(bus, pfx);
            }
            _ => return self.exec_ext(bus, op, pfx),
        }
        Some(Step::Ok)
    }
}
