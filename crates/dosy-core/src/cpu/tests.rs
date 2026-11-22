//! Hand-assembled programs on `FlatBus`.

use super::flags::{CF, DF, IF, OF, PF, RESERVED1, SF, TF, ZF};
use super::regs::*;
use super::*;
use crate::bus::{FlatBus, linear};
use crate::hle::{StubKind, install_stub};

const CODE: u16 = 0x1000;
const STACK: u16 = 0x2000;
const DATA: u16 = 0x3000;

/// CPU at CODE:0000 with `code` loaded, SS:SP = STACK:0100, DS = DATA.
fn setup(code: &[u8]) -> (Cpu, FlatBus) {
    let mut cpu = Cpu::new();
    cpu.far_jump(CODE, 0);
    cpu.sregs.set(Seg::Ss, STACK);
    cpu.sregs.set(Seg::Ds, DATA);
    cpu.regs.set_sp(0x100);
    let mut bus = FlatBus::new();
    bus.load(linear(CODE, 0), code);
    (cpu, bus)
}

/// Step until HLT; panics on any other non-Ok step or after 1000 steps.
fn run(cpu: &mut Cpu, bus: &mut FlatBus) {
    for _ in 0..1000 {
        match cpu.step(bus) {
            (Step::Ok, 1) => {}
            (Step::Halt, _) => return,
            other => panic!("unexpected {other:?} at {:04x}", cpu.ip),
        }
    }
    panic!("no HLT");
}

#[test]
fn reset_state() {
    let cpu = Cpu::new();
    assert_eq!((cpu.sregs.get(Seg::Cs), cpu.ip), (0xF000, 0xFFF0));
    assert_eq!(cpu.flags, RESERVED1);
}

#[test]
fn mov_forms() {
    #[rustfmt::skip]
    let (mut cpu, mut bus) = setup(&[
        0xB8, 0x34, 0x12,             // mov ax, 1234h
        0xBB, 0x10, 0x00,             // mov bx, 0010h
        0x89, 0x07,                   // mov [bx], ax
        0x8A, 0x4F, 0x01,             // mov cl, [bx+1]
        0xB4, 0x77,                   // mov ah, 77h
        0xA3, 0x20, 0x00,             // mov [0020h], ax
        0xA0, 0x20, 0x00,             // mov al, [0020h]
        0xC6, 0x47, 0x05, 0xAB,       // mov byte [bx+5], 0ABh
        0xC7, 0x87, 0x00, 0x01, 0xEF, 0xBE, // mov word [bx+100h], 0BEEFh
        0x8B, 0x97, 0x00, 0x01,       // mov dx, [bx+100h]
        0x8C, 0xDE,                   // mov si, ds
        0x8E, 0xC6,                   // mov es, si
        0xF4,
    ]);
    run(&mut cpu, &mut bus);
    assert_eq!(bus.read16(linear(DATA, 0x10)), 0x1234);
    assert_eq!(cpu.regs.cl(), 0x12);
    assert_eq!(bus.read16(linear(DATA, 0x20)), 0x7734);
    assert_eq!(cpu.regs.ax(), 0x7734);
    assert_eq!(bus.read8(linear(DATA, 0x15)), 0xAB);
    assert_eq!(cpu.regs.dx(), 0xBEEF);
    assert_eq!(cpu.sregs.get(Seg::Es), DATA);
}

#[test]
fn xchg_lea_lds_les() {
    #[rustfmt::skip]
    let (mut cpu, mut bus) = setup(&[
        0xB8, 0x01, 0x00,       // mov ax, 1
        0xB9, 0x02, 0x00,       // mov cx, 2
        0x91,                   // xchg ax, cx
        0x87, 0xD9,             // xchg bx, cx  (mod=3 reg=bx rm=cx)
        0xBD, 0x00, 0x10,       // mov bp, 1000h
        0xBE, 0x20, 0x00,       // mov si, 20h
        0x8D, 0x7A, 0xFF,       // lea di, [bp+si-1]
        0xC5, 0x36, 0x40, 0x00, // lds si, [0040h]
        0xF4,
    ]);
    bus.write16(linear(DATA, 0x40), 0x5678);
    bus.write16(linear(DATA, 0x42), 0x4000);
    run(&mut cpu, &mut bus);
    assert_eq!((cpu.regs.ax(), cpu.regs.cx(), cpu.regs.bx()), (2, 0, 1));
    assert_eq!(cpu.regs.di(), 0x101F);
    assert_eq!((cpu.sregs.get(Seg::Ds), cpu.regs.si()), (0x4000, 0x5678));

    let (mut cpu, mut bus) = setup(&[0xC4, 0x1E, 0x40, 0x00, 0xF4]); // les bx, [40h]
    bus.write16(linear(DATA, 0x40), 0x1111);
    bus.write16(linear(DATA, 0x42), 0x2222);
    run(&mut cpu, &mut bus);
    assert_eq!((cpu.sregs.get(Seg::Es), cpu.regs.bx()), (0x2222, 0x1111));
}

#[test]
fn segment_override_and_default_segments() {
    #[rustfmt::skip]
    let (mut cpu, mut bus) = setup(&[
        0xBB, 0x10, 0x00, // mov bx, 10h
        0x8B, 0x07,       // mov ax, [bx]       ; DS
        0x26, 0x8B, 0x0F, // mov cx, es:[bx]
        0xBD, 0x10, 0x00, // mov bp, 10h
        0x8B, 0x56, 0x00, // mov dx, [bp+0]     ; SS
        0x3E, 0x8B, 0x76, 0x00, // mov si, ds:[bp+0]
        0x2E, 0xA1, 0x00, 0x00, // mov ax, cs:[0000h]
        0xF4,
    ]);
    cpu.sregs.set(Seg::Es, 0x5000);
    bus.write16(linear(DATA, 0x10), 0xDDDD);
    bus.write16(linear(0x5000, 0x10), 0xEEEE);
    bus.write16(linear(STACK, 0x10), 0x5555);
    // Check AX after the DS load, before it is overwritten.
    for _ in 0..2 {
        cpu.step(&mut bus);
    }
    assert_eq!(cpu.regs.ax(), 0xDDDD);
    run(&mut cpu, &mut bus);
    assert_eq!(cpu.regs.cx(), 0xEEEE);
    assert_eq!(cpu.regs.dx(), 0x5555);
    assert_eq!(cpu.regs.si(), 0xDDDD);
    assert_eq!(cpu.regs.ax(), 0x10BB); // first code bytes
}

#[test]
fn near_and_far_jump_call_ret() {
    #[rustfmt::skip]
    let (mut cpu, mut bus) = setup(&[
        0xE8, 0x0E, 0x00,             // 00: call 0011h
        0x9A, 0x00, 0x00, 0x00, 0x40, // 03: call far 4000:0000
        0xEA, 0x00, 0x01, 0x00, 0x40, // 08: jmp far 4000:0100
        0xF4, 0xF4, 0xF4,             // 0D..0F
        0xF4,                         // 10
        0xBB, 0x01, 0x00,             // 11: mov bx, 1
        0xC3,                         // 15: ret
    ]);
    // 4000:0000: mov cx, 2 ; retf
    bus.load(linear(0x4000, 0), &[0xB9, 0x02, 0x00, 0xCB]);
    // 4000:0100: mov dx, 3 ; hlt
    bus.load(linear(0x4000, 0x100), &[0xBA, 0x03, 0x00, 0xF4]);
    run(&mut cpu, &mut bus);
    assert_eq!((cpu.regs.bx(), cpu.regs.cx(), cpu.regs.dx()), (1, 2, 3));
    assert_eq!((cpu.sregs.get(Seg::Cs), cpu.ip), (0x4000, 0x104));
    assert_eq!(cpu.regs.sp(), 0x100);
}

#[test]
fn ret_imm_and_short_jumps() {
    #[rustfmt::skip]
    let (mut cpu, mut bus) = setup(&[
        0x50,             // 00: push ax  (argument)
        0xE8, 0x03, 0x00, // 01: call 0007h
        0xEB, 0x05,       // 04: jmp 000Bh
        0xF4,             // 06
        0xC2, 0x02, 0x00, // 07: ret 2
        0xF4,             // 0A
        0x50,             // 0B: push ax  (argument)
        0x0E,             // 0C: push cs
        0xE8, 0x03, 0x00, // 0D: call 0013h  (near call to a far routine)
        0xF4,             // 10
        0xF4, 0xF4,       // 11..12
        0xCA, 0x02, 0x00, // 13: retf 2
    ]);
    run(&mut cpu, &mut bus);
    assert_eq!((cpu.sregs.get(Seg::Cs), cpu.ip), (CODE, 0x11));
    assert_eq!(cpu.regs.sp(), 0x100);
}

#[test]
fn indirect_jump_and_call() {
    #[rustfmt::skip]
    let (mut cpu, mut bus) = setup(&[
        0xBB, 0x0E, 0x00, // 00: mov bx, 000Eh
        0xFF, 0xD3,       // 03: call bx
        0xFF, 0x1E, 0x00, 0x00, // 05: call far [0000h] -> 4000:0000
        0xFF, 0x26, 0x04, 0x00, // 09: jmp [0004h] -> 0012h
        0xF4,             // 0D
        0xC3,             // 0E: ret
        0xF4, 0xF4, 0xF4, // 0F..11
        0xFF, 0x2E, 0x06, 0x00, // 12: jmp far [0006h] -> 4000:0010
    ]);
    bus.write16(linear(DATA, 0), 0x0000);
    bus.write16(linear(DATA, 2), 0x4000);
    bus.write16(linear(DATA, 4), 0x0012);
    bus.write16(linear(DATA, 6), 0x0010);
    bus.write16(linear(DATA, 8), 0x4000);
    bus.load(linear(0x4000, 0), &[0xCB]); // retf
    bus.load(linear(0x4000, 0x10), &[0xF4]);
    run(&mut cpu, &mut bus);
    assert_eq!((cpu.sregs.get(Seg::Cs), cpu.ip), (0x4000, 0x11));
    assert_eq!(cpu.regs.sp(), 0x100);
}

#[test]
fn stack_wraps_at_sp_zero() {
    #[rustfmt::skip]
    let (mut cpu, mut bus) = setup(&[
        0x50,       // push ax
        0x5B,       // pop bx
        0x5A,       // pop dx  (reads SS:0000)
        0xF4,
    ]);
    cpu.regs.set_sp(0);
    cpu.regs.set_ax(0xA55A);
    bus.write16(linear(STACK, 0), 0x1234);
    cpu.step(&mut bus);
    assert_eq!(cpu.regs.sp(), 0xFFFE);
    assert_eq!(bus.read16(linear(STACK, 0xFFFE)), 0xA55A);
    run(&mut cpu, &mut bus);
    assert_eq!(
        (cpu.regs.bx(), cpu.regs.dx(), cpu.regs.sp()),
        (0xA55A, 0x1234, 2)
    );

    // Odd SP: word straddles SS:FFFF and SS:0000.
    let (mut cpu, mut bus) = setup(&[0x50, 0xF4]);
    cpu.regs.set_sp(1);
    cpu.regs.set_ax(0xBBAA);
    run(&mut cpu, &mut bus);
    assert_eq!(cpu.regs.sp(), 0xFFFF);
    assert_eq!(bus.read8(linear(STACK, 0xFFFF)), 0xAA);
    assert_eq!(bus.read8(linear(STACK, 0)), 0xBB);
}

#[test]
fn push_pop_sreg_rm_and_sp() {
    #[rustfmt::skip]
    let (mut cpu, mut bus) = setup(&[
        0x1E,             // push ds
        0x07,             // pop es
        0x54,             // push sp  (pushes 0100h: pre-decrement value, 286+)
        0x8F, 0x06, 0x00, 0x00, // pop word [0000h]
        0xFF, 0x36, 0x00, 0x00, // push word [0000h]
        0x17,             // pop ss  -> SS = 0100h
        0xF4,
    ]);
    run(&mut cpu, &mut bus);
    assert_eq!(cpu.sregs.get(Seg::Es), DATA);
    assert_eq!(bus.read16(linear(DATA, 0)), 0x100);
    assert_eq!(cpu.sregs.get(Seg::Ss), 0x100);
    assert_eq!(cpu.regs.sp(), 0x100);
}

#[test]
fn flag_instructions() {
    #[rustfmt::skip]
    let (mut cpu, mut bus) = setup(&[
        0xF9, 0xFD, 0xFB, // stc std sti
        0x9C,             // pushf
        0xF5, 0xFC, 0xFA, // cmc cld cli
        0x9F,             // lahf
        0x5B,             // pop bx (pushed flags)
        0xB4, 0xD5,       // mov ah, D5h
        0x9E,             // sahf
        0xB8, 0xFF, 0xFF, // mov ax, FFFFh
        0x50, 0x9D,       // push ax ; popf
        0xF4,
    ]);
    for _ in 0..3 {
        cpu.step(&mut bus);
    }
    assert_eq!(cpu.flags, RESERVED1 | CF | DF | IF);
    assert!(cpu.int_shadow, "STI with IF=0 inhibits IRQs for one insn");
    cpu.step(&mut bus);
    assert!(!cpu.int_shadow);
    for _ in 0..4 {
        cpu.step(&mut bus);
    }
    assert_eq!(cpu.flags, RESERVED1);
    assert_eq!(cpu.regs.ah(), 0x02);
    for _ in 0..3 {
        cpu.step(&mut bus);
    }
    assert_eq!(cpu.regs.bx() as u32, RESERVED1 | CF | DF | IF);
    assert_eq!(cpu.flags, RESERVED1 | SF | ZF | flags::AF | PF | CF);
    run(&mut cpu, &mut bus);
    // POPF of FFFF: bits 12..15, 5, 3 stay clear (386 real mode).
    assert_eq!(cpu.flags, 0x0FD7);
}

#[test]
fn mov_ss_sets_interrupt_shadow() {
    let (mut cpu, mut bus) = setup(&[0x8E, 0xD0, 0x90, 0xF4]); // mov ss, ax ; nop
    cpu.step(&mut bus);
    assert!(cpu.int_shadow);
    cpu.step(&mut bus);
    assert!(!cpu.int_shadow);
}

#[test]
fn int_iret_round_trip() {
    #[rustfmt::skip]
    let (mut cpu, mut bus) = setup(&[
        0xFB,             // sti
        0xF9,             // stc
        0xCD, 0x21,       // int 21h
        0xF4,
    ]);
    // Handler at 5000:0010: pushf; pop dx; mov bx, 1234h; iret
    bus.load(linear(0x5000, 0x10), &[0x9C, 0x5A, 0xBB, 0x34, 0x12, 0xCF]);
    bus.write16(0x21 * 4, 0x0010);
    bus.write16(0x21 * 4 + 2, 0x5000);
    for _ in 0..3 {
        cpu.step(&mut bus);
    }
    assert_eq!((cpu.sregs.get(Seg::Cs), cpu.ip), (0x5000, 0x10));
    assert_eq!(cpu.regs.sp(), 0x100 - 6);
    assert_eq!(bus.read16(linear(STACK, 0xFA)), 4); // return IP
    assert_eq!(bus.read16(linear(STACK, 0xFC)), CODE);
    assert_eq!(cpu.flags & (IF | TF), 0);
    run(&mut cpu, &mut bus);
    assert_eq!(cpu.regs.dx() as u32, RESERVED1 | CF); // flags inside handler
    assert_eq!(cpu.regs.bx(), 0x1234);
    assert_eq!(cpu.flags, RESERVED1 | CF | IF);
    assert_eq!((cpu.sregs.get(Seg::Cs), cpu.ip), (CODE, 5));
    assert_eq!(cpu.regs.sp(), 0x100);
}

#[test]
fn int3_and_into() {
    let (mut cpu, mut bus) = setup(&[0xCE, 0xCC]);
    bus.write16(3 * 4, 0x0000);
    bus.write16(3 * 4 + 2, 0x6000);
    bus.write16(4 * 4, 0x0000);
    bus.write16(4 * 4 + 2, 0x7000);
    cpu.step(&mut bus); // INTO with OF=0: no-op
    assert_eq!(cpu.ip, 1);
    cpu.step(&mut bus);
    assert_eq!(cpu.sregs.get(Seg::Cs), 0x6000);

    let (mut cpu, mut bus) = setup(&[0xCE]);
    bus.write16(4 * 4 + 2, 0x7000);
    cpu.flags |= OF;
    cpu.step(&mut bus);
    assert_eq!(cpu.sregs.get(Seg::Cs), 0x7000);
}

#[test]
fn hlt_and_interrupt_wakes() {
    let (mut cpu, mut bus) = setup(&[0xF4]);
    assert_eq!(cpu.step(&mut bus), (Step::Halt, 1));
    assert!(cpu.halted);
    assert_eq!(cpu.step(&mut bus).0, Step::Halt);
    assert_eq!(cpu.ip, 1);
    cpu.interrupt(&mut bus, 8);
    assert!(!cpu.halted);
}

#[test]
fn callback_opcode() {
    let (mut cpu, mut bus) = setup(&[0xFE, 0x38, 0x34, 0x12, 0xF4]);
    assert_eq!(cpu.step(&mut bus), (Step::Callback(HleId(0x1234)), 1));
    assert_eq!(cpu.ip, 4);

    // Through a ROM stub: INT 16h -> STI; callback; IRET.
    let (mut cpu, mut bus) = setup(&[0xCD, 0x16, 0xF4]);
    let id = HleId(0x0016);
    install_stub(&mut bus, id, StubKind::IretSti);
    bus.write16(0x16 * 4, id.stub_offset());
    bus.write16(0x16 * 4 + 2, crate::hle::ROM_SEGMENT);
    cpu.step(&mut bus); // int
    cpu.step(&mut bus); // sti
    assert_eq!(cpu.step(&mut bus).0, Step::Callback(id));
    run(&mut cpu, &mut bus);
    assert_eq!((cpu.sregs.get(Seg::Cs), cpu.ip), (CODE, 3));
}

#[test]
fn jcc_table() {
    // (cc, flags that make it taken, flags that make it not taken)
    let cases: [(u8, u32, u32); 16] = [
        (0x0, OF, 0),
        (0x1, 0, OF),
        (0x2, CF, 0),
        (0x3, 0, CF),
        (0x4, ZF, 0),
        (0x5, 0, ZF),
        (0x6, ZF, 0),
        (0x7, 0, CF),
        (0x8, SF, 0),
        (0x9, 0, SF),
        (0xA, PF, 0),
        (0xB, 0, PF),
        (0xC, SF, SF | OF),
        (0xD, SF | OF, OF),
        (0xE, ZF | SF | OF, SF | OF),
        (0xF, SF | OF, ZF),
    ];
    for (cc, yes, no) in cases {
        for (f, taken) in [(yes, true), (no, false)] {
            let (mut cpu, mut bus) = setup(&[0x70 | cc, 0x10]);
            cpu.flags = RESERVED1 | f;
            cpu.step(&mut bus);
            assert_eq!(cpu.ip, if taken { 0x12 } else { 2 }, "cc={cc:x} f={f:x}");
        }
    }
    // Backward jump: jz -2 loops on itself.
    let (mut cpu, mut bus) = setup(&[0x74, 0xFE]);
    cpu.flags |= ZF;
    cpu.step(&mut bus);
    assert_eq!(cpu.ip, 0);
}

#[test]
fn loop_family() {
    #[rustfmt::skip]
    let (mut cpu, mut bus) = setup(&[
        0xB9, 0x05, 0x00, // mov cx, 5
        0x8D, 0x5F, 0x01, // L: lea bx, [bx+1]
        0xE2, 0xFB,       // loop L
        0xE3, 0x01,       // jcxz +1 (taken)
        0xF4,
        0xB9, 0x03, 0x00, // mov cx, 3
        0xE1, 0xFE,       // loope $ (ZF=0: falls through, CX=2)
        0xE0, 0xFE,       // loopne $ (ZF=0: spins to CX=0)
        0xF4,
    ]);
    run(&mut cpu, &mut bus);
    assert_eq!(cpu.regs.bx(), 5);
    assert_eq!(cpu.regs.cx(), 0);
    assert_eq!(cpu.ip, 0x13);

    // LOOP with CX=0 wraps to FFFF and jumps.
    let (mut cpu, mut bus) = setup(&[0xE2, 0x10]);
    cpu.step(&mut bus);
    assert_eq!((cpu.regs.cx(), cpu.ip), (0xFFFF, 0x12));
}

#[test]
fn ip_wraps_within_segment() {
    let (mut cpu, mut bus) = setup(&[]);
    // mov ax, 1234h split across CODE:FFFE..CODE:0000; hlt at 0001.
    bus.load(linear(CODE, 0xFFFE), &[0xB8, 0x34]);
    bus.load(linear(CODE, 0), &[0x12, 0xF4]);
    cpu.ip = 0xFFFE;
    run(&mut cpu, &mut bus);
    assert_eq!((cpu.regs.ax(), cpu.ip), (0x1234, 2));

    // Short jump backwards from 0000 lands at FFF0.
    let (mut cpu, mut bus) = setup(&[0xEB, 0xEE]);
    cpu.step(&mut bus);
    assert_eq!(cpu.ip, 0xFFF0);
}

#[test]
fn invalid_opcode_faults_and_rewinds() {
    let (mut cpu, mut bus) = setup(&[0x90, 0x26, 0xFF, 0xFF]); // nop; es: FF /7
    cpu.step(&mut bus);
    let (step, _) = cpu.step(&mut bus);
    assert_eq!(
        step,
        Step::Fault(CpuFault::InvalidOpcode {
            cs: CODE,
            ip: 1,
            opcode: 0xFF
        })
    );
    assert_eq!(cpu.ip, 1);

    // Endless prefixes fault instead of looping.
    let (mut cpu, mut bus) = setup(&[0x26; 32]);
    assert!(matches!(cpu.step(&mut bus).0, Step::Fault(_)));
    assert_eq!(cpu.ip, 0);
}

#[test]
fn rep_and_lock_are_recorded_and_ignored() {
    let (mut cpu, mut bus) = setup(&[0xF0, 0xF3, 0x90, 0xF2, 0xF4]);
    run(&mut cpu, &mut bus);
    assert_eq!(cpu.ip, 5);
}
