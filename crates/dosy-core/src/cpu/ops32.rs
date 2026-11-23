//! 386 operand-size (66h) forms of one-byte opcodes. Only opcodes whose
//! meaning depends on the operand size are here; byte-sized and string
//! opcodes fall through to the normal table (`string_op` reads 66h itself).

use super::alu;
use super::regs::{AX, BP, DX, SP, Seg};
use super::{Cpu, Prefixes, Rm, Step};
use crate::bus::Bus;

impl Cpu {
    /// `None`: not size-dependent, use the 16-bit table. `Some(None)`:
    /// invalid. `Some(Some(step))`: executed.
    pub(super) fn exec32<B: Bus>(
        &mut self,
        bus: &mut B,
        op: u8,
        pfx: &Prefixes,
    ) -> Option<Option<Step>> {
        match op {
            // ALU r/m32,r32 | r32,r/m32 | EAX,imm32.
            0x00..=0x3F if matches!(op & 7, 1 | 3 | 5) => {
                let kind = (op >> 3) as usize;
                match op & 7 {
                    1 => {
                        let m = self.decode_modrm(bus, pfx);
                        let (a, b) = (self.read_rm32(bus, m.rm), self.regs.get32(m.reg));
                        if let Some(r) = self.alu32(kind, a, b) {
                            self.write_rm32(bus, m.rm, r);
                        }
                    }
                    3 => {
                        let m = self.decode_modrm(bus, pfx);
                        let (a, b) = (self.regs.get32(m.reg), self.read_rm32(bus, m.rm));
                        if let Some(r) = self.alu32(kind, a, b) {
                            self.regs.set32(m.reg, r);
                        }
                    }
                    _ => {
                        let b = self.fetch32(bus);
                        if let Some(r) = self.alu32(kind, self.regs.get32(AX), b) {
                            self.regs.set32(AX, r);
                        }
                    }
                }
            }
            // PUSH/POP sreg with 66h move 32 bits (selector zero-extended).
            0x06 | 0x0E | 0x16 | 0x1E => {
                let s = Seg::from_index((op >> 3) as usize)?;
                self.push32(bus, self.sregs.get(s) as u32);
            }
            0x07 | 0x17 | 0x1F => {
                let s = Seg::from_index((op >> 3) as usize)?;
                let v = self.pop32(bus) as u16;
                self.sregs.set(s, v);
                if s == Seg::Ss {
                    self.int_shadow = true;
                }
            }
            0x40..=0x4F => {
                let r = (op & 7) as usize;
                let a = self.regs.get32(r);
                let (v, f) = if op < 0x48 {
                    alu::inc32(a, self.flags)
                } else {
                    alu::dec32(a, self.flags)
                };
                self.flags = f;
                self.regs.set32(r, v);
            }
            0x50..=0x57 => self.push32(bus, self.regs.get32((op & 7) as usize)),
            0x58..=0x5F => {
                let v = self.pop32(bus);
                self.regs.set32((op & 7) as usize, v);
            }
            // PUSHAD / POPAD.
            0x60 => {
                let sp = self.regs.get32(SP);
                for r in 0..8 {
                    let v = if r == SP { sp } else { self.regs.get32(r) };
                    self.push32(bus, v);
                }
            }
            0x61 => {
                for r in (0..8).rev() {
                    let v = self.pop32(bus);
                    if r != SP {
                        self.regs.set32(r, v);
                    }
                }
            }
            0x68 => {
                let v = self.fetch32(bus);
                self.push32(bus, v);
            }
            0x6A => {
                let v = self.fetch8(bus) as i8 as u32;
                self.push32(bus, v);
            }
            0x69 | 0x6B => {
                let m = self.decode_modrm(bus, pfx);
                let a = self.read_rm32(bus, m.rm);
                let b = if op == 0x69 {
                    self.fetch32(bus)
                } else {
                    self.fetch8(bus) as i8 as u32
                };
                let (p, f) = alu::imul32(a, b, self.flags);
                self.flags = f;
                self.regs.set32(m.reg, p as u32);
            }
            0x81 | 0x83 => {
                let m = self.decode_modrm(bus, pfx);
                let a = self.read_rm32(bus, m.rm);
                let b = if op == 0x83 {
                    self.fetch8(bus) as i8 as u32
                } else {
                    self.fetch32(bus)
                };
                if let Some(r) = self.alu32(m.reg, a, b) {
                    self.write_rm32(bus, m.rm, r);
                }
            }
            0x85 => {
                let m = self.decode_modrm(bus, pfx);
                let a = self.read_rm32(bus, m.rm);
                self.flags = alu::test32(a, self.regs.get32(m.reg), self.flags);
            }
            0x87 => {
                let m = self.decode_modrm(bus, pfx);
                let a = self.regs.get32(m.reg);
                let b = self.read_rm32(bus, m.rm);
                self.write_rm32(bus, m.rm, a);
                self.regs.set32(m.reg, b);
            }
            0x89 => {
                let m = self.decode_modrm(bus, pfx);
                let v = self.regs.get32(m.reg);
                self.write_rm32(bus, m.rm, v);
            }
            0x8B => {
                let m = self.decode_modrm(bus, pfx);
                let v = self.read_rm32(bus, m.rm);
                self.regs.set32(m.reg, v);
            }
            // MOV r32, sreg zero-extends; to memory it stores 16 bits.
            0x8C => {
                let m = self.decode_modrm(bus, pfx);
                let v = self.sregs.get(Seg::from_index(m.reg)?);
                match m.rm {
                    Rm::Reg(r) => self.regs.set32(r, v as u32),
                    rm => self.write_rm16(bus, rm, v),
                }
            }
            0x8D => {
                let m = self.decode_modrm(bus, pfx);
                let Rm::Mem { off, .. } = m.rm else {
                    return Some(None);
                };
                self.regs.set32(m.reg, off as u32);
            }
            0x8F => {
                let m = self.decode_modrm(bus, pfx);
                if m.reg != 0 {
                    return Some(None);
                }
                let v = self.pop32(bus);
                self.write_rm32(bus, m.rm, v);
            }
            0x91..=0x97 => {
                let r = (op & 7) as usize;
                let (a, b) = (self.regs.get32(AX), self.regs.get32(r));
                self.regs.set32(AX, b);
                self.regs.set32(r, a);
            }
            0x98 => self.regs.set32(AX, alu::cwde(self.regs.ax())),
            0x99 => self.regs.set32(DX, alu::cdq(self.regs.get32(AX))),
            // PUSHFD (VM/RF read 0) / POPFD (bits 16+ are not writable on a
            // 386 in real mode: AC and ID stay 0, as detection code expects).
            0x9C => self.push32(bus, self.flags16() as u32),
            0x9D => {
                let v = self.pop32(bus);
                self.set_flags16(v as u16);
            }
            0xA1 | 0xA3 => {
                let off = if pfx.addr32 {
                    self.fetch32(bus) as u16
                } else {
                    self.fetch16(bus)
                };
                let seg = pfx.seg.unwrap_or(Seg::Ds);
                if op == 0xA1 {
                    let v = self.read_mem32(bus, seg, off);
                    self.regs.set32(AX, v);
                } else {
                    self.write_mem32(bus, seg, off, self.regs.get32(AX));
                }
            }
            0xA9 => {
                let b = self.fetch32(bus);
                self.flags = alu::test32(self.regs.get32(AX), b, self.flags);
            }
            0xB8..=0xBF => {
                let v = self.fetch32(bus);
                self.regs.set32((op & 7) as usize, v);
            }
            0xC1 | 0xD1 | 0xD3 => {
                let m = self.decode_modrm(bus, pfx);
                let a = self.read_rm32(bus, m.rm);
                let count = match op {
                    0xC1 => self.fetch8(bus),
                    0xD1 => 1,
                    _ => self.regs.cl(),
                };
                let r = self.shift32(m.reg, a, count);
                self.write_rm32(bus, m.rm, r);
            }
            // Near RET with 66h pops a 32-bit EIP.
            0xC2 | 0xC3 => {
                let n = if op == 0xC2 { self.fetch16(bus) } else { 0 };
                self.ip = self.pop32(bus) & 0xFFFF;
                self.regs.set_sp(self.regs.sp().wrapping_add(n));
            }
            // LES / LDS r32, m16:32.
            0xC4 | 0xC5 => {
                let m = self.decode_modrm(bus, pfx);
                let s = if op == 0xC4 { Seg::Es } else { Seg::Ds };
                return Some(self.load_far32(bus, m.reg, m.rm, s));
            }
            0xC7 => {
                let m = self.decode_modrm(bus, pfx);
                let v = self.fetch32(bus);
                if m.reg != 0 {
                    return Some(None);
                }
                self.write_rm32(bus, m.rm, v);
            }
            // LEAVE with 66h: ESP = EBP; pop EBP.
            0xC9 => {
                self.regs.set32(SP, self.regs.get32(BP));
                let v = self.pop32(bus);
                self.regs.set32(BP, v);
            }
            0xCA | 0xCB => {
                let n = if op == 0xCA { self.fetch16(bus) } else { 0 };
                let ip = self.pop32(bus) as u16;
                let cs = self.pop32(bus) as u16;
                self.far_jump(cs, ip);
                self.regs.set_sp(self.regs.sp().wrapping_add(n));
            }
            // IRETD.
            0xCF => {
                let ip = self.pop32(bus) as u16;
                let cs = self.pop32(bus) as u16;
                let f = self.pop32(bus);
                self.far_jump(cs, ip);
                self.set_flags16(f as u16);
            }
            0xE5 => {
                let p = self.fetch8(bus) as u16;
                let v = in32(bus, p);
                self.regs.set32(AX, v);
            }
            0xE7 => {
                let p = self.fetch8(bus) as u16;
                out32(bus, p, self.regs.get32(AX));
            }
            0xED => {
                let v = in32(bus, self.regs.dx());
                self.regs.set32(AX, v);
            }
            0xEF => out32(bus, self.regs.dx(), self.regs.get32(AX)),
            // CALL / JMP rel32 (EIP truncated to 16 bits).
            0xE8 => {
                let d = self.fetch32(bus);
                self.push32(bus, self.ip);
                self.ip = self.ip.wrapping_add(d) & 0xFFFF;
            }
            0xE9 => {
                let d = self.fetch32(bus);
                self.ip = self.ip.wrapping_add(d) & 0xFFFF;
            }
            0xF7 => {
                let m = self.decode_modrm(bus, pfx);
                return Some(Some(self.group3_32(bus, m.reg, m.rm)));
            }
            0xFF => {
                let m = self.decode_modrm(bus, pfx);
                match m.reg {
                    0 | 1 => {
                        let a = self.read_rm32(bus, m.rm);
                        let (r, f) = if m.reg == 0 {
                            alu::inc32(a, self.flags)
                        } else {
                            alu::dec32(a, self.flags)
                        };
                        self.flags = f;
                        self.write_rm32(bus, m.rm, r);
                    }
                    2 => {
                        let t = self.read_rm32(bus, m.rm);
                        self.push32(bus, self.ip);
                        self.ip = t & 0xFFFF;
                    }
                    4 => self.ip = self.read_rm32(bus, m.rm) & 0xFFFF,
                    3 | 5 => {
                        let Rm::Mem { seg, off } = m.rm else {
                            return Some(None);
                        };
                        let ip = self.read_mem32(bus, seg, off) as u16;
                        let cs = self.read_mem16(bus, seg, off.wrapping_add(4));
                        if m.reg == 3 {
                            self.push32(bus, self.sregs.get(Seg::Cs) as u32);
                            self.push32(bus, self.ip);
                        }
                        self.far_jump(cs, ip);
                    }
                    6 => {
                        let v = self.read_rm32(bus, m.rm);
                        self.push32(bus, v);
                    }
                    _ => return Some(None),
                }
            }
            _ => return None,
        }
        Some(Some(Step::Ok))
    }

    /// LxS r32, m16:32 (offset dword, then selector).
    pub(super) fn load_far32<B: Bus>(
        &mut self,
        bus: &mut B,
        reg: usize,
        rm: Rm,
        s: Seg,
    ) -> Option<Step> {
        let Rm::Mem { seg, off } = rm else {
            return None;
        };
        let v = self.read_mem32(bus, seg, off);
        let sel = self.read_mem16(bus, seg, off.wrapping_add(4));
        self.regs.set32(reg, v);
        self.sregs.set(s, sel);
        if s == Seg::Ss {
            self.int_shadow = true;
        }
        Some(Step::Ok)
    }
}

fn in32<B: Bus>(bus: &mut B, port: u16) -> u32 {
    bus.io_in16(port) as u32 | (bus.io_in16(port.wrapping_add(2)) as u32) << 16
}

fn out32<B: Bus>(bus: &mut B, port: u16, v: u32) {
    bus.io_out16(port, v as u16);
    bus.io_out16(port.wrapping_add(2), (v >> 16) as u16);
}
