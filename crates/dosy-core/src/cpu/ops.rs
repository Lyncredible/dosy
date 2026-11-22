//! Helpers behind the 8086 ALU, group and string opcodes in `exec.rs`.

use super::alu::{self, DivideError};
use super::flags::{DF, ZF};
use super::regs::{CX, DI, SI, Seg};
use super::{Cpu, Prefixes, Rep, Rm, Step};
use crate::bus::Bus;

/// `kind` = ModRM `reg` / `op >> 3` order: ADD OR ADC SBB AND SUB XOR CMP.
macro_rules! alu_fn {
    ($name:ident, $t:ty, $add:ident $or:ident $adc:ident $sbb:ident $and:ident
     $sub:ident $xor:ident $cmp:ident) => {
        /// Returns `None` for CMP (no write-back).
        pub(super) fn $name(&mut self, kind: usize, a: $t, b: $t) -> Option<$t> {
            let f = self.flags;
            let (r, f) = match kind {
                0 => alu::$add(a, b, f),
                1 => alu::$or(a, b, f),
                2 => alu::$adc(a, b, f),
                3 => alu::$sbb(a, b, f),
                4 => alu::$and(a, b, f),
                5 => alu::$sub(a, b, f),
                6 => alu::$xor(a, b, f),
                _ => {
                    self.flags = alu::$cmp(a, b, f);
                    return None;
                }
            };
            self.flags = f;
            Some(r)
        }
    };
}

/// `kind`: ROL ROR RCL RCR SHL SHR SAL(=SHL) SAR.
macro_rules! shift_fn {
    ($name:ident, $t:ty, $rol:ident $ror:ident $rcl:ident $rcr:ident $shl:ident
     $shr:ident $sar:ident) => {
        pub(super) fn $name(&mut self, kind: usize, a: $t, count: u8) -> $t {
            let f = self.flags;
            let (r, f) = match kind {
                0 => alu::$rol(a, count, f),
                1 => alu::$ror(a, count, f),
                2 => alu::$rcl(a, count, f),
                3 => alu::$rcr(a, count, f),
                4 | 6 => alu::$shl(a, count, f),
                5 => alu::$shr(a, count, f),
                _ => alu::$sar(a, count, f),
            };
            self.flags = f;
            r
        }
    };
}

impl Cpu {
    alu_fn!(alu8, u8, add8 or8 adc8 sbb8 and8 sub8 xor8 cmp8);
    alu_fn!(alu16, u16, add16 or16 adc16 sbb16 and16 sub16 xor16 cmp16);
    shift_fn!(shift8, u8, rol8 ror8 rcl8 rcr8 shl8 shr8 sar8);
    shift_fn!(shift16, u16, rol16 ror16 rcl16 rcr16 shl16 shr16 sar16);

    /// #DE: INT 0 with the return address at the faulting instruction
    /// (286+ behaviour; the 8086 pushes the next instruction).
    pub(super) fn divide_error<B: Bus>(&mut self, bus: &mut B) -> Step {
        self.ip = self.insn_ip;
        self.interrupt(bus, 0);
        Step::Ok
    }

    /// F6 group: TEST NOT NEG MUL IMUL DIV IDIV on r/m8.
    pub(super) fn group3_8<B: Bus>(&mut self, bus: &mut B, reg: usize, rm: Rm) -> Step {
        let a = self.read_rm8(bus, rm);
        let f = self.flags;
        let div: Result<(u8, u8), DivideError> = match reg {
            0 | 1 => {
                let b = self.fetch8(bus);
                self.flags = alu::test8(a, b, f);
                return Step::Ok;
            }
            2 => {
                self.write_rm8(bus, rm, alu::not8(a));
                return Step::Ok;
            }
            3 => {
                let (r, f) = alu::neg8(a, f);
                self.flags = f;
                self.write_rm8(bus, rm, r);
                return Step::Ok;
            }
            4 | 5 => {
                let (p, f) = if reg == 4 {
                    alu::mul8(self.regs.al(), a, f)
                } else {
                    alu::imul8(self.regs.al(), a, f)
                };
                self.flags = f;
                self.regs.set_ax(p);
                return Step::Ok;
            }
            6 => alu::div8(self.regs.ax(), a),
            _ => alu::idiv8(self.regs.ax(), a),
        };
        match div {
            Ok((q, r)) => {
                self.regs.set_al(q);
                self.regs.set_ah(r);
                Step::Ok
            }
            Err(DivideError) => self.divide_error(bus),
        }
    }

    /// F7 group on r/m16; MUL/DIV use DX:AX.
    pub(super) fn group3_16<B: Bus>(&mut self, bus: &mut B, reg: usize, rm: Rm) -> Step {
        let a = self.read_rm16(bus, rm);
        let f = self.flags;
        let dxax = (self.regs.dx() as u32) << 16 | self.regs.ax() as u32;
        let div: Result<(u16, u16), DivideError> = match reg {
            0 | 1 => {
                let b = self.fetch16(bus);
                self.flags = alu::test16(a, b, f);
                return Step::Ok;
            }
            2 => {
                self.write_rm16(bus, rm, alu::not16(a));
                return Step::Ok;
            }
            3 => {
                let (r, f) = alu::neg16(a, f);
                self.flags = f;
                self.write_rm16(bus, rm, r);
                return Step::Ok;
            }
            4 | 5 => {
                let (p, f) = if reg == 4 {
                    alu::mul16(self.regs.ax(), a, f)
                } else {
                    alu::imul16(self.regs.ax(), a, f)
                };
                self.flags = f;
                self.regs.set_ax(p as u16);
                self.regs.set_dx((p >> 16) as u16);
                return Step::Ok;
            }
            6 => alu::div16(dxax, a),
            _ => alu::idiv16(dxax, a),
        };
        match div {
            Ok((q, r)) => {
                self.regs.set_ax(q);
                self.regs.set_dx(r);
                Step::Ok
            }
            Err(DivideError) => self.divide_error(bus),
        }
    }

    /// One iteration of MOVS/CMPS/STOS/LODS/SCAS (`A4..=AF`, not A8/A9).
    /// With REP, each iteration is its own step: IP is rewound to the
    /// prefix while iterations remain, so interrupts and the cycle budget
    /// can cut in between iterations.
    pub(super) fn string_op<B: Bus>(&mut self, bus: &mut B, op: u8, pfx: &Prefixes) {
        if pfx.rep != Rep::None && self.regs.get16(CX) == 0 {
            return;
        }
        let word = op & 1 != 0;
        let delta: u16 = match (word, self.flags & DF != 0) {
            (false, false) => 1,
            (false, true) => 0xFFFF,
            (true, false) => 2,
            (true, true) => 0xFFFE,
        };
        let src = pfx.seg.unwrap_or(Seg::Ds);
        let (si, di) = (self.regs.get16(SI), self.regs.get16(DI));
        let (mut use_si, mut use_di) = (false, false);
        let mut compares = false;
        match op & !1 {
            // MOVS
            0xA4 => {
                if word {
                    let v = self.read_mem16(bus, src, si);
                    self.write_mem16(bus, Seg::Es, di, v);
                } else {
                    let v = self.read_mem8(bus, src, si);
                    self.write_mem8(bus, Seg::Es, di, v);
                }
                (use_si, use_di) = (true, true);
            }
            // CMPS: flags of [src:SI] - ES:[DI]
            0xA6 => {
                if word {
                    let a = self.read_mem16(bus, src, si);
                    let b = self.read_mem16(bus, Seg::Es, di);
                    self.flags = alu::cmp16(a, b, self.flags);
                } else {
                    let a = self.read_mem8(bus, src, si);
                    let b = self.read_mem8(bus, Seg::Es, di);
                    self.flags = alu::cmp8(a, b, self.flags);
                }
                (use_si, use_di, compares) = (true, true, true);
            }
            // STOS
            0xAA => {
                if word {
                    self.write_mem16(bus, Seg::Es, di, self.regs.ax());
                } else {
                    self.write_mem8(bus, Seg::Es, di, self.regs.al());
                }
                use_di = true;
            }
            // LODS
            0xAC => {
                if word {
                    let v = self.read_mem16(bus, src, si);
                    self.regs.set_ax(v);
                } else {
                    let v = self.read_mem8(bus, src, si);
                    self.regs.set_al(v);
                }
                use_si = true;
            }
            // SCAS: flags of AX/AL - ES:[DI]
            _ => {
                if word {
                    let b = self.read_mem16(bus, Seg::Es, di);
                    self.flags = alu::cmp16(self.regs.ax(), b, self.flags);
                } else {
                    let b = self.read_mem8(bus, Seg::Es, di);
                    self.flags = alu::cmp8(self.regs.al(), b, self.flags);
                }
                (use_di, compares) = (true, true);
            }
        }
        if use_si {
            self.regs.set16(SI, si.wrapping_add(delta));
        }
        if use_di {
            self.regs.set16(DI, di.wrapping_add(delta));
        }
        if pfx.rep == Rep::None {
            return;
        }
        let cx = self.regs.get16(CX).wrapping_sub(1);
        self.regs.set16(CX, cx);
        let zf = self.flags & ZF != 0;
        let more = cx != 0 && (!compares || (pfx.rep == Rep::Rep) == zf);
        if more {
            self.ip = self.insn_ip;
        }
    }
}
