//! 386 real-mode extensions: 66h/67h, SIB, `0F xx` (025 part 2).

use super::flags::{CF, OF, ZF};
use super::tests::{CODE, DATA, STACK, run, setup};
use super::*;
use crate::bus::{FlatBus, linear};

fn exec(code: &[u8]) -> (Cpu, FlatBus) {
    let (mut cpu, mut bus) = setup(code);
    run(&mut cpu, &mut bus);
    (cpu, bus)
}

fn hook(bus: &mut FlatBus, vector: u8, handler: u16) {
    bus.write16(vector as u32 * 4, handler);
    bus.write16(vector as u32 * 4 + 2, CODE);
}

#[test]
fn operand_size_prefix_alu_stack_shift() {
    #[rustfmt::skip]
    let (cpu, _) = exec(&[
        0x66, 0xB8, 0x78, 0x56, 0x34, 0x12, // mov eax, 12345678h
        0x66, 0xBB, 0x11, 0x11, 0x11, 0x11, // mov ebx, 11111111h
        0x66, 0x01, 0xD8,                   // add eax, ebx
        0x66, 0x50,                         // push eax
        0x66, 0x59,                         // pop ecx
        0x66, 0x41,                         // inc ecx
        0x66, 0xC1, 0xE1, 0x04,             // shl ecx, 4
        0xB8, 0x00, 0x80,                   // mov ax, 8000h
        0x66, 0x98,                         // cwde
        0xF4,
    ]);
    assert_eq!(cpu.regs.get32(regs::CX), 0x3456_78A0);
    assert_eq!(cpu.regs.get32(regs::AX), 0xFFFF_8000);
    assert_eq!(cpu.regs.sp(), 0x100);
}

#[test]
fn pushad_popad() {
    #[rustfmt::skip]
    let (cpu, mut bus) = exec(&[
        0x66, 0xB8, 0x01, 0x00, 0x00, 0x80, // mov eax, 80000001h
        0x66, 0x60,                         // pushad
        0x66, 0x31, 0xC0,                   // xor eax, eax
        0x66, 0x61,                         // popad
        0xF4,
    ]);
    assert_eq!(cpu.regs.get32(regs::AX), 0x8000_0001);
    assert_eq!(cpu.regs.sp(), 0x100);
    // ESP slot (5th push) holds the original ESP.
    assert_eq!(bus.read16(linear(STACK, 0x100 - 20)), 0x100);
}

#[test]
fn mul_div_32_and_divide_error() {
    #[rustfmt::skip]
    let (cpu, _) = exec(&[
        0x66, 0xB8, 0x00, 0x00, 0x00, 0x80, // mov eax, 80000000h
        0x66, 0xBB, 0x04, 0x00, 0x00, 0x00, // mov ebx, 4
        0x66, 0xF7, 0xE3,                   // mul ebx -> EDX:EAX = 2:0
        0x66, 0x89, 0xD6,                   // mov esi, edx
        0x66, 0xF7, 0xF3,                   // div ebx
        0x66, 0x99,                         // cdq
        0xF4,
    ]);
    assert_eq!(cpu.regs.get32(regs::SI), 2);
    assert_eq!(cpu.regs.get32(regs::AX), 0x8000_0000);
    assert_eq!(cpu.regs.get32(regs::DX), 0xFFFF_FFFF);
    assert_eq!(cpu.flags & (CF | OF), CF | OF);

    #[rustfmt::skip]
    let (mut cpu, mut bus) = setup(&[
        0x66, 0x31, 0xDB, // xor ebx, ebx
        0x66, 0xF7, 0xF3, // div ebx -> #DE
        0xF4,
        0xF4,             // 7: handler
    ]);
    hook(&mut bus, 0, 7);
    run(&mut cpu, &mut bus);
    assert_eq!(cpu.ip, 8);
    assert_eq!(bus.read16(linear(STACK, 0xFA)), 3, "returns to the DIV");
}

#[test]
fn address_size_prefix_and_sib() {
    #[rustfmt::skip]
    let (mut cpu, mut bus) = setup(&[
        0x66, 0xBB, 0x10, 0x00, 0x00, 0x00,       // mov ebx, 10h
        0x66, 0xBE, 0x02, 0x00, 0x00, 0x00,       // mov esi, 2
        0x67, 0x66, 0x8B, 0x4C, 0xB3, 0x08,       // mov ecx, [ebx+esi*4+8]
        0x67, 0x66, 0x8B, 0x04, 0x25, 0x30, 0x00, 0x00, 0x00, // mov eax, [30h]
        0x66, 0xBD, 0x50, 0x00, 0x00, 0x00,       // mov ebp, 50h
        0x67, 0x8B, 0x55, 0x00,                   // mov dx, [ebp+0] (SS)
        0x67, 0x8D, 0x7C, 0x76, 0x01,             // lea di, [esi+esi*2+1]
        0xF4,
    ]);
    bus.write32(linear(DATA, 0x20), 0xCAFE_BABE);
    bus.write32(linear(DATA, 0x30), 0x0102_0304);
    bus.write16(linear(STACK, 0x50), 0xBEEF);
    run(&mut cpu, &mut bus);
    assert_eq!(cpu.regs.get32(regs::CX), 0xCAFE_BABE);
    assert_eq!(cpu.regs.get32(regs::AX), 0x0102_0304);
    assert_eq!(cpu.regs.dx(), 0xBEEF);
    assert_eq!(cpu.regs.di(), 7);
}

#[test]
fn movzx_movsx_bsf_bsr_setcc() {
    #[rustfmt::skip]
    let (cpu, _) = exec(&[
        0xB0, 0xF0,             // mov al, F0h
        0x66, 0x0F, 0xBE, 0xD8, // movsx ebx, al
        0x0F, 0xB6, 0xC8,       // movzx cx, al
        0x0F, 0xBC, 0xD1,       // bsf dx, cx
        0x0F, 0xBD, 0xF1,       // bsr si, cx
        0x39, 0xC0,             // cmp ax, ax
        0x0F, 0x94, 0xC7,       // sete bh
        0x0F, 0x95, 0xC4,       // setne ah
        0x31, 0xFF,             // xor di, di
        0x0F, 0xBC, 0xD7,       // bsf dx, di (zero: ZF=1, dx kept)
        0xF4,
    ]);
    assert_eq!(cpu.regs.get32(regs::BX), 0xFFFF_01F0);
    assert_eq!((cpu.regs.cx(), cpu.regs.dx(), cpu.regs.si()), (0xF0, 4, 7));
    assert_eq!(cpu.regs.ax(), 0x00F0);
    assert_ne!(cpu.flags & ZF, 0);
}

#[test]
fn bit_test_family() {
    #[rustfmt::skip]
    let (cpu, mut bus) = exec(&[
        0xB8, 0x05, 0x00,       // mov ax, 5
        0x0F, 0xBA, 0xE8, 0x01, // bts ax, 1  -> 7, CF=0
        0x0F, 0xBA, 0xF0, 0x00, // btr ax, 0  -> 6, CF=1
        0x0F, 0xBA, 0xF8, 0x02, // btc ax, 2  -> 2, CF=1
        0xBB, 0x10, 0x00,       // mov bx, 10h
        0xB9, 0x13, 0x00,       // mov cx, 19
        0x0F, 0xAB, 0x0F,       // bts [bx], cx  -> word [12h] bit 3
        0xB9, 0xFF, 0xFF,       // mov cx, -1
        0x0F, 0xAB, 0x0F,       // bts [bx], cx  -> word [0Eh] bit 15
        0x0F, 0xBA, 0xE0, 0x01, // bt ax, 1 -> CF=1
        0xF4,
    ]);
    assert_eq!(cpu.regs.ax(), 2);
    assert_eq!(bus.read16(linear(DATA, 0x12)), 0x0008);
    assert_eq!(bus.read16(linear(DATA, 0x0E)), 0x8000);
    assert_ne!(cpu.flags & CF, 0);
}

#[test]
fn shld_shrd_and_imul_rrm() {
    #[rustfmt::skip]
    let (cpu, _) = exec(&[
        0xB8, 0x34, 0x12,       // mov ax, 1234h
        0xBA, 0xCD, 0xAB,       // mov dx, ABCDh
        0x0F, 0xA4, 0xD0, 0x04, // shld ax, dx, 4 -> 234Ah
        0xBB, 0xFF, 0x00,       // mov bx, 00FFh
        0x0F, 0xAC, 0xD3, 0x04, // shrd bx, dx, 4 -> D00Fh, CF=1
        0xBE, 0xFD, 0xFF,       // mov si, -3
        0xBF, 0x07, 0x00,       // mov di, 7
        0x0F, 0xAF, 0xF7,       // imul si, di -> -21
        0xF4,
    ]);
    assert_eq!(cpu.regs.ax(), 0x234A);
    assert_eq!(cpu.regs.bx(), 0xD00F);
    assert_eq!(cpu.regs.si(), 0xFFEB);
    assert_eq!(cpu.flags & (CF | OF), 0, "IMUL fits");
}

#[test]
fn near_jcc_and_loop_with_ecx() {
    #[rustfmt::skip]
    let (cpu, _) = exec(&[
        0x31, 0xC0,             // 0: xor ax, ax (ZF=1)
        0x0F, 0x84, 0x03, 0x00, // 2: jz +3 -> 9
        0xB0, 0x01, 0xF4,       // 6: mov al, 1; hlt
        0xB0, 0x02,             // 9: mov al, 2
        0x0F, 0x85, 0xF7, 0xFF, // B: jnz -9 (not taken)
        0x66, 0xB9, 0x01, 0x00, 0x01, 0x00, // F: mov ecx, 10001h
        0x67, 0xE2, 0x01,       // 15: loop +1 (ECX = 10000h: taken)
        0xF4,                   // 18
        0xF4,                   // 19
    ]);
    assert_eq!(cpu.regs.al(), 2);
    assert_eq!(cpu.regs.get32(regs::CX), 0x10000);
    assert_eq!(cpu.ip, 0x1A);
}

#[test]
fn rep_movsd() {
    #[rustfmt::skip]
    let (mut cpu, mut bus) = setup(&[
        0x8C, 0xD8, 0x8E, 0xC0,             // mov ax, ds; mov es, ax
        0xBE, 0x00, 0x00, 0xBF, 0x20, 0x00, // mov si, 0; mov di, 20h
        0xB9, 0x02, 0x00,                   // mov cx, 2
        0xF3, 0x66, 0xA5,                   // rep movsd
        0xF4,
    ]);
    bus.write32(linear(DATA, 0), 0x1122_3344);
    bus.write32(linear(DATA, 4), 0x5566_7788);
    run(&mut cpu, &mut bus);
    assert_eq!(bus.read32(linear(DATA, 0x20)), 0x1122_3344);
    assert_eq!(bus.read32(linear(DATA, 0x24)), 0x5566_7788);
    assert_eq!((cpu.regs.si(), cpu.regs.di(), cpu.regs.cx()), (8, 0x28, 0));
}

#[test]
fn fs_gs_and_far_loads() {
    #[rustfmt::skip]
    let (mut cpu, mut bus) = setup(&[
        0xB8, 0x00, 0x40,             // mov ax, 4000h
        0x8E, 0xE0,                   // mov fs, ax
        0x64, 0xA1, 0x00, 0x00,       // mov ax, fs:[0]
        0x0F, 0xA0,                   // push fs
        0x0F, 0xA9,                   // pop gs
        0x65, 0x8B, 0x1E, 0x02, 0x00, // mov bx, gs:[2]
        0x0F, 0xB4, 0x36, 0x40, 0x00, // lfs si, [40h]
        0xF4,
    ]);
    bus.write16(linear(0x4000, 0), 0x1111);
    bus.write16(linear(0x4000, 2), 0x2222);
    bus.write16(linear(DATA, 0x40), 0x5678);
    bus.write16(linear(DATA, 0x42), 0x9ABC);
    run(&mut cpu, &mut bus);
    assert_eq!((cpu.regs.ax(), cpu.regs.bx()), (0x1111, 0x2222));
    assert_eq!(cpu.sregs.get(Seg::Gs), 0x4000);
    assert_eq!((cpu.regs.si(), cpu.sregs.get(Seg::Fs)), (0x5678, 0x9ABC));
}

#[test]
fn popfd_cannot_set_ac_so_it_is_a_386() {
    #[rustfmt::skip]
    let (cpu, _) = exec(&[
        0x66, 0x9C,                         // pushfd
        0x66, 0x58,                         // pop eax
        0x66, 0x35, 0x00, 0x00, 0x24, 0x00, // xor eax, 240000h (AC | ID)
        0x66, 0x50,                         // push eax
        0x66, 0x9D,                         // popfd
        0x66, 0x9C,                         // pushfd
        0x66, 0x5B,                         // pop ebx
        0xF4,
    ]);
    assert_eq!(cpu.regs.get32(regs::BX) & 0xFFFF_0000, 0);
}

#[test]
fn control_registers() {
    #[rustfmt::skip]
    let (cpu, _) = exec(&[
        0x66, 0xB8, 0xFF, 0xFF, 0xFF, 0xFF, // mov eax, -1
        0x0F, 0x20, 0xC0,                   // mov eax, cr0 -> 0
        0x0F, 0x22, 0xC0,                   // mov cr0, eax (PE=0)
        0xF4,
    ]);
    assert_eq!(cpu.regs.get32(regs::AX), 0);
    // Setting PE is unsupported.
    let (mut cpu, mut bus) = setup(&[0x66, 0xB8, 1, 0, 0, 0, 0x0F, 0x22, 0xC0]);
    cpu.step(&mut bus);
    assert!(matches!(cpu.step(&mut bus).0, Step::Fault(_)));
}

#[test]
fn prefix_66_is_ud_on_186() {
    let (mut cpu, mut bus) = setup(&[0x66, 0x40, 0xF4, 0xF4]);
    cpu.model = CpuModel::I186;
    hook(&mut bus, 6, 3);
    run(&mut cpu, &mut bus);
    assert_eq!(cpu.ip, 4);
    assert_eq!(cpu.regs.ax(), 0);
}
