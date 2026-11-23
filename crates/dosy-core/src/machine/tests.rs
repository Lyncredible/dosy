use super::*;
use crate::bus::{Bus, linear};
use crate::cpu::Seg;
use crate::hw::DeviceId;
use std::sync::{Arc, Mutex};

const CODE: u16 = 0x1000;
const HANDLER: u16 = 0x2000;
/// One PIT tick at the BIOS default divisor (65536 / 1.193182 MHz).
const TICK_NS: u64 = 54_925_494;

/// Machine with `code` at CODE:0000, SS:SP = 3000:0100, IF=0.
fn machine(code: &[u8]) -> Machine {
    let mut m = Machine::new(MachineConfig::default());
    m.hw.load_rom(linear(CODE, 0), code);
    m.cpu.far_jump(CODE, 0);
    m.cpu.sregs.set(Seg::Ss, 0x3000);
    m.cpu.regs.set_sp(0x100);
    m
}

/// IRQ0 handler: `inc word cs:[0100h]; mov cs:[0102h], ax; EOI; iret`.
fn install_irq0_counter(m: &mut Machine) {
    #[rustfmt::skip]
    let h = [
        0x2E, 0xFF, 0x06, 0x00, 0x01, // inc word cs:[0100h]
        0x2E, 0xA3, 0x02, 0x01,       // mov cs:[0102h], ax
        0xB0, 0x20, 0xE6, 0x20,       // mov al, 20h ; out 20h, al
        0xCF,                         // iret
    ];
    m.hw.load_rom(linear(HANDLER, 0), &h);
    m.hw.write16(8 * 4, 0);
    m.hw.write16(8 * 4 + 2, HANDLER);
}

fn irq0_count(m: &mut Machine) -> u16 {
    m.hw.read16(linear(HANDLER, 0x100))
}

/// `sti ; L: hlt ; jmp L`
const IDLE: [u8; 4] = [0xFB, 0xF4, 0xEB, 0xFD];

#[test]
fn callback_dispatch_by_subsystem() {
    let mut m = machine(&[0xFE, 0x38, 0x05, 0x01, 0xF4]); // DOS id 0105h
    let seen = Arc::new(Mutex::new(None));
    let s = seen.clone();
    m.set_handler(
        Subsystem::Dos,
        Box::new(move |id, cpu, _hw| {
            *s.lock().unwrap() = Some(id);
            cpu.regs.set_ax(0x4242);
            HleResult::Done
        }),
    );
    assert_eq!(m.run_for(100), RunExit::Halted);
    assert_eq!(*seen.lock().unwrap(), Some(HleId(0x0105)));
    assert_eq!(m.cpu.regs.ax(), 0x4242);

    let mut m = machine(&[0xFE, 0x38, 0x01, 0x02]); // shell: no handler yet
    assert_eq!(m.run_for(100), RunExit::UnhandledCallback(HleId(0x0201)));
    assert_eq!(m.cpu.ip, 0);
}

#[test]
fn fault_stops_the_run() {
    let mut m = machine(&[0x90, 0xFF, 0xFF]);
    assert!(matches!(m.run_for(100), RunExit::Fault(_)));
    assert_eq!(m.cpu.ip, 1);
}

#[test]
fn hlt_wakes_on_irq_and_time_passes() {
    let mut m = machine(&IDLE);
    install_irq0_counter(&mut m);
    assert_eq!(m.run_for(3000), RunExit::Budget);
    assert!(m.cpu.halted);
    assert_eq!(m.hw.now(), 1_000_000, "halted time still passes");
    m.hw.pic.raise(0);
    assert_eq!(m.run_for(3000), RunExit::Budget);
    assert_eq!(irq0_count(&mut m), 1);
    assert!(m.cpu.halted, "back in the idle loop");
    assert_eq!(m.hw.now(), 2_000_000);
}

#[test]
fn pit_irq0_ticks_over_one_second() {
    // Program PIT ch0 as the BIOS does (mode 3, count 0), then idle.
    #[rustfmt::skip]
    let mut m = machine(&[
        0xB0, 0x36, 0xE6, 0x43, // mov al, 36h ; out 43h, al
        0x30, 0xC0,             // xor al, al
        0xE6, 0x40, 0xE6, 0x40, // out 40h, al (lo) ; out 40h, al (hi)
        0xFB,                   // sti
        0xF4, 0xEB, 0xFD,       // L: hlt ; jmp L
    ]);
    install_irq0_counter(&mut m);
    let per_ms = m.hw.cycles_per_ms() as u64;
    for _ in 0..1000 {
        m.run_for(per_ms);
    }
    assert_eq!(m.hw.now(), 1_000_000_000);
    assert_eq!(irq0_count(&mut m), 18, "1 s / {TICK_NS} ns");
}

#[test]
fn interrupt_shadow_after_sti_and_mov_ss() {
    // IRQ0 pending with IF=0; STI's shadow lets `mov ax, 1` run first.
    #[rustfmt::skip]
    let mut m = machine(&[
        0xFB,             // sti
        0xB8, 0x01, 0x00, // mov ax, 1
        0xB8, 0x02, 0x00, // mov ax, 2
        0xF4,
    ]);
    install_irq0_counter(&mut m);
    m.hw.pic.raise(0);
    m.run_for(100);
    assert_eq!(irq0_count(&mut m), 1);
    assert_eq!(
        m.hw.read16(linear(HANDLER, 0x102)),
        1,
        "AX seen by the handler"
    );

    // MOV SS shadow with IF already set.
    #[rustfmt::skip]
    let mut m = machine(&[
        0x8E, 0xD3,       // mov ss, bx
        0xB8, 0x01, 0x00, // mov ax, 1
        0xF4,
    ]);
    install_irq0_counter(&mut m);
    m.cpu.regs.set_bx(0x3000);
    m.cpu.flags |= IF;
    m.run_for(1); // one instruction (mov ss); IRQ arrives after it
    m.hw.pic.raise(0);
    m.run_for(100);
    assert_eq!(m.hw.read16(linear(HANDLER, 0x102)), 1);
}

#[test]
fn retry_halts_and_reruns_after_irq() {
    let mut m = machine(&[0xFE, 0x38, 0x07, 0x00, 0xF4]);
    install_irq0_counter(&mut m);
    let calls = Arc::new(Mutex::new(0));
    let c = calls.clone();
    m.set_handler(
        Subsystem::Bios,
        Box::new(move |_, _, _| {
            let mut n = c.lock().unwrap();
            *n += 1;
            if *n == 1 {
                HleResult::Retry
            } else {
                HleResult::Done
            }
        }),
    );
    assert_eq!(m.run_for(1000), RunExit::Budget);
    assert_eq!(*calls.lock().unwrap(), 1);
    assert!(m.cpu.halted);
    assert_eq!(m.cpu.ip, 0, "rewound to the callback");
    assert_ne!(m.cpu.flags & IF, 0);

    m.hw.pic.raise(0);
    m.run_for(1000);
    assert_eq!(irq0_count(&mut m), 1);
    assert_eq!(*calls.lock().unwrap(), 2);
    assert_eq!(m.cpu.ip, 5);
}

#[test]
fn halt_fast_forwards_through_events() {
    let mut m = machine(&IDLE);
    m.hw.sched.schedule(250_000, DeviceId(200), 0);
    m.hw.sched.schedule(600_000, DeviceId(200), 1);
    assert_eq!(m.run_for(3000), RunExit::Budget);
    // Both events fired; only the BIOS timer tick remains.
    assert!(m.hw.sched.next_due().unwrap() > 1_000_000);
    assert_eq!(m.hw.now(), 1_000_000);
}

#[test]
fn budget_stops_at_scheduler_deadline() {
    // jmp $ forever; an event at 100 cycles must fire on time.
    let mut m = machine(&[0xEB, 0xFE]);
    m.hw.sched.schedule(100_000_000 / 3000, DeviceId(200), 0); // ~100 cycles
    m.run_for(100);
    assert!(m.hw.sched.next_due().unwrap() > 100_000_000 / 3000);
    assert!(m.hw.now() <= 34_000);
}

fn run_program() -> (Vec<u8>, Cpu) {
    // Mixes ALU work with IRQs at fixed emulated times.
    #[rustfmt::skip]
    let mut m = machine(&[
        0xFB,             // sti
        0x31, 0xC0,       // xor ax, ax
        0x05, 0x07, 0x00, // L: add ax, 7
        0xD1, 0xC0,       // rol ax, 1
        0xEB, 0xF9,       // jmp L
    ]);
    install_irq0_counter(&mut m);
    for i in 0..50 {
        m.run_for(997);
        if i % 7 == 0 {
            m.hw.pic.raise(0);
        }
    }
    (m.hw.mem.ram().to_vec(), m.cpu.clone())
}

#[test]
fn deterministic() {
    let (a, ca) = run_program();
    let (b, cb) = run_program();
    assert!(a == b, "memory differs");
    assert_eq!(ca, cb);
}

#[test]
fn machine_is_send() {
    fn assert_send<T: Send>() {}
    assert_send::<Machine>();
}

#[test]
fn bios_int16_blocks_until_a_key_arrives() {
    #[rustfmt::skip]
    let mut m = machine(&[
        0xB4, 0x00,             // mov ah, 0
        0xCD, 0x16,             // int 16h
        0x2E, 0xA3, 0x00, 0x02, // mov cs:[0200h], ax
        0xFA, 0xF4,             // cli ; hlt
    ]);
    m.hw.write16(linear(CODE, 0x200), 0xFFFF);
    m.run_for(3000 * 200); // 200 ms: several timer ticks, no key
    assert!(m.cpu.halted);
    assert_eq!(m.hw.read16(linear(CODE, 0x200)), 0xFFFF);
    assert!(crate::bios::bda::read32(&mut m.hw, crate::bios::bda::TICKS) >= 3);

    assert!(crate::bios::kbd::push_key(&mut m.hw, 0x1E61));
    assert_eq!(m.run_for(3000 * 100), RunExit::Halted);
    assert_eq!(m.hw.read16(linear(CODE, 0x200)), 0x1E61);
}

#[test]
fn top_level_program_exit_stops_the_machine() {
    let mut m = Machine::new(MachineConfig::default());
    let mut c = crate::vfs::MemDrive::new();
    // mov ah,30h ; int 21h ; mov ah,4Ch ; int 21h  (exit code = DOS major)
    c.add_file(
        "\\APPS\\VER.COM",
        &[0xB4, 0x30, 0xCD, 0x21, 0xB4, 0x4C, 0xCD, 0x21],
    );
    m.mount('c', Box::new(c));
    assert_eq!(
        m.exec("C:\\APPS\\NONE.COM", ""),
        Err(DosError::FileNotFound)
    );
    m.exec("C:\\APPS\\VER.COM", "").unwrap();
    assert_eq!(m.run_for(10_000), RunExit::Exited(5));
    assert_eq!(m.run_for(10_000), RunExit::Exited(5), "sticky");
    assert!(m.dos.processes.is_empty());
}
