use super::*;
use crate::cpu::flags::{DF, IF, PF, RESERVED1};

/// Non-ALU bits that must always pass through untouched.
const KEEP: u32 = RESERVED1 | IF | DF;

fn arith(f: u32) -> u32 {
    f & ARITH
}

#[test]
fn add_basic_and_flags() {
    assert_eq!(add8(1, 2, 0), (3, PF));
    assert_eq!(add8(0xff, 1, 0), (0, CF | ZF | PF | AF));
    assert_eq!(add8(0x7f, 1, 0), (0x80, SF | OF | AF));
    assert_eq!(add8(0x80, 0x80, 0), (0, CF | ZF | PF | OF));
    assert_eq!(add8(0x0f, 0x01, 0), (0x10, AF));
    assert_eq!(add16(0xffff, 1, 0), (0, CF | ZF | PF | AF));
    assert_eq!(add16(0x7fff, 1, 0), (0x8000, SF | OF | AF | PF));
    assert_eq!(add16(0x00ff, 1, 0), (0x0100, AF | PF));
    assert_eq!(add32(0xffff_ffff, 1, 0), (0, CF | ZF | PF | AF));
    assert_eq!(add32(0x7fff_ffff, 1, 0).1, SF | OF | AF | PF);
}

#[test]
fn ops_preserve_non_arith_bits() {
    let (_, f) = add8(1, 1, KEEP | ARITH);
    assert_eq!(f & KEEP, KEEP);
    assert_eq!(arith(f), 0);
    assert_eq!(test16(0, 0, KEEP) & !ARITH, KEEP);
    assert_eq!(shl8(1, 1, KEEP).1 & !ARITH, KEEP);
    assert_eq!(rcl16(1, 1, KEEP).1 & !ARITH, KEEP);
    assert_eq!(mul8(2, 3, KEEP).1 & !ARITH, KEEP);
}

#[test]
fn adc_uses_carry() {
    assert_eq!(adc8(0xff, 0, CF), (0, CF | ZF | PF | AF));
    assert_eq!(adc8(0xff, 0xff, CF), (0xff, CF | SF | PF | AF));
    assert_eq!(adc8(0x7f, 0, CF), (0x80, SF | OF | AF));
    assert_eq!(adc16(1, 1, 0), (2, 0));
    assert_eq!(adc32(0xffff_ffff, 0xffff_ffff, CF).0, 0xffff_ffff);
    assert_ne!(adc32(0xffff_ffff, 0xffff_ffff, CF).1 & CF, 0);
}

#[test]
fn sub_and_sbb() {
    assert_eq!(sub8(0, 1, 0), (0xff, CF | SF | PF | AF));
    assert_eq!(sub8(0x80, 1, 0), (0x7f, OF | AF));
    assert_eq!(sub8(5, 5, 0), (0, ZF | PF));
    assert_eq!(sub8(0x10, 1, 0), (0x0f, AF | PF));
    assert_eq!(sub16(0x8000, 1, 0), (0x7fff, OF | AF | PF));
    assert_eq!(sub32(0, 1, 0).1, CF | SF | PF | AF);
    // SBB with borrow-in: 0 - 0xff - 1 = 0 with borrow.
    assert_eq!(sbb8(0, 0xff, CF), (0, CF | ZF | PF | AF));
    assert_eq!(sbb8(5, 5, CF), (0xff, CF | SF | PF | AF));
    assert_eq!(sbb16(0x8000, 0, CF), (0x7fff, OF | AF | PF));
    assert_eq!(sbb32(1, 0, CF), (0, ZF | PF));
}

#[test]
fn cmp_matches_sub_and_test_matches_and() {
    for (a, b) in [(0u8, 1u8), (0x80, 1), (5, 5), (0x10, 0x01), (0xff, 0x7f)] {
        assert_eq!(cmp8(a, b, KEEP), sub8(a, b, KEEP).1);
        assert_eq!(test8(a, b, KEEP), and8(a, b, KEEP).1);
    }
    assert_eq!(cmp16(1, 2, 0), sub16(1, 2, 0).1);
    assert_eq!(cmp32(1, 2, 0), sub32(1, 2, 0).1);
}

#[test]
fn logic_clears_cf_of_af() {
    assert_eq!(and8(0xf0, 0x0f, ARITH), (0, ZF | PF));
    assert_eq!(or8(0x80, 0x01, ARITH), (0x81, SF | PF));
    assert_eq!(xor8(0xff, 0xfe, ARITH), (0x01, 0));
    assert_eq!(xor16(0xffff, 0x7fff, 0), (0x8000, SF | PF));
    assert_eq!(or32(0, 0, ARITH), (0, ZF | PF));
    assert_eq!(and32(0x8000_0000, 0xffff_ffff, 0).1, SF | PF);
    // PF looks at the low byte only.
    assert_eq!(or16(0x0100, 0, 0).1, PF);
    assert_eq!(or16(0x0101, 0, 0).1, 0);
}

#[test]
fn inc_dec_preserve_cf() {
    assert_eq!(inc8(0xff, CF), (0, CF | ZF | PF | AF));
    assert_eq!(inc8(0xff, 0), (0, ZF | PF | AF));
    assert_eq!(inc8(0x7f, 0), (0x80, SF | OF | AF));
    assert_eq!(dec8(0, 0), (0xff, SF | PF | AF));
    assert_eq!(dec8(0x80, CF), (0x7f, CF | OF | AF));
    assert_eq!(inc16(0x7fff, 0).1, SF | OF | AF | PF);
    assert_eq!(dec16(1, CF), (0, CF | ZF | PF));
    assert_eq!(inc32(0xffff_ffff, 0), (0, ZF | PF | AF));
    assert_eq!(dec32(0x8000_0000, 0).1, OF | AF | PF);
}

#[test]
fn neg_and_not() {
    assert_eq!(neg8(0, ARITH), (0, ZF | PF));
    assert_eq!(neg8(1, 0), (0xff, CF | SF | PF | AF));
    assert_eq!(neg8(0x80, 0), (0x80, CF | SF | OF));
    assert_eq!(neg16(0x8000, 0), (0x8000, CF | SF | OF | PF));
    assert_eq!(neg32(5, 0).0, 0xffff_fffb);
    assert_eq!(not8(0x0f), 0xf0);
    assert_eq!(not16(0), 0xffff);
    assert_eq!(not32(0x1234_5678), 0xedcb_a987);
}

#[test]
fn shift_count_zero_is_noop() {
    let f = KEEP | ARITH;
    assert_eq!(shl8(0x81, 0, f), (0x81, f));
    assert_eq!(shr16(0x81, 0, f), (0x81, f));
    assert_eq!(sar32(0x81, 0, f), (0x81, f));
    assert_eq!(rol8(0x81, 0, f), (0x81, f));
    assert_eq!(ror16(0x81, 0, f), (0x81, f));
    assert_eq!(rcl8(0x81, 0, f), (0x81, f));
    assert_eq!(rcr32(0x81, 0, f), (0x81, f));
    // Count is masked to 5 bits: 32 behaves like 0.
    assert_eq!(shl8(0x81, 32, f), (0x81, f));
    assert_eq!(shl32(0x81, 32, f), (0x81, f));
    assert_eq!(rol16(0x81, 0x20, f), (0x81, f));
}

#[test]
fn shl_edges() {
    assert_eq!(shl8(0x81, 1, 0), (0x02, CF | OF));
    assert_eq!(shl8(0x40, 1, 0), (0x80, SF | OF));
    assert_eq!(shl8(0xc0, 1, 0), (0x80, CF | SF));
    assert_eq!(shl8(0x01, 8, 0), (0x00, CF | ZF | PF));
    assert_eq!(shl8(0xff, 9, 0), (0x00, ZF | PF | OF));
    assert_eq!(shl8(0xff, 31, 0).0, 0);
    assert_eq!(shl8(0xff, 31, 0).1 & CF, 0);
    assert_eq!(shl16(0x0001, 16, 0), (0, CF | ZF | PF));
    assert_eq!(shl16(0x0001, 15, 0), (0x8000, SF | PF | OF));
    assert_eq!(shl16(0x8000, 1, 0), (0, CF | ZF | PF | OF));
    assert_eq!(shl32(1, 31, 0), (0x8000_0000, SF | PF | OF));
    // OF = MSB(res) ^ MSB(a) for count > 1 (DOSBox).
    assert_eq!(shl32(3, 31, 0), (0x8000_0000, CF | SF | PF | OF));
    assert_eq!(shl32(0xc000_0000, 1, 0), (0x8000_0000, CF | SF | PF));
    assert_eq!(shl32(0x8000_0000, 1, 0), (0, CF | ZF | PF | OF));
}

#[test]
fn shr_edges() {
    assert_eq!(shr8(0x81, 1, 0), (0x40, CF | OF));
    assert_eq!(shr8(0x01, 1, 0), (0, CF | ZF | PF));
    assert_eq!(shr8(0x80, 8, 0), (0, CF | ZF | PF));
    assert_eq!(shr8(0x80, 9, 0), (0, ZF | PF));
    assert_eq!(shr8(0xff, 2, 0), (0x3f, CF | PF)); // OF only for count 1
    assert_eq!(shr16(0x8000, 16, 0), (0, CF | ZF | PF));
    assert_eq!(shr16(0x8000, 15, 0), (1, 0));
    assert_eq!(shr16(0xffff, 31, 0), (0, ZF | PF));
    assert_eq!(shr32(0x8000_0000, 31, 0), (1, 0));
    assert_eq!(shr32(0xc000_0000, 31, 0), (1, CF));
    assert_eq!(shr32(0x8000_0001, 1, 0), (0x4000_0000, CF | PF | OF));
}

#[test]
fn sar_edges() {
    assert_eq!(sar8(0x81, 1, 0), (0xc0, CF | SF | PF));
    assert_eq!(sar8(0x80, 7, 0), (0xff, SF | PF));
    assert_eq!(sar8(0x80, 8, 0), (0xff, CF | SF | PF));
    assert_eq!(sar8(0x80, 31, 0), (0xff, CF | SF | PF));
    assert_eq!(sar8(0x7f, 8, 0), (0, ZF | PF));
    assert_eq!(sar16(0x8000, 16, 0), (0xffff, CF | SF | PF));
    assert_eq!(sar16(0x4000, 16, 0), (0, ZF | PF));
    assert_eq!(sar32(0x8000_0000, 31, 0), (0xffff_ffff, SF | PF));
    assert_eq!(sar32(0x8000_0001, 1, 0), (0xc000_0000, CF | SF | PF));
}

#[test]
fn rol_ror_edges() {
    // Only CF/OF change.
    let other = KEEP | ZF | SF | PF | AF;
    assert_eq!(rol8(0x81, 1, other), (0x03, other | CF | OF));
    assert_eq!(rol8(0x40, 1, 0), (0x80, OF));
    assert_eq!(rol8(0x01, 8, 0), (0x01, CF | OF)); // count%8==0 still sets CF/OF
    assert_eq!(rol8(0x80, 8, CF | OF), (0x80, OF));
    assert_eq!(rol8(0x01, 9, 0), (0x02, 0));
    assert_eq!(rol16(0x8001, 16, 0), (0x8001, CF));
    assert_eq!(rol16(0x8000, 1, 0), (0x0001, CF | OF));
    assert_eq!(rol32(0x8000_0000, 1, 0), (1, CF | OF));
    assert_eq!(rol32(0x1234_5678, 31, 0).0, 0x1234_5678u32.rotate_left(31));
    assert_eq!(ror8(0x01, 1, other), (0x80, other | CF | OF));
    assert_eq!(ror8(0x02, 1, 0), (0x01, 0));
    assert_eq!(ror8(0x03, 1, 0), (0x81, CF | OF));
    assert_eq!(ror8(0xc0, 1, 0), (0x60, OF));
    assert_eq!(ror8(0x80, 8, 0), (0x80, CF | OF));
    assert_eq!(ror8(0xc0, 16, 0), (0xc0, CF));
    assert_eq!(ror16(0x0001, 16, 0), (0x0001, 0));
    assert_eq!(ror16(0x0001, 1, 0), (0x8000, CF | OF));
    assert_eq!(ror32(1, 31, 0), (2, 0));
    assert_eq!(ror32(1, 1, 0), (0x8000_0000, CF | OF));
}

#[test]
fn rcl_rcr_edges() {
    assert_eq!(rcl8(0x80, 1, 0), (0x00, CF | OF));
    assert_eq!(rcl8(0x00, 1, CF), (0x01, 0));
    assert_eq!(rcl8(0x40, 1, 0), (0x80, OF));
    // 9-bit rotate: count 9 is a no-op (flags untouched), 8 moves CF to bit 7.
    assert_eq!(rcl8(0x55, 9, CF | OF), (0x55, CF | OF));
    assert_eq!(rcl8(0x01, 8, CF), (0x80, CF));
    assert_eq!(rcl16(0x8000, 1, 0), (0, CF | OF));
    assert_eq!(rcl16(0x1234, 17, CF), (0x1234, CF));
    assert_eq!(rcl16(0x0001, 16, 0), (0x0000, CF | OF));
    assert_eq!(rcl32(0x8000_0000, 1, 0), (0, CF | OF));
    assert_eq!(rcl32(0, 31, CF), (0x4000_0000, 0));
    assert_eq!(rcr8(0x01, 1, 0), (0x00, CF));
    assert_eq!(rcr8(0x00, 1, CF), (0x80, OF));
    assert_eq!(rcr8(0x55, 9, 0), (0x55, 0));
    assert_eq!(rcr8(0x80, 8, 0), (0x00, CF));
    assert_eq!(rcr16(0x0001, 16, 0), (0x0002, 0)); // 17-bit: right 16 == left 1
    assert_eq!(rcr16(0x0001, 1, 0), (0x0000, CF));
    assert_eq!(rcr16(0x0000, 16, CF), (0x0001, 0));
    assert_eq!(rcr32(1, 1, CF), (0x8000_0000, CF | OF));
    assert_eq!(rcr32(0, 31, CF), (2, 0));
}

#[test]
fn rotates_match_reference_for_all_counts() {
    // Cross-check against a bit-by-bit reference rotate through carry.
    for count in 0u8..32 {
        for &a in &[0x00u8, 0x01, 0x80, 0x5a, 0xff] {
            for cf in [0, CF] {
                let (mut v, mut c) = (a, cf != 0);
                for _ in 0..(count % 9) {
                    let out = v & 0x80 != 0;
                    v = (v << 1) | c as u8;
                    c = out;
                }
                assert_eq!(rcl8(a, count, cf).0, v, "rcl8 {a:#x} {count}");
                if count % 9 != 0 {
                    assert_eq!(rcl8(a, count, cf).1 & CF != 0, c);
                }
                let (mut v, mut c) = (a, cf != 0);
                for _ in 0..(count % 9) {
                    let out = v & 1 != 0;
                    v = (v >> 1) | ((c as u8) << 7);
                    c = out;
                }
                assert_eq!(rcr8(a, count, cf).0, v, "rcr8 {a:#x} {count}");
                if count % 9 != 0 {
                    assert_eq!(rcr8(a, count, cf).1 & CF != 0, c);
                }
            }
            assert_eq!(rol8(a, count, 0).0, a.rotate_left(count as u32));
            assert_eq!(ror8(a, count, 0).0, a.rotate_right(count as u32));
        }
    }
}

#[test]
fn mul_flags() {
    assert_eq!(mul8(0x10, 0x10, 0), (0x0100, CF | OF | ZF | PF));
    assert_eq!(mul8(0x0f, 0x11, 0), (0x00ff, PF | SF));
    assert_eq!(mul8(0xff, 0xff, 0).0, 0xfe01);
    assert_eq!(mul16(0xffff, 0xffff, 0).0, 0xfffe_0001);
    assert_ne!(mul16(0xffff, 0xffff, 0).1 & (CF | OF), 0);
    assert_eq!(mul16(0x100, 0xff, 0).1 & (CF | OF), 0);
    assert_eq!(mul32(0xffff_ffff, 2, 0).0, 0x1_ffff_fffe);
    assert_eq!(mul32(0xffff, 0x10001, 0).1 & (CF | OF), 0);
}

#[test]
fn imul_flags() {
    // -1 * -1 = 1 fits.
    assert_eq!(imul8(0xff, 0xff, 0), (0x0001, 0));
    // -128 * 1 fits; -128 * -1 = 128 does not.
    assert_eq!(imul8(0x80, 0x01, 0), (0xff80, SF));
    assert_eq!(imul8(0x80, 0xff, 0), (0x0080, CF | OF | SF));
    // 64 * 2 = 128 does not fit in i8.
    assert_eq!(imul8(0x40, 0x02, 0).1 & (CF | OF), CF | OF);
    assert_eq!(imul16(0xffff, 0x0002, 0), (0xffff_fffe, SF));
    assert_eq!(imul16(0x4000, 0x0002, 0).1 & (CF | OF), CF | OF);
    assert_eq!(imul32(0xffff_ffff, 0xffff_ffff, 0), (1, 0));
    assert_eq!(imul32(0x8000_0000, 0xffff_ffff, 0).0, 0x0000_0000_8000_0000);
    assert_eq!(imul32(0x8000_0000, 0xffff_ffff, 0).1 & (CF | OF), CF | OF);
    assert_eq!(imul32(0x8000_0000, 1, 0).0, 0xffff_ffff_8000_0000);
}

#[test]
fn div_and_divide_errors() {
    assert_eq!(div8(100, 7), Ok((14, 2)));
    assert_eq!(div8(0x00ff, 1), Ok((0xff, 0)));
    assert_eq!(div8(0x0100, 1), Err(DivideError));
    assert_eq!(div8(5, 0), Err(DivideError));
    assert_eq!(div16(0x0001_0000, 2), Ok((0x8000, 0)));
    assert_eq!(div16(0xffff_ffff, 0xffff), Err(DivideError));
    assert_eq!(div16(0xfffe_ffff, 0xffff), Ok((0xffff, 0xfffe)));
    assert_eq!(
        div32(0xffff_fffe_ffff_ffff, 0xffff_ffff),
        Ok((0xffff_ffff, 0xffff_fffe))
    );
    assert_eq!(div32(1 << 32, 1), Err(DivideError));
    assert_eq!(div32(1, 0), Err(DivideError));
}

#[test]
fn idiv_signs_and_range() {
    // -7 / 2 = -3 rem -1 (truncation toward zero, remainder keeps dividend sign).
    assert_eq!(idiv8((-7i16) as u16, 2), Ok(((-3i8) as u8, (-1i8) as u8)));
    assert_eq!(idiv8(7, (-2i8) as u8), Ok(((-3i8) as u8, 1)));
    assert_eq!(idiv8(127, 1), Ok((127, 0)));
    assert_eq!(idiv8(128, 1), Err(DivideError));
    assert_eq!(idiv8((-128i16) as u16, 1), Ok((0x80, 0))); // 186+ allows -128
    assert_eq!(idiv8((-129i16) as u16, 1), Err(DivideError));
    assert_eq!(idiv8(1, 0), Err(DivideError));
    assert_eq!(idiv16((-32768i32) as u32, 0xffff), Err(DivideError));
    assert_eq!(idiv16((-32768i32) as u32, 1), Ok((0x8000, 0)));
    assert_eq!(idiv16(100_000, 1000), Ok((100, 0)));
    assert_eq!(idiv32(i64::MIN as u64, u32::MAX), Err(DivideError));
    assert_eq!(
        idiv32((-10i64) as u64, 3),
        Ok(((-3i32) as u32, (-1i32) as u32))
    );
    assert_eq!(idiv32((i32::MIN as i64) as u64, 1), Ok((0x8000_0000, 0)));
}

#[test]
fn daa_das() {
    // 0x15 + 0x27 = 0x3c -> 0x42.
    assert_eq!(daa(0x3c, 0), (0x42, AF | PF));
    assert_eq!(daa(0x9a, 0), (0x00, CF | AF | ZF | PF));
    assert_eq!(daa(0x00, CF), (0x60, CF | PF));
    assert_eq!(daa(0x12, AF), (0x18, AF | PF));
    assert_eq!(daa(0x79, 0), (0x79, 0));
    assert_eq!(daa(0x7a, 0).0, 0x80);
    assert_ne!(daa(0x7a, 0).1 & OF, 0);
    // 0x42 - 0x15 = 0x2d -> 0x27.
    assert_eq!(das(0x2d, 0), (0x27, AF | PF));
    assert_eq!(das(0x00, CF), (0xa0, CF | SF | PF));
    assert_eq!(das(0x9a, 0), (0x34, CF | AF | OF));
    // Borrow from AL - 6 sets CF even without the 0x60 adjust.
    assert_eq!(das(0x03, AF), (0xfd, CF | AF | SF));
    assert_eq!(das(0x80, AF).0, 0x7a);
    assert_ne!(das(0x80, AF).1 & OF, 0);
}

#[test]
fn aaa_aas() {
    assert_eq!(aaa(0x000a, 0), (0x0100, CF | AF | ZF | PF));
    assert_eq!(aaa(0x0005, 0), (0x0005, PF));
    assert_eq!(aaa(0x0005, AF), (0x010b & 0xff0f, CF | AF));
    assert_eq!(aaa(0x00ff, 0), (0x0205, CF | AF | PF)); // 386: carry into AH
    assert_eq!(aaa(0xff0a, 0).0, 0x0000);
    assert_eq!(aas(0x010a, 0), (0x0004, CF | AF));
    assert_eq!(aas(0x0205, 0), (0x0205, PF));
    assert_eq!(aas(0x0005, AF), (0xfe0f, CF | AF | PF)); // 386: borrow from AH
}

#[test]
fn aam_aad() {
    assert_eq!(aam(63, 10, 0), Ok((0x0603, PF)));
    assert_eq!(aam(0, 10, 0), Ok((0, ZF | PF)));
    assert_eq!(aam(0xff, 16, 0), Ok((0x0f0f, PF)));
    assert_eq!(aam(5, 0, 0), Err(DivideError));
    assert_eq!(aad(0x0603, 10, 0), (63, PF));
    assert_eq!(aad(0x0f0f, 16, 0), (0xff, SF | PF));
    assert_eq!(aad(0x0000, 10, 0), (0, ZF | PF));
    // 0x80 + 0x80 overflows like ADD.
    assert_eq!(aad(0x0880, 16, 0).1, add8(0x80, 0x80, 0).1);
}

#[test]
fn sign_extension() {
    assert_eq!(cbw(0x7f), 0x007f);
    assert_eq!(cbw(0x80), 0xff80);
    assert_eq!(cwd(0x7fff), 0);
    assert_eq!(cwd(0x8000), 0xffff);
    assert_eq!(cwde(0x8000), 0xffff_8000);
    assert_eq!(cwde(0x1234), 0x1234);
    assert_eq!(cdq(0x8000_0000), u32::MAX);
    assert_eq!(cdq(0x7fff_ffff), 0);
}

#[test]
fn shifts_match_reference_for_all_counts() {
    // Bit-by-bit reference: result and CF for counts 1..=31 (count 0 is a no-op).
    for count in 1u8..32 {
        for &a in &[0x0000u16, 0x0001, 0x8000, 0x5aa5, 0xffff] {
            let (mut l, mut lc) = (a, false);
            let (mut r, mut rc) = (a, false);
            let (mut s, mut sc) = (a, false);
            for _ in 0..count {
                lc = l & 0x8000 != 0;
                l <<= 1;
                rc = r & 1 != 0;
                r >>= 1;
                sc = s & 1 != 0;
                s = ((s as i16) >> 1) as u16;
            }
            assert_eq!(shl16(a, count, 0).0, l);
            assert_eq!(shl16(a, count, 0).1 & CF != 0, lc, "shl16 {a:#x} {count}");
            assert_eq!(shr16(a, count, 0).0, r);
            assert_eq!(shr16(a, count, 0).1 & CF != 0, rc, "shr16 {a:#x} {count}");
            assert_eq!(sar16(a, count, 0).0, s);
            assert_eq!(sar16(a, count, 0).1 & CF != 0, sc, "sar16 {a:#x} {count}");
            assert_eq!(shl8(a as u8, count, 0).0, ((a as u8 as u32) << count) as u8);
        }
    }
}
