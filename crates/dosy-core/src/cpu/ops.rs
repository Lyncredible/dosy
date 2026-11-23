//! Helpers behind the 8086 ALU, group and string opcodes in `exec.rs`.

use super::alu::{self, DivideError};
use super::flags::{DF, ZF};
use super::regs::{AX, CX, DI, SI, Seg};
use super::{Cpu, CpuModel, Prefixes, Rep, Rm, Step};
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
        pub(super) fn $name(&mut self, kind: usize, a: $t, mut count: u8) -> $t {
            let mut a = a;
            // The 8086 doesn't mask the count: shift 31 at a time.
            if self.model == CpuModel::I8086 {
                while count > 31 {
                    a = self.$name(kind, a, 31);
                    count -= 31;
                }
            }
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
    alu_fn!(alu32, u32, add32 or32 adc32 sbb32 and32 sub32 xor32 cmp32);
    shift_fn!(shift32, u32, rol32 ror32 rcl32 rcr32 shl32 shr32 sar32);

    /// F7 group with 66h on r/m32; MUL/DIV use EDX:EAX.
    pub(super) fn group3_32<B: Bus>(&mut self, bus: &mut B, reg: usize, rm: Rm) -> Step {
        use super::regs::DX;
        let a = self.read_rm32(bus, rm);
        let f = self.flags;
        let edxeax = (self.regs.get32(DX) as u64) << 32 | self.regs.get32(AX) as u64;
        let div: Result<(u32, u32), DivideError> = match reg {
            0 | 1 => {
                let b = self.fetch32(bus);
                self.flags = alu::test32(a, b, f);
                return Step::Ok;
            }
            2 => {
                self.write_rm32(bus, rm, alu::not32(a));
                return Step::Ok;
            }
            3 => {
                let (r, f) = alu::neg32(a, f);
                self.flags = f;
                self.write_rm32(bus, rm, r);
                return Step::Ok;
            }
            4 | 5 => {
                let (p, f) = if reg == 4 {
                    alu::mul32(self.regs.get32(AX), a, f)
                } else {
                    alu::imul32(self.regs.get32(AX), a, f)
                };
                self.flags = f;
                self.regs.set32(AX, p as u32);
                self.regs.set32(DX, (p >> 32) as u32);
                return Step::Ok;
            }
            6 => alu::div32(edxeax, a),
            _ => alu::idiv32(edxeax, a),
        };
        match div {
            Ok((q, r)) => {
                self.regs.set32(AX, q);
                self.regs.set32(DX, r);
                Step::Ok
            }
            Err(DivideError) => self.divide_error(bus),
        }
    }

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

    /// CX, or ECX with a 67h prefix.
    pub(super) fn count_reg(&self, pfx: &Prefixes) -> u32 {
        if pfx.addr32 {
            self.regs.get32(CX)
        } else {
            self.regs.get16(CX) as u32
        }
    }
    pub(super) fn set_count_reg(&mut self, pfx: &Prefixes, v: u32) {
        if pfx.addr32 {
            self.regs.set32(CX, v);
        } else {
            self.regs.set16(CX, v as u16);
        }
    }

    fn read_sized<B: Bus>(&mut self, bus: &mut B, seg: Seg, off: u16, size: u32) -> u32 {
        match size {
            1 => self.read_mem8(bus, seg, off) as u32,
            2 => self.read_mem16(bus, seg, off) as u32,
            _ => self.read_mem32(bus, seg, off),
        }
    }
    fn write_sized<B: Bus>(&mut self, bus: &mut B, seg: Seg, off: u16, size: u32, v: u32) {
        match size {
            1 => self.write_mem8(bus, seg, off, v as u8),
            2 => self.write_mem16(bus, seg, off, v as u16),
            _ => self.write_mem32(bus, seg, off, v),
        }
    }
    /// AL / AX / EAX.
    fn acc(&self, size: u32) -> u32 {
        match size {
            1 => self.regs.al() as u32,
            2 => self.regs.ax() as u32,
            _ => self.regs.get32(AX),
        }
    }
    fn set_acc(&mut self, size: u32, v: u32) {
        match size {
            1 => self.regs.set_al(v as u8),
            2 => self.regs.set_ax(v as u16),
            _ => self.regs.set32(AX, v),
        }
    }
    fn cmp_sized(&mut self, a: u32, b: u32, size: u32) {
        self.flags = match size {
            1 => alu::cmp8(a as u8, b as u8, self.flags),
            2 => alu::cmp16(a as u16, b as u16, self.flags),
            _ => alu::cmp32(a, b, self.flags),
        };
    }

    /// One iteration of MOVS/CMPS/STOS/LODS/SCAS (`A4..=AF`, not A8/A9) or
    /// INS/OUTS (`6C..=6F`). 66h selects dwords, 67h ESI/EDI/ECX (offsets
    /// are still truncated to 16 bits).
    /// With REP, each iteration is its own step: IP is rewound to the
    /// prefix while iterations remain, so interrupts and the cycle budget
    /// can cut in between iterations.
    pub(super) fn string_op<B: Bus>(&mut self, bus: &mut B, op: u8, pfx: &Prefixes) {
        if pfx.rep != Rep::None && self.count_reg(pfx) == 0 {
            return;
        }
        let size: u32 = match (op & 1, pfx.opsize) {
            (0, _) => 1,
            (_, false) => 2,
            _ => 4,
        };
        let delta = if self.flags & DF != 0 {
            size.wrapping_neg()
        } else {
            size
        };
        let src = pfx.seg.unwrap_or(Seg::Ds);
        let idx = |cpu: &Cpu, r: usize| {
            if pfx.addr32 {
                cpu.regs.get32(r)
            } else {
                cpu.regs.get16(r) as u32
            }
        };
        let (si, di) = (idx(self, SI), idx(self, DI));
        let (so, dof) = (si as u16, di as u16);
        let (mut use_si, mut use_di) = (false, false);
        let mut compares = false;
        match op & !1 {
            // MOVS
            0xA4 => {
                let v = self.read_sized(bus, src, so, size);
                self.write_sized(bus, Seg::Es, dof, size, v);
                (use_si, use_di) = (true, true);
            }
            // CMPS: flags of [src:SI] - ES:[DI]
            0xA6 => {
                let a = self.read_sized(bus, src, so, size);
                let b = self.read_sized(bus, Seg::Es, dof, size);
                self.cmp_sized(a, b, size);
                (use_si, use_di, compares) = (true, true, true);
            }
            // STOS
            0xAA => {
                let v = self.acc(size);
                self.write_sized(bus, Seg::Es, dof, size, v);
                use_di = true;
            }
            // INS: ES:[DI] <- port DX
            0x6C => {
                let port = self.regs.dx();
                let v = match size {
                    1 => bus.io_in8(port) as u32,
                    2 => bus.io_in16(port) as u32,
                    _ => {
                        bus.io_in16(port) as u32 | (bus.io_in16(port.wrapping_add(2)) as u32) << 16
                    }
                };
                self.write_sized(bus, Seg::Es, dof, size, v);
                use_di = true;
            }
            // OUTS: port DX <- [src:SI]
            0x6E => {
                let port = self.regs.dx();
                let v = self.read_sized(bus, src, so, size);
                match size {
                    1 => bus.io_out8(port, v as u8),
                    2 => bus.io_out16(port, v as u16),
                    _ => {
                        bus.io_out16(port, v as u16);
                        bus.io_out16(port.wrapping_add(2), (v >> 16) as u16);
                    }
                }
                use_si = true;
            }
            // LODS
            0xAC => {
                let v = self.read_sized(bus, src, so, size);
                self.set_acc(size, v);
                use_si = true;
            }
            // SCAS: flags of AL/AX/EAX - ES:[DI]
            _ => {
                let b = self.read_sized(bus, Seg::Es, dof, size);
                let a = self.acc(size);
                self.cmp_sized(a, b, size);
                (use_di, compares) = (true, true);
            }
        }
        let set_idx = |cpu: &mut Cpu, r: usize, v: u32| {
            if pfx.addr32 {
                cpu.regs.set32(r, v);
            } else {
                cpu.regs.set16(r, v as u16);
            }
        };
        if use_si {
            set_idx(self, SI, si.wrapping_add(delta));
        }
        if use_di {
            set_idx(self, DI, di.wrapping_add(delta));
        }
        if pfx.rep == Rep::None {
            return;
        }
        let cx = self.count_reg(pfx).wrapping_sub(1);
        self.set_count_reg(pfx, cx);
        let zf = self.flags & ZF != 0;
        let more = cx != 0 && (!compares || (pfx.rep == Rep::Rep) == zf);
        if more {
            self.ip = self.insn_ip;
        }
    }
}
