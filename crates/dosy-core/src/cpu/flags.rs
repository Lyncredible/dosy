//! FLAGS / EFLAGS bit masks, shared by the ALU and the executor.

pub const CF: u32 = 1 << 0;
/// Always 1 on 8086..386.
pub const RESERVED1: u32 = 1 << 1;
pub const PF: u32 = 1 << 2;
pub const AF: u32 = 1 << 4;
pub const ZF: u32 = 1 << 6;
pub const SF: u32 = 1 << 7;
pub const TF: u32 = 1 << 8;
pub const IF: u32 = 1 << 9;
pub const DF: u32 = 1 << 10;
pub const OF: u32 = 1 << 11;

/// Flags written by arithmetic ops (ADD, SUB, CMP, ...).
pub const ARITH: u32 = CF | PF | AF | ZF | SF | OF;

/// Parity flag value for the low byte of `v` (set when the bit count is even).
#[inline]
pub fn parity(v: u32) -> u32 {
    if (v as u8).count_ones() % 2 == 0 {
        PF
    } else {
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parity_uses_low_byte_only() {
        assert_eq!(parity(0x00), PF);
        assert_eq!(parity(0x01), 0);
        assert_eq!(parity(0x03), PF);
        assert_eq!(parity(0x100), PF);
    }
}
