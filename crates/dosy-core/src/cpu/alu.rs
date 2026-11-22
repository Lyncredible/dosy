//! Bit-exact x86 arithmetic/logic as pure functions (task 001).
//!
//! Style: `op16(a, b, flags) -> (result, flags)`. `flags` is the full
//! FLAGS/EFLAGS value; only the bits an op defines are changed, everything
//! else (IF, DF, TF, ...) passes through. CMP/TEST return flags only.
//!
//! Undefined flags (Intel "undefined") are fixed as follows, matching DOSBox
//! where known; task 014 (SingleStepTests) may refine them:
//! - AND/OR/XOR/TEST: AF cleared.
//! - Shifts (count > 0): AF cleared; SHL OF = MSB(result) ^ MSB(a) for any
//!   count; SHR OF = MSB(a) when count == 1, else 0; SAR OF = 0.
//! - Rotates with count > 1: OF computed with the count-1 formula.
//! - MUL/IMUL: SF/ZF/PF from the low half of the product, AF cleared.
//! - DIV/IDIV: flags unchanged (the functions don't take flags).
//! - DAA/DAS: OF = signed overflow of the adjustment add/sub.
//! - AAA/AAS: SF/ZF/PF from the final AL, OF cleared.
//! - AAM: CF/OF/AF cleared. AAD: flags as for `ADD AL, AH*imm`.
//!
//! Shift/rotate counts are masked to 5 bits (80186+ behaviour).

use super::flags::{AF, ARITH, CF, OF, SF, ZF, parity};

/// `#DE`: divisor zero or quotient out of range (vector 0).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DivideError;

// ---------------------------------------------------------------------------
// Width-generic internals. Values are held zero-extended in u32/u64.

const fn mask(bits: u32) -> u32 {
    if bits == 32 {
        u32::MAX
    } else {
        (1 << bits) - 1
    }
}

const fn msb(bits: u32) -> u32 {
    1 << (bits - 1)
}

fn sext(v: u32, bits: u32) -> i32 {
    let s = 32 - bits;
    ((v << s) as i32) >> s
}

/// SF, ZF, PF for `res`.
fn szp(res: u32, bits: u32) -> u32 {
    let mut f = parity(res);
    if res & mask(bits) == 0 {
        f |= ZF;
    }
    if res & msb(bits) != 0 {
        f |= SF;
    }
    f
}

fn bit(cond: bool, flag: u32) -> u32 {
    if cond { flag } else { 0 }
}

fn add_g(a: u32, b: u32, carry: u32, bits: u32, flags: u32) -> (u32, u32) {
    let wide = a as u64 + b as u64 + carry as u64;
    let res = wide as u32 & mask(bits);
    let f = (flags & !ARITH)
        | szp(res, bits)
        | bit(wide >> bits != 0, CF)
        | bit((a ^ b ^ res) & 0x10 != 0, AF)
        | bit((a ^ res) & (b ^ res) & msb(bits) != 0, OF);
    (res, f)
}

fn sub_g(a: u32, b: u32, borrow: u32, bits: u32, flags: u32) -> (u32, u32) {
    let res = a.wrapping_sub(b).wrapping_sub(borrow) & mask(bits);
    let f = (flags & !ARITH)
        | szp(res, bits)
        | bit((b as u64 + borrow as u64) > a as u64, CF)
        | bit((a ^ b ^ res) & 0x10 != 0, AF)
        | bit((a ^ b) & (a ^ res) & msb(bits) != 0, OF);
    (res, f)
}

/// Flags after AND/OR/XOR/TEST: CF=OF=AF=0, SZP from result.
fn logic_g(res: u32, bits: u32, flags: u32) -> u32 {
    (flags & !ARITH) | szp(res, bits)
}

fn inc_g(a: u32, bits: u32, flags: u32) -> (u32, u32) {
    let (r, f) = add_g(a, 1, 0, bits, flags);
    (r, (f & !CF) | (flags & CF))
}

fn dec_g(a: u32, bits: u32, flags: u32) -> (u32, u32) {
    let (r, f) = sub_g(a, 1, 0, bits, flags);
    (r, (f & !CF) | (flags & CF))
}

fn shl_g(a: u32, count: u8, bits: u32, flags: u32) -> (u32, u32) {
    let c = (count & 0x1f) as u32;
    if c == 0 {
        return (a, flags);
    }
    let wide = (a as u64) << c;
    let res = wide as u32 & mask(bits);
    let f = (flags & !ARITH)
        | szp(res, bits)
        | bit((wide >> bits) & 1 != 0, CF)
        | bit((res ^ a) & msb(bits) != 0, OF);
    (res, f)
}

fn shr_g(a: u32, count: u8, bits: u32, flags: u32) -> (u32, u32) {
    let c = (count & 0x1f) as u32;
    if c == 0 {
        return (a, flags);
    }
    let res = a >> c;
    let f = (flags & !ARITH)
        | szp(res, bits)
        | bit((a >> (c - 1)) & 1 != 0, CF)
        | bit(c == 1 && a & msb(bits) != 0, OF);
    (res, f)
}

fn sar_g(a: u32, count: u8, bits: u32, flags: u32) -> (u32, u32) {
    let c = (count & 0x1f) as u32;
    if c == 0 {
        return (a, flags);
    }
    let s = sext(a, bits);
    let res = (s >> c) as u32 & mask(bits);
    let f = (flags & !ARITH) | szp(res, bits) | bit((s >> (c - 1)) & 1 != 0, CF);
    (res, f)
}

fn rol_g(a: u32, count: u8, bits: u32, flags: u32) -> (u32, u32) {
    let c = (count & 0x1f) as u32;
    if c == 0 {
        return (a, flags);
    }
    let r = c % bits;
    let res = if r == 0 {
        a
    } else {
        ((a << r) | (a >> (bits - r))) & mask(bits)
    };
    let cf = res & 1 != 0;
    let of = (res & msb(bits) != 0) ^ cf;
    (res, (flags & !(CF | OF)) | bit(cf, CF) | bit(of, OF))
}

fn ror_g(a: u32, count: u8, bits: u32, flags: u32) -> (u32, u32) {
    let c = (count & 0x1f) as u32;
    if c == 0 {
        return (a, flags);
    }
    let r = c % bits;
    let res = if r == 0 {
        a
    } else {
        ((a >> r) | (a << (bits - r))) & mask(bits)
    };
    let top = res & msb(bits) != 0;
    let next = res & (msb(bits) >> 1) != 0;
    (
        res,
        (flags & !(CF | OF)) | bit(top, CF) | bit(top ^ next, OF),
    )
}

/// Effective RCL/RCR count: 5-bit mask, then mod 9 / 17 for 8/16-bit.
fn rc_count(count: u8, bits: u32) -> u32 {
    let c = (count & 0x1f) as u32;
    if bits == 32 { c } else { c % (bits + 1) }
}

fn rcl_g(a: u32, count: u8, bits: u32, flags: u32) -> (u32, u32) {
    let r = rc_count(count, bits);
    if r == 0 {
        return (a, flags);
    }
    let w = bits + 1;
    let wmask = (1u64 << w) - 1;
    let v = ((((flags & CF) as u64) << bits) | a as u64) & wmask;
    let v = ((v << r) | (v >> (w - r))) & wmask;
    let res = v as u32 & mask(bits);
    let cf = (v >> bits) & 1 != 0;
    let of = (res & msb(bits) != 0) ^ cf;
    (res, (flags & !(CF | OF)) | bit(cf, CF) | bit(of, OF))
}

fn rcr_g(a: u32, count: u8, bits: u32, flags: u32) -> (u32, u32) {
    let r = rc_count(count, bits);
    if r == 0 {
        return (a, flags);
    }
    let w = bits + 1;
    let wmask = (1u64 << w) - 1;
    let v = ((((flags & CF) as u64) << bits) | a as u64) & wmask;
    let v = ((v >> r) | (v << (w - r))) & wmask;
    let res = v as u32 & mask(bits);
    let cf = (v >> bits) & 1 != 0;
    let top = res & msb(bits) != 0;
    let next = res & (msb(bits) >> 1) != 0;
    (
        res,
        (flags & !(CF | OF)) | bit(cf, CF) | bit(top ^ next, OF),
    )
}

fn mul_flags(lo: u32, overflow: bool, bits: u32, flags: u32) -> u32 {
    (flags & !ARITH) | szp(lo, bits) | bit(overflow, CF | OF)
}

fn mul_g(a: u32, b: u32, bits: u32, flags: u32) -> (u64, u32) {
    let p = a as u64 * b as u64;
    let lo = p as u32 & mask(bits);
    (p, mul_flags(lo, p >> bits != 0, bits, flags))
}

fn imul_g(a: u32, b: u32, bits: u32, flags: u32) -> (u64, u32) {
    let p = sext(a, bits) as i64 * sext(b, bits) as i64;
    let lo = p as u32 & mask(bits);
    let fits = p == sext(lo, bits) as i64;
    let full_mask = if bits == 32 {
        u64::MAX
    } else {
        (1u64 << (2 * bits)) - 1
    };
    (p as u64 & full_mask, mul_flags(lo, !fits, bits, flags))
}

fn div_g(n: u64, d: u32, bits: u32) -> Result<(u32, u32), DivideError> {
    if d == 0 {
        return Err(DivideError);
    }
    let q = n / d as u64;
    if q > mask(bits) as u64 {
        return Err(DivideError);
    }
    Ok((q as u32, (n % d as u64) as u32))
}

/// `n` is the double-width dividend (zero-extended bit pattern).
fn idiv_g(n: u64, d: u32, bits: u32) -> Result<(u32, u32), DivideError> {
    let d = sext(d, bits) as i128;
    if d == 0 {
        return Err(DivideError);
    }
    let s = 64 - 2 * bits;
    let n = (((n << s) as i64) >> s) as i128; // i128: i64::MIN / -1 can't overflow
    let q = n / d;
    let r = n % d;
    let lim = 1i128 << (bits - 1);
    // 80186+: a quotient of -2^(bits-1) is valid (the 8086 faults on it).
    if q >= lim || q < -lim {
        return Err(DivideError);
    }
    Ok((q as u32 & mask(bits), r as u32 & mask(bits)))
}

// ---------------------------------------------------------------------------
// Public per-width API.

macro_rules! width_ops {
    ($t:ty, $wide:ty, $bits:expr,
     $add:ident $adc:ident $sub:ident $sbb:ident $cmp:ident
     $and:ident $or:ident $xor:ident $test:ident
     $inc:ident $dec:ident $neg:ident $not:ident
     $shl:ident $shr:ident $sar:ident $rol:ident $ror:ident $rcl:ident $rcr:ident
     $mul:ident $imul:ident $div:ident $idiv:ident) => {
        #[doc = concat!("ADD (", stringify!($bits), "-bit).")]
        pub fn $add(a: $t, b: $t, flags: u32) -> ($t, u32) {
            let (r, f) = add_g(a as u32, b as u32, 0, $bits, flags);
            (r as $t, f)
        }
        #[doc = concat!("ADC (", stringify!($bits), "-bit): adds CF.")]
        pub fn $adc(a: $t, b: $t, flags: u32) -> ($t, u32) {
            let (r, f) = add_g(a as u32, b as u32, flags & CF, $bits, flags);
            (r as $t, f)
        }
        #[doc = concat!("SUB (", stringify!($bits), "-bit).")]
        pub fn $sub(a: $t, b: $t, flags: u32) -> ($t, u32) {
            let (r, f) = sub_g(a as u32, b as u32, 0, $bits, flags);
            (r as $t, f)
        }
        #[doc = concat!("SBB (", stringify!($bits), "-bit): subtracts CF.")]
        pub fn $sbb(a: $t, b: $t, flags: u32) -> ($t, u32) {
            let (r, f) = sub_g(a as u32, b as u32, flags & CF, $bits, flags);
            (r as $t, f)
        }
        #[doc = concat!("CMP (", stringify!($bits), "-bit): flags of `a - b`.")]
        pub fn $cmp(a: $t, b: $t, flags: u32) -> u32 {
            sub_g(a as u32, b as u32, 0, $bits, flags).1
        }
        #[doc = concat!("AND (", stringify!($bits), "-bit).")]
        pub fn $and(a: $t, b: $t, flags: u32) -> ($t, u32) {
            let r = a & b;
            (r, logic_g(r as u32, $bits, flags))
        }
        #[doc = concat!("OR (", stringify!($bits), "-bit).")]
        pub fn $or(a: $t, b: $t, flags: u32) -> ($t, u32) {
            let r = a | b;
            (r, logic_g(r as u32, $bits, flags))
        }
        #[doc = concat!("XOR (", stringify!($bits), "-bit).")]
        pub fn $xor(a: $t, b: $t, flags: u32) -> ($t, u32) {
            let r = a ^ b;
            (r, logic_g(r as u32, $bits, flags))
        }
        #[doc = concat!("TEST (", stringify!($bits), "-bit): flags of `a & b`.")]
        pub fn $test(a: $t, b: $t, flags: u32) -> u32 {
            logic_g((a & b) as u32, $bits, flags)
        }
        #[doc = concat!("INC (", stringify!($bits), "-bit): CF preserved.")]
        pub fn $inc(a: $t, flags: u32) -> ($t, u32) {
            let (r, f) = inc_g(a as u32, $bits, flags);
            (r as $t, f)
        }
        #[doc = concat!("DEC (", stringify!($bits), "-bit): CF preserved.")]
        pub fn $dec(a: $t, flags: u32) -> ($t, u32) {
            let (r, f) = dec_g(a as u32, $bits, flags);
            (r as $t, f)
        }
        #[doc = concat!("NEG (", stringify!($bits), "-bit): `0 - a`, CF = (a != 0).")]
        pub fn $neg(a: $t, flags: u32) -> ($t, u32) {
            let (r, f) = sub_g(0, a as u32, 0, $bits, flags);
            (r as $t, f)
        }
        #[doc = concat!("NOT (", stringify!($bits), "-bit): no flags affected.")]
        pub fn $not(a: $t) -> $t {
            !a
        }
        #[doc = concat!("SHL/SAL (", stringify!($bits), "-bit).")]
        pub fn $shl(a: $t, count: u8, flags: u32) -> ($t, u32) {
            let (r, f) = shl_g(a as u32, count, $bits, flags);
            (r as $t, f)
        }
        #[doc = concat!("SHR (", stringify!($bits), "-bit).")]
        pub fn $shr(a: $t, count: u8, flags: u32) -> ($t, u32) {
            let (r, f) = shr_g(a as u32, count, $bits, flags);
            (r as $t, f)
        }
        #[doc = concat!("SAR (", stringify!($bits), "-bit).")]
        pub fn $sar(a: $t, count: u8, flags: u32) -> ($t, u32) {
            let (r, f) = sar_g(a as u32, count, $bits, flags);
            (r as $t, f)
        }
        #[doc = concat!("ROL (", stringify!($bits), "-bit): only CF/OF change.")]
        pub fn $rol(a: $t, count: u8, flags: u32) -> ($t, u32) {
            let (r, f) = rol_g(a as u32, count, $bits, flags);
            (r as $t, f)
        }
        #[doc = concat!("ROR (", stringify!($bits), "-bit): only CF/OF change.")]
        pub fn $ror(a: $t, count: u8, flags: u32) -> ($t, u32) {
            let (r, f) = ror_g(a as u32, count, $bits, flags);
            (r as $t, f)
        }
        #[doc = concat!("RCL (", stringify!($bits), "-bit): only CF/OF change.")]
        pub fn $rcl(a: $t, count: u8, flags: u32) -> ($t, u32) {
            let (r, f) = rcl_g(a as u32, count, $bits, flags);
            (r as $t, f)
        }
        #[doc = concat!("RCR (", stringify!($bits), "-bit): only CF/OF change.")]
        pub fn $rcr(a: $t, count: u8, flags: u32) -> ($t, u32) {
            let (r, f) = rcr_g(a as u32, count, $bits, flags);
            (r as $t, f)
        }
        #[doc = concat!("Unsigned MUL (", stringify!($bits), "-bit): full double-width product; ")]
        #[doc = "CF=OF set when the high half is non-zero."]
        pub fn $mul(a: $t, b: $t, flags: u32) -> ($wide, u32) {
            let (p, f) = mul_g(a as u32, b as u32, $bits, flags);
            (p as $wide, f)
        }
        #[doc = concat!("Signed IMUL (", stringify!($bits), "-bit): full double-width product ")]
        #[doc = "(two's complement bit pattern); CF=OF set when it does not fit the low half."]
        #[doc = ""]
        #[doc = "The 2/3-operand forms keep the low half and use the same flags."]
        pub fn $imul(a: $t, b: $t, flags: u32) -> ($wide, u32) {
            let (p, f) = imul_g(a as u32, b as u32, $bits, flags);
            (p as $wide, f)
        }
        #[doc = concat!("Unsigned DIV (", stringify!($bits), "-bit): double-width dividend ")]
        #[doc = "-> `(quotient, remainder)`."]
        pub fn $div(n: $wide, d: $t) -> Result<($t, $t), DivideError> {
            div_g(n as u64, d as u32, $bits).map(|(q, r)| (q as $t, r as $t))
        }
        #[doc = concat!("Signed IDIV (", stringify!($bits), "-bit): double-width dividend ")]
        #[doc = "-> `(quotient, remainder)` as bit patterns; remainder takes the dividend's sign."]
        pub fn $idiv(n: $wide, d: $t) -> Result<($t, $t), DivideError> {
            idiv_g(n as u64, d as u32, $bits).map(|(q, r)| (q as $t, r as $t))
        }
    };
}

width_ops!(u8, u16, 8,
    add8 adc8 sub8 sbb8 cmp8 and8 or8 xor8 test8 inc8 dec8 neg8 not8
    shl8 shr8 sar8 rol8 ror8 rcl8 rcr8 mul8 imul8 div8 idiv8);
width_ops!(u16, u32, 16,
    add16 adc16 sub16 sbb16 cmp16 and16 or16 xor16 test16 inc16 dec16 neg16 not16
    shl16 shr16 sar16 rol16 ror16 rcl16 rcr16 mul16 imul16 div16 idiv16);
width_ops!(u32, u64, 32,
    add32 adc32 sub32 sbb32 cmp32 and32 or32 xor32 test32 inc32 dec32 neg32 not32
    shl32 shr32 sar32 rol32 ror32 rcl32 rcr32 mul32 imul32 div32 idiv32);

// ---------------------------------------------------------------------------
// BCD adjust and sign extension.

/// DAA on AL.
pub fn daa(al: u8, flags: u32) -> (u8, u32) {
    let mut adj = 0u8;
    let mut cf = false;
    let mut af = false;
    if al & 0x0f > 9 || flags & AF != 0 {
        adj = 0x06;
        af = true;
    }
    if al > 0x99 || flags & CF != 0 {
        adj |= 0x60;
        cf = true;
    }
    let res = al.wrapping_add(adj);
    let of = (al ^ res) & (adj ^ res) & 0x80 != 0;
    let f = (flags & !ARITH) | szp(res as u32, 8) | bit(cf, CF) | bit(af, AF) | bit(of, OF);
    (res, f)
}

/// DAS on AL.
pub fn das(al: u8, flags: u32) -> (u8, u32) {
    let mut adj = 0u8;
    let mut cf = false;
    let mut af = false;
    if al & 0x0f > 9 || flags & AF != 0 {
        adj = 0x06;
        af = true;
        cf = al < 0x06; // borrow from AL - 6 (SDM: CF = old_CF | borrow)
    }
    if al > 0x99 || flags & CF != 0 {
        adj |= 0x60;
        cf = true;
    }
    let res = al.wrapping_sub(adj);
    let of = (al ^ adj) & (al ^ res) & 0x80 != 0;
    let f = (flags & !ARITH) | szp(res as u32, 8) | bit(cf, CF) | bit(af, AF) | bit(of, OF);
    (res, f)
}

/// AAA on AX (386 form: `AX += 0x106`, carrying from AL into AH).
pub fn aaa(ax: u16, flags: u32) -> (u16, u32) {
    let adjust = ax as u8 & 0x0f > 9 || flags & AF != 0;
    let ax = if adjust { ax.wrapping_add(0x106) } else { ax };
    let ax = ax & 0xff0f;
    let f = (flags & !ARITH) | szp(ax as u8 as u32, 8) | bit(adjust, CF | AF);
    (ax, f)
}

/// AAS on AX (386 form: `AX -= 6; AH -= 1`, borrowing from AH).
pub fn aas(ax: u16, flags: u32) -> (u16, u32) {
    let adjust = ax as u8 & 0x0f > 9 || flags & AF != 0;
    let ax = if adjust {
        ax.wrapping_sub(6).wrapping_sub(0x100)
    } else {
        ax
    };
    let ax = ax & 0xff0f;
    let f = (flags & !ARITH) | szp(ax as u8 as u32, 8) | bit(adjust, CF | AF);
    (ax, f)
}

/// AAM imm on AL -> AX. `imm == 0` raises #DE.
pub fn aam(al: u8, imm: u8, flags: u32) -> Result<(u16, u32), DivideError> {
    if imm == 0 {
        return Err(DivideError);
    }
    let ah = al / imm;
    let al = al % imm;
    let f = (flags & !ARITH) | szp(al as u32, 8);
    Ok((((ah as u16) << 8) | al as u16, f))
}

/// AAD imm on AX -> AX (`AL = AL + AH*imm`, `AH = 0`).
pub fn aad(ax: u16, imm: u8, flags: u32) -> (u16, u32) {
    let ah = (ax >> 8) as u8;
    let (al, f) = add8(ax as u8, ah.wrapping_mul(imm), flags);
    (al as u16, f)
}

/// CBW: AL sign-extended to AX.
pub fn cbw(al: u8) -> u16 {
    al as i8 as i16 as u16
}

/// CWD: the DX value for AX sign-extended to DX:AX.
pub fn cwd(ax: u16) -> u16 {
    if ax & 0x8000 != 0 { 0xffff } else { 0 }
}

/// CWDE: AX sign-extended to EAX.
pub fn cwde(ax: u16) -> u32 {
    ax as i16 as i32 as u32
}

/// CDQ: the EDX value for EAX sign-extended to EDX:EAX.
pub fn cdq(eax: u32) -> u32 {
    if eax & 0x8000_0000 != 0 { u32::MAX } else { 0 }
}

#[cfg(test)]
mod tests;
