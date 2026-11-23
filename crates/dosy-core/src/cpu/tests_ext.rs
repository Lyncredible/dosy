//! 80186 / 286-real-mode opcodes and CPU-model detection idioms (025).

use super::flags::{CF, OF, ZF};
use super::tests::{CODE, DATA, STACK, run, setup};
use super::*;
use crate::bus::{FlatBus, linear};

fn exec(code: &[u8]) -> (Cpu, FlatBus) {
    exec_model(CpuModel::I386, code)
}

fn exec_model(model: CpuModel, code: &[u8]) -> (Cpu, FlatBus) {
    let (mut cpu, mut bus) = setup(code);
    cpu.model = model;
    run(&mut cpu, &mut bus);
    (cpu, bus)
}

/// IVT entry `vector` -> CODE:`handler`.
fn hook(bus: &mut FlatBus, vector: u8, handler: u16) {
    bus.write16(vector as u32 * 4, handler);
    bus.write16(vector as u32 * 4 + 2, CODE);
}

#[test]
fn pusha_popa() {
    #[rustfmt::skip]
    let (cpu, mut bus) = exec(&[
        0xB8, 0x01, 0x00, 0xB9, 0x02, 0x00, 0xBA, 0x03, 0x00, 0xBB, 0x04, 0x00,
        0xBD, 0x06, 0x00, 0xBE, 0x07, 0x00, 0xBF, 0x08, 0x00, // ax..di (no sp)
        0x60,             // pusha
        0x31, 0xC0, 0x31, 0xDB, 0x31, 0xFF, // xor ax, bx, di
        0x61,             // popa
        0xF4,
    ]);
    assert_eq!(cpu.regs.sp(), 0x100);
    assert_eq!((cpu.regs.ax(), cpu.regs.bx(), cpu.regs.di()), (1, 4, 8));
    // Pushed order AX CX DX BX SP BP SI DI: SP slot holds the original SP.
    assert_eq!(bus.read16(linear(STACK, 0x100 - 10)), 0x100);
    assert_eq!(bus.read16(linear(STACK, 0x100 - 16)), 8);
}

#[test]
fn push_imm_and_imul_imm() {
    #[rustfmt::skip]
    let (cpu, _) = exec(&[
        0x68, 0x34, 0x12, // push 1234h
        0x6A, 0xFE,       // push -2
        0x5B,             // pop bx (FFFE)
        0x59,             // pop cx (1234)
        0x6B, 0xD1, 0x03, // imul dx, cx, 3
        0x69, 0xC3, 0x00, 0x80, // imul ax, bx, 8000h  (-2 * -32768 = 65536: overflow)
        0xF4,
    ]);
    assert_eq!((cpu.regs.bx(), cpu.regs.cx()), (0xFFFE, 0x1234));
    assert_eq!(cpu.regs.dx(), 0x369C);
    assert_eq!(cpu.regs.ax(), 0);
    assert_eq!(cpu.flags & (CF | OF), CF | OF);
}

#[test]
fn shift_by_immediate() {
    #[rustfmt::skip]
    let (cpu, _) = exec(&[
        0xB8, 0x01, 0x00, // mov ax, 1
        0xC1, 0xE0, 0x04, // shl ax, 4
        0xB3, 0x81,       // mov bl, 81h
        0xC0, 0xCB, 0x01, // ror bl, 1
        0xC1, 0xF8, 0x21, // sar ax, 33 (masked to 1)
        0xF4,
    ]);
    assert_eq!(cpu.regs.ax(), 0x0008);
    assert_eq!(cpu.regs.bl(), 0xC0);
}

#[test]
fn enter_leave() {
    #[rustfmt::skip]
    let (cpu, mut bus) = exec(&[
        0xBD, 0xF0, 0x00, // mov bp, 00F0h (caller frame)
        0xC8, 0x08, 0x00, 0x00, // enter 8, 0
        0x89, 0xEA,       // mov dx, bp
        0x89, 0xE1,       // mov cx, sp
        0xC9,             // leave
        0xC8, 0x04, 0x00, 0x02, // enter 4, 2
        0x89, 0xEB,       // mov bx, bp
        0xF4,
    ]);
    assert_eq!((cpu.regs.dx(), cpu.regs.cx()), (0xFE, 0xF6));
    // After LEAVE: SP=100, BP=F0. ENTER 4,2: push BP (SP=FE), frame=FE,
    // push [SS:BP-2] (SP=FC), push frame (SP=FA), BP=FE, SP=F6.
    assert_eq!(cpu.regs.bx(), 0xFE);
    assert_eq!(cpu.regs.sp(), 0xF6);
    assert_eq!(bus.read16(linear(STACK, 0xFA)), 0xFE);
}

#[test]
fn bound_raises_int5_at_the_instruction() {
    #[rustfmt::skip]
    let (mut cpu, mut bus) = setup(&[
        0xB8, 0x05, 0x00,       // 0: mov ax, 5
        0x62, 0x06, 0x00, 0x00, // 3: bound ax, [0000]  (in range)
        0xB8, 0x0B, 0x00,       // 7: mov ax, 11
        0x62, 0x06, 0x00, 0x00, // A: bound ax, [0000]  (out of range)
        0xF4,                   // E
        0xF4,                   // F: handler
    ]);
    bus.write16(linear(DATA, 0), 0); // lower bound
    bus.write16(linear(DATA, 2), 10); // upper bound
    hook(&mut bus, 5, 0x0F);
    run(&mut cpu, &mut bus);
    assert_eq!(cpu.ip, 0x10);
    // Return address on the stack is the BOUND instruction.
    assert_eq!(bus.read16(linear(STACK, 0xFA)), 0x0A);
}

#[test]
fn ins_outs_with_rep() {
    struct IoBus {
        inner: FlatBus,
        out: Vec<(u16, u8)>,
    }
    impl Bus for IoBus {
        fn read8(&mut self, a: u32) -> u8 {
            self.inner.read8(a)
        }
        fn write8(&mut self, a: u32, v: u8) {
            self.inner.write8(a, v)
        }
        fn io_in8(&mut self, port: u16) -> u8 {
            port as u8 ^ 0x5A
        }
        fn io_out8(&mut self, port: u16, v: u8) {
            self.out.push((port, v));
        }
    }
    #[rustfmt::skip]
    let (mut cpu, inner) = setup(&[
        0x8C, 0xD8,       // mov ax, ds
        0x8E, 0xC0,       // mov es, ax
        0xBA, 0x42, 0x00, // mov dx, 42h
        0xBF, 0x10, 0x00, // mov di, 10h
        0xB9, 0x03, 0x00, // mov cx, 3
        0xF3, 0x6C,       // rep insb
        0xBE, 0x10, 0x00, // mov si, 10h
        0xB9, 0x02, 0x00, // mov cx, 2
        0xF3, 0x6E,       // rep outsb
        0xF4,
    ]);
    let mut bus = IoBus { inner, out: vec![] };
    for _ in 0..100 {
        if let (Step::Halt, _) = cpu.step(&mut bus) {
            break;
        }
    }
    assert_eq!(cpu.regs.di(), 0x13);
    assert_eq!(bus.inner.read8(linear(DATA, 0x12)), 0x42 ^ 0x5A);
    assert_eq!(bus.out, [(0x42, 0x18), (0x42, 0x18)]);
    assert_eq!(cpu.regs.si(), 0x12);
}

#[test]
fn smsw_sidt_and_lmsw() {
    #[rustfmt::skip]
    let (cpu, mut bus) = exec(&[
        0xB8, 0xFF, 0xFF,       // mov ax, FFFFh
        0x0F, 0x01, 0xE0,       // smsw ax
        0x0F, 0x01, 0x0E, 0x20, 0x00, // sidt [0020h]
        0x0F, 0x01, 0xF0,       // lmsw ax (PE=0: ignored)
        0xF4,
    ]);
    assert_eq!(cpu.regs.ax(), 0);
    assert_eq!(bus.read16(linear(DATA, 0x20)), 0x03FF);
    assert_eq!(bus.read16(linear(DATA, 0x22)), 0);
    // LMSW with PE=1 is unsupported and faults.
    let (mut cpu, mut bus) = setup(&[0xB8, 0x01, 0x00, 0x0F, 0x01, 0xF0]);
    cpu.step(&mut bus);
    assert!(matches!(cpu.step(&mut bus).0, Step::Fault(_)));
}

#[test]
fn f1_is_int1_on_386_and_lock_before() {
    // 386: F1 -> INT 1.
    let (mut cpu, mut bus) = setup(&[0xF1, 0xF4, 0xF4]);
    hook(&mut bus, 1, 2);
    run(&mut cpu, &mut bus);
    assert_eq!(cpu.ip, 3);
    // 186: F1 is a prefix; F1 40 = INC AX.
    let (cpu, _) = exec_model(CpuModel::I186, &[0xF1, 0x40, 0xF4]);
    assert_eq!(cpu.regs.ax(), 1);
}

// --- CPU detection idioms ---

/// `pushf; pop ax; and ax,0FFFh; push ax; popf; pushf; pop cx` then
/// `or ax,0F000h; push ax; popf; pushf; pop bx`.
#[rustfmt::skip]
const FLAGS_PROBE: &[u8] = &[
    0x9C, 0x58, 0x25, 0xFF, 0x0F, 0x50, 0x9D, 0x9C, 0x59,
    0x0D, 0x00, 0xF0, 0x50, 0x9D, 0x9C, 0x5B,
    0xF4,
];

#[test]
fn flags_high_bits_identify_the_model() {
    let (cpu, _) = exec_model(CpuModel::I8086, FLAGS_PROBE);
    assert_eq!(
        cpu.regs.cx() & 0xF000,
        0xF000,
        "8086: bits 12..15 stuck at 1"
    );
    let (cpu, _) = exec_model(CpuModel::I186, FLAGS_PROBE);
    assert_eq!(cpu.regs.cx() & 0xF000, 0xF000);
    let (cpu, _) = exec_model(CpuModel::I386, FLAGS_PROBE);
    assert_eq!(cpu.regs.cx() & 0xF000, 0, "386: clearable");
    assert_eq!(
        cpu.regs.bx() & 0xF000,
        0x7000,
        "386: IOPL/NT set, bit 15 reads 0"
    );
}

#[test]
fn push_sp_identifies_8086() {
    // push sp; pop ax; cmp ax, sp
    let code = [0x54, 0x58, 0x39, 0xE0, 0xF4];
    let (cpu, _) = exec_model(CpuModel::I8086, &code);
    assert_eq!(cpu.flags & ZF, 0, "8086 pushes SP-2");
    let (cpu, _) = exec_model(CpuModel::I186, &code);
    assert_ne!(cpu.flags & ZF, 0);
}

#[test]
fn shift_count_masking_identifies_8086() {
    // mov ax, 1; mov cl, 33; shl ax, cl
    let code = [0xB8, 0x01, 0x00, 0xB1, 0x21, 0xD3, 0xE0, 0xF4];
    let (cpu, _) = exec_model(CpuModel::I8086, &code);
    assert_eq!(cpu.regs.ax(), 0);
    let (cpu, _) = exec_model(CpuModel::I186, &code);
    assert_eq!(cpu.regs.ax(), 2);
}

#[test]
fn opcode_0f_per_model() {
    // 8086: 0F = POP CS.
    let (mut cpu, mut bus) = setup(&[0x0F]);
    cpu.model = CpuModel::I8086;
    cpu.push16(&mut bus, 0x1234);
    assert_eq!(cpu.step(&mut bus).0, Step::Ok);
    assert_eq!(cpu.sregs.get(Seg::Cs), 0x1234);
    // 186: #UD -> INT 6 with the return address at the opcode.
    let (mut cpu, mut bus) = setup(&[0x90, 0x0F, 0x00, 0xF4]);
    cpu.model = CpuModel::I186;
    hook(&mut bus, 6, 3);
    run(&mut cpu, &mut bus);
    assert_eq!(cpu.ip, 4);
    assert_eq!(bus.read16(linear(STACK, 0xFA)), 1);
}

#[test]
fn reset_keeps_the_model() {
    let mut cpu = Cpu::with_model(CpuModel::I8086);
    cpu.reset();
    assert_eq!(cpu.model, CpuModel::I8086);
    assert_eq!(Cpu::new().model, CpuModel::I386);
}
