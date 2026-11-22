//! General-purpose and segment registers.

/// General register numbers in x86 encoding order (ModRM `reg`/`rm`, `50+r`).
pub const AX: usize = 0;
pub const CX: usize = 1;
pub const DX: usize = 2;
pub const BX: usize = 3;
pub const SP: usize = 4;
pub const BP: usize = 5;
pub const SI: usize = 6;
pub const DI: usize = 7;

/// EAX..EDI, stored 32-bit so 386 code (task 025) needs no layout change.
/// 16-bit and 8-bit writes preserve the untouched bits.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Regs {
    pub r: [u32; 8],
}

macro_rules! named16 {
    ($($get:ident $set:ident $i:expr;)*) => {$(
        #[inline] pub fn $get(&self) -> u16 { self.get16($i) }
        #[inline] pub fn $set(&mut self, v: u16) { self.set16($i, v) }
    )*};
}
macro_rules! named8 {
    ($($get:ident $set:ident $i:expr;)*) => {$(
        #[inline] pub fn $get(&self) -> u8 { self.get8($i) }
        #[inline] pub fn $set(&mut self, v: u8) { self.set8($i, v) }
    )*};
}

impl Regs {
    #[inline]
    pub fn get32(&self, i: usize) -> u32 {
        self.r[i]
    }
    #[inline]
    pub fn set32(&mut self, i: usize, v: u32) {
        self.r[i] = v;
    }
    #[inline]
    pub fn get16(&self, i: usize) -> u16 {
        self.r[i] as u16
    }
    #[inline]
    pub fn set16(&mut self, i: usize, v: u16) {
        self.r[i] = (self.r[i] & 0xFFFF_0000) | v as u32;
    }
    /// 8-bit register by encoding: 0..3 = AL CL DL BL, 4..7 = AH CH DH BH.
    #[inline]
    pub fn get8(&self, i: usize) -> u8 {
        let r = self.r[i & 3];
        if i < 4 { r as u8 } else { (r >> 8) as u8 }
    }
    #[inline]
    pub fn set8(&mut self, i: usize, v: u8) {
        let r = &mut self.r[i & 3];
        if i < 4 {
            *r = (*r & !0xFF) | v as u32;
        } else {
            *r = (*r & !0xFF00) | (v as u32) << 8;
        }
    }

    named16! {
        ax set_ax AX; cx set_cx CX; dx set_dx DX; bx set_bx BX;
        sp set_sp SP; bp set_bp BP; si set_si SI; di set_di DI;
    }
    named8! {
        al set_al 0; cl set_cl 1; dl set_dl 2; bl set_bl 3;
        ah set_ah 4; ch set_ch 5; dh set_dh 6; bh set_bh 7;
    }
}

/// Segment registers in x86 encoding order (ModRM `reg` of `8C`/`8E`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum Seg {
    Es = 0,
    Cs = 1,
    Ss = 2,
    Ds = 3,
    Fs = 4,
    Gs = 5,
}

impl Seg {
    pub fn from_index(i: usize) -> Option<Seg> {
        Some(match i {
            0 => Seg::Es,
            1 => Seg::Cs,
            2 => Seg::Ss,
            3 => Seg::Ds,
            4 => Seg::Fs,
            5 => Seg::Gs,
            _ => return None,
        })
    }
}

/// Segment selectors. Real mode only: base is always `value << 4`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SegRegs {
    pub s: [u16; 6],
}

impl SegRegs {
    #[inline]
    pub fn get(&self, seg: Seg) -> u16 {
        self.s[seg as usize]
    }
    #[inline]
    pub fn set(&mut self, seg: Seg, v: u16) {
        self.s[seg as usize] = v;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn partial_writes_preserve_other_bits() {
        let mut r = Regs::default();
        r.set32(AX, 0x1234_5678);
        r.set16(AX, 0xABCD);
        assert_eq!(r.get32(AX), 0x1234_ABCD);
        r.set_ah(0x11);
        r.set_al(0x22);
        assert_eq!(r.get32(AX), 0x1234_1122);
        r.set8(7, 0x99); // BH
        assert_eq!(r.bx(), 0x9900);
        assert_eq!(r.get8(3), 0x00);
    }
}
