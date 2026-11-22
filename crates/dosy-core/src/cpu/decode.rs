//! Instruction fetch, prefixes and ModRM / 16-bit effective addresses.

use super::Cpu;
use super::regs::{BP, BX, DI, SI, Seg};
use crate::bus::{Bus, linear};

/// Repeat prefix. Recorded here; string instructions (007) act on it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Rep {
    #[default]
    None,
    /// `F3`: REP / REPE.
    Rep,
    /// `F2`: REPNE.
    Repne,
}

/// Prefixes seen before the opcode.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Prefixes {
    pub seg: Option<Seg>,
    pub rep: Rep,
    /// `F0`, ignored (single CPU).
    pub lock: bool,
}

/// Decoded r/m operand.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rm {
    /// Register number (8- or 16-bit encoding, per the instruction).
    Reg(usize),
    /// Memory at `seg:off`; `seg` already has overrides applied.
    Mem { seg: Seg, off: u16 },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ModRm {
    /// ModRM `reg` field: register number or group opcode extension.
    pub reg: usize,
    pub rm: Rm,
}

impl Cpu {
    /// Fetch a byte at CS:IP; IP wraps within the 64 KiB segment.
    #[inline]
    pub fn fetch8<B: Bus>(&mut self, bus: &mut B) -> u8 {
        let v = bus.read8(linear(self.sregs.get(Seg::Cs), self.ip as u16));
        self.ip = (self.ip + 1) & 0xFFFF;
        v
    }

    #[inline]
    pub fn fetch16<B: Bus>(&mut self, bus: &mut B) -> u16 {
        let lo = self.fetch8(bus);
        let hi = self.fetch8(bus);
        u16::from_le_bytes([lo, hi])
    }

    /// Fetch a ModRM byte (plus displacement) and decode it.
    pub fn decode_modrm<B: Bus>(&mut self, bus: &mut B, pfx: &Prefixes) -> ModRm {
        let b = self.fetch8(bus);
        self.decode_modrm_byte(bus, pfx, b)
    }

    /// Decode an already fetched ModRM byte, fetching any displacement.
    pub fn decode_modrm_byte<B: Bus>(&mut self, bus: &mut B, pfx: &Prefixes, b: u8) -> ModRm {
        let md = b >> 6;
        let reg = ((b >> 3) & 7) as usize;
        let rm = (b & 7) as usize;
        if md == 3 {
            return ModRm {
                reg,
                rm: Rm::Reg(rm),
            };
        }
        let r = |i| self.regs.get16(i);
        let (base, default_seg) = match rm {
            0 => (r(BX).wrapping_add(r(SI)), Seg::Ds),
            1 => (r(BX).wrapping_add(r(DI)), Seg::Ds),
            2 => (r(BP).wrapping_add(r(SI)), Seg::Ss),
            3 => (r(BP).wrapping_add(r(DI)), Seg::Ss),
            4 => (r(SI), Seg::Ds),
            5 => (r(DI), Seg::Ds),
            6 if md == 0 => (0, Seg::Ds),
            6 => (r(BP), Seg::Ss),
            _ => (r(BX), Seg::Ds),
        };
        let disp = match md {
            0 if rm == 6 => self.fetch16(bus),
            0 => 0,
            1 => self.fetch8(bus) as i8 as u16,
            _ => self.fetch16(bus),
        };
        ModRm {
            reg,
            rm: Rm::Mem {
                seg: pfx.seg.unwrap_or(default_seg),
                off: base.wrapping_add(disp),
            },
        }
    }
}
