//! 8086 ALU, group, string and I/O opcodes on `FlatBus`.

use super::flags::{CF, DF, OF, SF, ZF};
use super::tests::{CODE, DATA, STACK, run, setup};
use super::*;
use crate::bus::{FlatBus, linear};

/// Run `code` to HLT.
fn exec(code: &[u8]) -> (Cpu, FlatBus) {
    let (mut cpu, mut bus) = setup(code);
    run(&mut cpu, &mut bus);
    (cpu, bus)
}

#[test]
fn alu_forms_00_3f() {
    #[rustfmt::skip]
    let (mut cpu, mut bus) = setup(&[
        0xB8, 0x05, 0x00, // mov ax, 5
        0xBB, 0x10, 0x00, // mov bx, 10h
        0x01, 0x07,       // add [bx], ax
        0x03, 0x07,       // add ax, [bx]
        0x0C, 0x01,       // or al, 1
        0x2D, 0x0B, 0x00, // sub ax, 0Bh
        0xF9, 0x14, 0x00, // stc ; adc al, 0
        0xF9, 0x1C, 0x00, // stc ; sbb al, 0
        0x25, 0x00, 0xFF, // and ax, FF00h
        0xB0, 0x03,       // mov al, 3
        0x02, 0xE0,       // add ah, al
        0x00, 0xC0,       // add al, al
        0x31, 0xC9,       // xor cx, cx
        0x3D, 0x06, 0x04, // cmp ax, 0406h
        0xF4,
    ]);
    bus.write16(linear(DATA, 0x10), 0x0100);
    run(&mut cpu, &mut bus);
    assert_eq!(bus.read16(linear(DATA, 0x10)), 0x0105);
    assert_eq!((cpu.regs.ax(), cpu.regs.cx()), (0x0406, 0));
    assert_eq!(cpu.flags & (ZF | CF), ZF);
}

#[test]
fn group1_immediates() {
    #[rustfmt::skip]
    let (mut cpu, mut bus) = setup(&[
        0xBB, 0x20, 0x00,             // mov bx, 20h
        0x80, 0x07, 0xFF,             // add byte [bx], FFh
        0x83, 0x47, 0x02, 0xFF,       // add word [bx+2], -1
        0x81, 0x6F, 0x02, 0xFF, 0x0F, // sub word [bx+2], 0FFFh
        0x82, 0x0F, 0x80,             // or byte [bx], 80h (82 = 80 alias)
        0x80, 0x3F, 0x80,             // cmp byte [bx], 80h
        0xF4,
    ]);
    bus.write8(linear(DATA, 0x20), 0x01);
    bus.write16(linear(DATA, 0x22), 0x1000);
    run(&mut cpu, &mut bus);
    assert_eq!(bus.read8(linear(DATA, 0x20)), 0x80);
    assert_eq!(bus.read16(linear(DATA, 0x22)), 0);
    assert_ne!(cpu.flags & ZF, 0);
}

#[test]
fn test_inc_dec() {
    #[rustfmt::skip]
    let (mut cpu, mut bus) = setup(&[
        0xB0, 0xF0,             // mov al, F0h
        0xA8, 0x0F,             // test al, 0Fh  -> ZF
        0xF9,                   // stc
        0x40,                   // inc ax
        0x4B,                   // dec bx -> FFFF
        0xFE, 0x06, 0x30, 0x00, // inc byte [30h]  FF -> 00
        0xFF, 0x0E, 0x32, 0x00, // dec word [32h]  0 -> FFFF
        0xF4,
    ]);
    bus.write8(linear(DATA, 0x30), 0xFF);
    for _ in 0..2 {
        cpu.step(&mut bus);
    }
    assert_ne!(cpu.flags & ZF, 0);
    run(&mut cpu, &mut bus);
    assert_eq!((cpu.regs.ax(), cpu.regs.bx()), (0xF1, 0xFFFF));
    assert_eq!(bus.read8(linear(DATA, 0x30)), 0);
    assert_eq!(bus.read16(linear(DATA, 0x32)), 0xFFFF);
    assert_eq!(cpu.flags & (CF | SF), CF | SF, "INC/DEC keep CF");

    let (cpu, _) = exec(&[0xB0, 0x0F, 0xB4, 0x0F, 0x84, 0xE0, 0xF4]); // test al, ah
    assert_eq!(cpu.flags & ZF, 0);
}

#[test]
fn group3_mul_div_not_neg() {
    // mov al,10h ; mov bl,10h ; mul bl
    let (cpu, _) = exec(&[0xB0, 0x10, 0xB3, 0x10, 0xF6, 0xE3, 0xF4]);
    assert_eq!(cpu.regs.ax(), 0x100);
    assert_eq!(cpu.flags & (CF | OF), CF | OF);
    // mov ax,-2 ; mov cx,3 ; imul cx
    let (cpu, _) = exec(&[0xB8, 0xFE, 0xFF, 0xB9, 0x03, 0x00, 0xF7, 0xE9, 0xF4]);
    assert_eq!((cpu.regs.dx(), cpu.regs.ax()), (0xFFFF, 0xFFFA));
    assert_eq!(cpu.flags & (CF | OF), 0);
    // mov dx,1 ; mov ax,0 ; mov cx,100h ; div cx
    #[rustfmt::skip]
    let (cpu, _) = exec(&[0xBA, 0x01, 0x00, 0xB8, 0x00, 0x00, 0xB9, 0x00, 0x01,
        0xF7, 0xF1, 0xF4]);
    assert_eq!((cpu.regs.ax(), cpu.regs.dx()), (0x100, 0));
    // mov ax,-7 ; mov bl,2 ; idiv bl -> AL=-3, AH=-1
    let (cpu, _) = exec(&[0xB8, 0xF9, 0xFF, 0xB3, 0x02, 0xF6, 0xFB, 0xF4]);
    assert_eq!(cpu.regs.ax(), 0xFFFD);
    // mov bl,2 ; not bl ; mov cx,100h ; neg cx ; test bl,0FDh (F6 /0)
    #[rustfmt::skip]
    let (cpu, _) = exec(&[0xB3, 0x02, 0xF6, 0xD3, 0xB9, 0x00, 0x01, 0xF7, 0xD9,
        0xF4]);
    assert_eq!((cpu.regs.bl(), cpu.regs.cx()), (0xFD, 0xFF00));
    assert_ne!(cpu.flags & CF, 0);
    // F6 /0 imm: mov al,3 ; test al,4
    let (cpu, _) = exec(&[0xB0, 0x03, 0xF6, 0xC0, 0x04, 0xF4]);
    assert_ne!(cpu.flags & ZF, 0);
}

/// INT 0 handler at 5000:0000 (HLT); returns the pushed return IP.
fn divide_error(code: &[u8], setup_regs: impl Fn(&mut Cpu)) -> u16 {
    let (mut cpu, mut bus) = setup(code);
    setup_regs(&mut cpu);
    bus.write16(0, 0);
    bus.write16(2, 0x5000);
    bus.write8(linear(0x5000, 0), 0xF4);
    run(&mut cpu, &mut bus);
    assert_eq!(cpu.sregs.get(Seg::Cs), 0x5000, "INT 0 taken");
    assert_eq!(bus.read16(linear(STACK, 0xFC)), CODE);
    bus.read16(linear(STACK, 0xFA))
}

#[test]
fn divide_errors_vector_to_int0_at_the_instruction() {
    // nop ; es: div bl  with BL = 0
    assert_eq!(divide_error(&[0x90, 0x26, 0xF6, 0xF3], |_| {}), 1);
    // quotient overflow: AX=FFFF / 1 does not fit AL
    let ip = divide_error(&[0xF6, 0xF3], |c| {
        c.regs.set_ax(0xFFFF);
        c.regs.set_bl(1);
    });
    assert_eq!(ip, 0);
    // div word [0] (zero) and aam 0
    assert_eq!(divide_error(&[0xF7, 0x36, 0x00, 0x00], |_| {}), 0);
    assert_eq!(divide_error(&[0xD4, 0x00], |_| {}), 0);
}

#[test]
fn shifts_and_rotates() {
    #[rustfmt::skip]
    let (cpu, _) = exec(&[
        0xB0, 0x81,       // mov al, 81h
        0xD0, 0xC0,       // rol al, 1     -> 03, CF
        0xB1, 0x04,       // mov cl, 4
        0xBB, 0x34, 0x12, // mov bx, 1234h
        0xD3, 0xEB,       // shr bx, cl    -> 0123
        0xD1, 0xFB,       // sar bx, 1     -> 0091, CF
        0xF4,
    ]);
    assert_eq!((cpu.regs.al(), cpu.regs.bx()), (0x03, 0x91));
    assert_ne!(cpu.flags & CF, 0);
    // D2 /4 shl byte with CL: mov al,1 ; mov cl,7 ; shl al,cl
    let (cpu, _) = exec(&[0xB0, 0x01, 0xB1, 0x07, 0xD2, 0xE0, 0xF4]);
    assert_eq!(cpu.regs.al(), 0x80);
}

/// String-op setup: DS:SI = DATA:si, ES:DI = 4000:di.
fn strings(code: &[u8], si: u16, di: u16, cx: u16) -> (Cpu, FlatBus) {
    let (mut cpu, bus) = setup(code);
    cpu.sregs.set(Seg::Es, 0x4000);
    cpu.regs.set_si(si);
    cpu.regs.set_di(di);
    cpu.regs.set_cx(cx);
    (cpu, bus)
}

#[test]
fn rep_movsb_one_iteration_per_step() {
    let (mut cpu, mut bus) = strings(&[0xF3, 0xA4, 0xF4], 0, 0x100, 5);
    bus.load(linear(DATA, 0), b"hello");
    assert_eq!(cpu.step(&mut bus), (Step::Ok, 1));
    assert_eq!((cpu.ip, cpu.regs.cx()), (0, 4), "IP back at the prefix");
    run(&mut cpu, &mut bus);
    assert_eq!(&bus.mem[linear(0x4000, 0x100) as usize..][..5], b"hello");
    assert_eq!((cpu.regs.cx(), cpu.regs.si(), cpu.regs.di()), (0, 5, 0x105));
    assert_eq!(cpu.ip, 3);
}

#[test]
fn rep_with_cx_zero_does_nothing() {
    let (mut cpu, mut bus) = strings(&[0xF3, 0xAA, 0xF4], 0, 0, 0);
    cpu.regs.set_al(0x55);
    run(&mut cpu, &mut bus);
    assert_eq!(bus.read8(linear(0x4000, 0)), 0);
    assert_eq!((cpu.regs.di(), cpu.ip), (0, 3));
}

#[test]
fn rep_wraps_within_segment_and_df() {
    // rep stosb from DI=FFFE, 4 bytes: FFFE FFFF 0000 0001.
    let (mut cpu, mut bus) = strings(&[0xF3, 0xAA, 0xF4], 0, 0xFFFE, 4);
    cpu.regs.set_al(0xAA);
    run(&mut cpu, &mut bus);
    for off in [0xFFFE, 0xFFFF, 0, 1] {
        assert_eq!(bus.read8(linear(0x4000, off)), 0xAA, "{off:x}");
    }
    assert_eq!(bus.read8(linear(0x4000, 2)), 0);
    assert_eq!(cpu.regs.di(), 2);

    // std ; rep movsw from SI=DI=2: words at 2 then 0, ending at FFFE.
    let (mut cpu, mut bus) = strings(&[0xFD, 0xF3, 0xA5, 0xF4], 2, 2, 2);
    bus.write16(linear(DATA, 0), 0x1111);
    bus.write16(linear(DATA, 2), 0x2222);
    run(&mut cpu, &mut bus);
    assert_eq!(bus.read16(linear(0x4000, 0)), 0x1111);
    assert_eq!(bus.read16(linear(0x4000, 2)), 0x2222);
    assert_eq!((cpu.regs.si(), cpu.regs.di()), (0xFFFE, 0xFFFE));
    assert_ne!(cpu.flags & DF, 0);
}

#[test]
fn repe_cmps_and_repne_scas_stop_on_zf() {
    let (mut cpu, mut bus) = strings(&[0xF3, 0xA6, 0xF4], 0, 0, 10);
    bus.load(linear(DATA, 0), b"abcX");
    bus.load(linear(0x4000, 0), b"abcY");
    run(&mut cpu, &mut bus);
    assert_eq!((cpu.regs.cx(), cpu.regs.si(), cpu.regs.di()), (6, 4, 4));
    assert_eq!(cpu.flags & ZF, 0);

    let (mut cpu, mut bus) = strings(&[0xF2, 0xAE, 0xF4], 0, 0, 4);
    bus.load(linear(0x4000, 0), b"abcd");
    cpu.regs.set_al(b'c');
    run(&mut cpu, &mut bus);
    assert_eq!((cpu.regs.cx(), cpu.regs.di()), (1, 3));
    assert_ne!(cpu.flags & ZF, 0);
}

#[test]
fn lods_stos_and_source_override() {
    // es: lodsw ; stosb
    let (mut cpu, mut bus) = strings(&[0x26, 0xAD, 0xAA, 0xF4], 0x10, 0x20, 0);
    bus.write16(linear(0x4000, 0x10), 0xBEEF);
    run(&mut cpu, &mut bus);
    assert_eq!(cpu.regs.ax(), 0xBEEF);
    assert_eq!(bus.read8(linear(0x4000, 0x20)), 0xEF);
    assert_eq!((cpu.regs.si(), cpu.regs.di()), (0x12, 0x21));
}

#[test]
fn io_xlat_and_misc() {
    #[rustfmt::skip]
    let (mut cpu, mut bus) = setup(&[
        0xE4, 0x60,       // in al, 60h
        0xBA, 0x20, 0x00, // mov dx, 20h
        0xEE,             // out dx, al
        0xE7, 0x40,       // out 40h, ax
        0xED,             // in ax, dx
        0xF4,
    ]);
    bus.io_in.insert(0x60, 0x1E);
    bus.io_in.insert(0x20, 0x34);
    bus.io_in.insert(0x21, 0x12);
    run(&mut cpu, &mut bus);
    assert_eq!(bus.io_out, [(0x20, 0x1E), (0x40, 0x1E), (0x41, 0x00)]);
    assert_eq!(cpu.regs.ax(), 0x1234);

    // mov bx,100h ; mov al,5 ; xlat
    let (mut cpu, mut bus) = setup(&[0xBB, 0x00, 0x01, 0xB0, 0x05, 0xD7, 0xF4]);
    bus.write8(linear(DATA, 0x105), 0x77);
    run(&mut cpu, &mut bus);
    assert_eq!(cpu.regs.al(), 0x77);

    // mov al,80h ; cbw ; cwd
    let (cpu, _) = exec(&[0xB0, 0x80, 0x98, 0x99, 0xF4]);
    assert_eq!((cpu.regs.ax(), cpu.regs.dx()), (0xFF80, 0xFFFF));
    // mov al,9 ; add al,1 ; daa
    let (cpu, _) = exec(&[0xB0, 0x09, 0x04, 0x01, 0x27, 0xF4]);
    assert_eq!(cpu.regs.al(), 0x10);
    // mov al,63 ; aam ; then aad
    let (cpu, _) = exec(&[0xB0, 0x3F, 0xD4, 0x0A, 0xF4]);
    assert_eq!(cpu.regs.ax(), 0x0603);
    let (cpu, _) = exec(&[0xB8, 0x03, 0x06, 0xD5, 0x0A, 0xF4]);
    assert_eq!(cpu.regs.ax(), 0x003F);
    // mov ax,0009h ; add al,3 ; aaa -> 0102h
    let (cpu, _) = exec(&[0xB8, 0x09, 0x00, 0x04, 0x03, 0x37, 0xF4]);
    assert_eq!(cpu.regs.ax(), 0x0102);
    // stc ; salc
    let (cpu, _) = exec(&[0xF9, 0xD6, 0xF4]);
    assert_eq!(cpu.regs.al(), 0xFF);
    // fld dword [0] ; wait  -> skipped
    let (cpu, _) = exec(&[0xD9, 0x06, 0x00, 0x00, 0x9B, 0xF4]);
    assert_eq!(cpu.ip, 6);
}

#[test]
fn fe_ff_groups_keep_callback_and_fault_on_bad_ext() {
    let (mut cpu, mut bus) = setup(&[0xFE, 0x38, 0x01, 0x02]);
    assert_eq!(
        cpu.step(&mut bus).0,
        Step::Callback(crate::hle::HleId(0x0201))
    );
    let (mut cpu, mut bus) = setup(&[0xFE, 0x10]); // FE /2
    assert!(matches!(cpu.step(&mut bus).0, Step::Fault(_)));
    assert_eq!(cpu.ip, 0);
}

#[test]
fn checksum_loop() {
    #[rustfmt::skip]
    let (mut cpu, mut bus) = setup(&[
        0xBE, 0x00, 0x00, // mov si, 0
        0xB9, 0x00, 0x01, // mov cx, 256
        0x31, 0xDB,       // xor bx, bx
        0xFC,             // cld
        0xAC,             // L: lodsb
        0x32, 0xE4,       // xor ah, ah
        0x01, 0xC3,       // add bx, ax
        0xD1, 0xC3,       // rol bx, 1
        0xE2, 0xF7,       // loop L
        0x89, 0x1E, 0x00, 0x02, // mov [200h], bx
        0xF4,
    ]);
    let data: Vec<u8> = (0..256u32).map(|i| (i * 7 + 3) as u8).collect();
    bus.load(linear(DATA, 0), &data);
    let mut steps = 0;
    while cpu.step(&mut bus).0 == Step::Ok {
        steps += 1;
        assert!(steps < 10_000);
    }
    let want = data
        .iter()
        .fold(0u16, |s, &b| s.wrapping_add(b as u16).rotate_left(1));
    assert_eq!(bus.read16(linear(DATA, 0x200)), want);
    assert_eq!(cpu.regs.cx(), 0);
}
