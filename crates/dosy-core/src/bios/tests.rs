use super::*;
use crate::bus::FlatBus;
use crate::cpu::Step;

/// Caller context: code at 1000:0100, stack 9000:FFFE, IF set.
fn setup() -> (Cpu, FlatBus) {
    let mut bus = FlatBus::new();
    install(&mut bus);
    bus.io_out.clear();
    let mut cpu = Cpu::new();
    cpu.sregs.set(Seg::Cs, 0x1000);
    cpu.ip = 0x0100;
    cpu.sregs.set(Seg::Ss, 0x9000);
    cpu.regs.set_sp(0xFFFE);
    cpu.flags |= flags::IF;
    (cpu, bus)
}

/// Enter `vector` like `INT n` and run the callback directly; returns the
/// FLAGS image that `IRET` will restore.
fn call(cpu: &mut Cpu, bus: &mut FlatBus, vector: u8, id: HleId) -> u16 {
    cpu.interrupt(bus, vector);
    assert_eq!(handle(id, cpu, bus), HleResult::Done);
    let sp = cpu.regs.sp().wrapping_add(4);
    cpu.read_mem16(bus, Seg::Ss, sp)
}

/// Run the CPU from inside an interrupt until it returns to 1000:0100,
/// dispatching BIOS callbacks.
fn run_back(cpu: &mut Cpu, bus: &mut FlatBus) {
    for _ in 0..1000 {
        if cpu.sregs.get(Seg::Cs) == 0x1000 && cpu.ip == 0x0100 {
            return;
        }
        match cpu.step(bus).0 {
            Step::Ok => {}
            Step::Callback(id) => {
                handle(id, cpu, bus);
            }
            s => panic!("unexpected {s:?}"),
        }
    }
    panic!("did not return");
}

#[test]
fn install_sets_up_rom_ivt_and_bda() {
    let mut bus = FlatBus::new();
    install(&mut bus);
    assert_eq!(bus.read8(0xF_FFFE), MODEL);
    assert_eq!(bus.read8(0xF_FF53), 0xCF);
    assert_eq!(bus.read8(0xF_FFF0), 0xEA);
    assert_eq!(bus.read16(0xF_FFF1), INT19.stub_offset());
    // Unused vector → dummy IRET; IRQs → EOI handlers; services → stubs.
    assert_eq!(bus.read32(0x21 * 4), 0xF000_FF53);
    assert_eq!(bus.read16(0x0F * 4), IRQ_MASTER.stub_offset());
    assert_eq!(bus.read16(0x74 * 4), IRQ_SLAVE.stub_offset());
    assert_eq!(bus.read16(0x08 * 4), INT08.stub_offset());
    assert_eq!(bus.read16(0x1A * 4 + 2), 0xF000);
    assert_eq!(bus.read16(linear(0xF000, INT1A.stub_offset()) + 1), 0x38FE);
    // BDA.
    assert_eq!(bda::read16(&mut bus, bda::MEMORY_KB), 640);
    assert_eq!(bda::read16(&mut bus, bda::EQUIPMENT), EQUIPMENT);
    assert_eq!(bda::read16(&mut bus, bda::KBD_HEAD), 0x1E);
    assert_eq!(bda::read16(&mut bus, bda::KBD_BUF_END), 0x3E);
    assert_eq!(bda::read8(&mut bus, bda::VIDEO_MODE), 3);
    // No RTC on FlatBus (ports read FFh): ticks start at 0.
    assert_eq!(bda::read32(&mut bus, bda::TICKS), 0);
    // PIC init, IMR, then PIT ch0 mode 3.
    let out: Vec<_> = bus.io_out.iter().filter(|(p, _)| *p != 0x70).collect();
    let want = [
        (0x20, 0x11),
        (0x21, 0x08),
        (0x21, 0x04),
        (0x21, 0x01),
        (0xA0, 0x11),
        (0xA1, 0x70),
        (0xA1, 0x02),
        (0xA1, 0x01),
        (0x21, MASTER_IMR),
        (0xA1, SLAVE_IMR),
        (0x43, 0x36),
        (0x40, 0x00),
        (0x40, 0x00),
    ];
    assert_eq!(out.into_iter().copied().collect::<Vec<_>>(), want);
}

#[test]
fn boot_ticks_come_from_the_rtc() {
    let mut bus = FlatBus::new();
    bus.io_in.insert(0x71, 0x12); // 12:12:12
    install(&mut bus);
    let secs: u64 = 12 * 3600 + 12 * 60 + 12;
    assert_eq!(
        bda::read32(&mut bus, bda::TICKS),
        (secs * 1_193_182 / 65_536) as u32
    );
}

#[test]
fn int08_counts_eois_and_chains_1c() {
    let (mut cpu, mut bus) = setup();
    // INT 1Ch handler at 0000:0500: INC byte CS:[0600]; IRET.
    bus.load(0x500, &[0x2E, 0xFE, 0x06, 0x00, 0x06, 0xCF]);
    set_vector(&mut bus, 0x1C, 0, 0x500);
    for _ in 0..2 {
        cpu.interrupt(&mut bus, 0x08);
        run_back(&mut cpu, &mut bus);
    }
    assert_eq!(bda::read32(&mut bus, bda::TICKS), 2);
    assert_eq!(bus.read8(0x600), 2);
    assert_eq!(bus.io_out, [(0x20, 0x20), (0x20, 0x20)]);
    assert_eq!(cpu.regs.sp(), 0xFFFE);
    assert_ne!(cpu.flags & flags::IF, 0, "IRET restores IF");
}

#[test]
fn tick_rollover_sets_midnight_flag() {
    let (mut cpu, mut bus) = setup();
    bda::write32(&mut bus, bda::TICKS, bda::TICKS_PER_DAY - 1);
    call(&mut cpu, &mut bus, 0x08, INT08);
    assert_eq!(bda::read32(&mut bus, bda::TICKS), 0);
    assert_eq!(bda::read8(&mut bus, bda::MIDNIGHT), 1);
    // INT 1Ah/00 reports and clears it.
    cpu.regs.set_sp(0xFFFE);
    cpu.regs.set_ax(0);
    call(&mut cpu, &mut bus, 0x1A, INT1A);
    assert_eq!((cpu.regs.cx(), cpu.regs.dx(), cpu.regs.al()), (0, 0, 1));
    assert_eq!(bda::read8(&mut bus, bda::MIDNIGHT), 0);
}

#[test]
fn int1a_ticks_and_rtc() {
    let (mut cpu, mut bus) = setup();
    cpu.regs.set_ax(0x0100);
    cpu.regs.set_cx(0x0012);
    cpu.regs.set_dx(0x3456);
    call(&mut cpu, &mut bus, 0x1A, INT1A);
    assert_eq!(bda::read32(&mut bus, bda::TICKS), 0x0012_3456);

    cpu.regs.set_sp(0xFFFE);
    cpu.regs.set_ax(0x0200);
    let f = call(&mut cpu, &mut bus, 0x1A, INT1A);
    assert_ne!(f as u32 & flags::CF, 0, "no RTC → CF=1");

    bus.io_in.insert(0x71, 0x11);
    cpu.regs.set_sp(0xFFFE);
    cpu.regs.set_ax(0x0200);
    let f = call(&mut cpu, &mut bus, 0x1A, INT1A);
    assert_eq!(f as u32 & flags::CF, 0);
    assert_eq!(
        (cpu.regs.cx(), cpu.regs.dh(), cpu.regs.dl()),
        (0x1111, 0x11, 1)
    );
    assert!(bus.io_out.ends_with(&[(0x70, 0x0B)]));

    cpu.regs.set_sp(0xFFFE);
    cpu.regs.set_ax(0x0400);
    let f = call(&mut cpu, &mut bus, 0x1A, INT1A);
    assert_eq!(f as u32 & flags::CF, 0);
    assert_eq!((cpu.regs.cx(), cpu.regs.dx()), (0x1111, 0x1111));
}

#[test]
fn int11_int12() {
    let (mut cpu, mut bus) = setup();
    call(&mut cpu, &mut bus, 0x11, INT11);
    assert_eq!(cpu.regs.ax(), EQUIPMENT);
    cpu.regs.set_sp(0xFFFE);
    call(&mut cpu, &mut bus, 0x12, INT12);
    assert_eq!(cpu.regs.ax(), 640);
}

#[test]
fn int15_services() {
    let (mut cpu, mut bus) = setup();
    let mut int15 = |cpu: &mut Cpu, ax: u16| {
        cpu.regs.set_sp(0xFFFE);
        cpu.regs.set_ax(ax);
        call(cpu, &mut bus, 0x15, INT15) as u32 & flags::CF != 0
    };
    assert!(!int15(&mut cpu, 0xC000));
    assert_eq!(
        (cpu.sregs.get(Seg::Es), cpu.regs.bx()),
        (0xF000, CONFIG_TABLE)
    );
    assert!(!int15(&mut cpu, 0x8800));
    assert_eq!(cpu.regs.ax(), 0);
    assert!(!int15(&mut cpu, 0x8600));
    assert!(int15(&mut cpu, 0x4F1E), "keyboard intercept: process key");
    assert!(int15(&mut cpu, 0xE801));
    assert_eq!(cpu.regs.ah(), 0x86);
    assert!(!int15(&mut cpu, 0x2403));
    assert_eq!(cpu.regs.bx(), 2);
    assert!(!int15(&mut cpu, 0x2401));
    assert!(int15(&mut cpu, 0x2407));
    drop(int15);
    assert_eq!(
        bus.io_out.last(),
        Some(&(0x92, 0xFE)),
        "FlatBus 92h reads FFh"
    );
    let table: Vec<u8> = (0..4).map(|i| bus.read8(0xF_E6F5 + i)).collect();
    assert_eq!(table, [8, 0, MODEL, 0]);
}

#[test]
fn int15_a20_status_reads_port_92() {
    let (mut cpu, mut bus) = setup();
    bus.io_in.insert(0x92, 0x02);
    cpu.regs.set_ax(0x2402);
    call(&mut cpu, &mut bus, 0x15, INT15);
    assert_eq!(cpu.regs.ax(), 0x0001);
    cpu.regs.set_sp(0xFFFE);
    cpu.regs.set_ax(0x2400);
    call(&mut cpu, &mut bus, 0x15, INT15);
    assert_eq!(bus.io_out.last(), Some(&(0x92, 0x00)));
}

#[test]
fn default_irq_handlers_send_eoi() {
    let (mut cpu, mut bus) = setup();
    cpu.interrupt(&mut bus, 0x0D);
    run_back(&mut cpu, &mut bus);
    cpu.interrupt(&mut bus, 0x74);
    run_back(&mut cpu, &mut bus);
    assert_eq!(bus.io_out, [(0x20, 0x20), (0xA0, 0x20), (0x20, 0x20)]);
}

#[test]
fn int19_halts() {
    let (mut cpu, mut bus) = setup();
    cpu.interrupt(&mut bus, 0x19);
    assert_eq!(cpu.step(&mut bus).0, Step::Callback(INT19));
    handle(INT19, &mut cpu, &mut bus);
    assert!(cpu.halted);
}

#[test]
fn install_on_hardware_starts_the_18_hz_timer() {
    let mut hw = crate::hw::Hardware::new();
    install(&mut hw);
    let mut irq0 = 0;
    while let Some(t) = hw.sched.next_due().filter(|&t| t <= 1_000_000_000) {
        hw.advance_to(t);
        hw.fire_due();
        if hw.pic.pending_vector() == Some(0x08) {
            hw.pic.acknowledge();
            hw.io_out8(0x20, 0x20);
            irq0 += 1;
        }
    }
    assert_eq!(irq0, 18);
    // Keyboard IRQ unmasked, IRQ3 masked.
    assert_eq!(hw.io_in8(0x21), MASTER_IMR);
    hw.irq.pulse(3);
    hw.irq.pulse(1);
    hw.io_in8(0x21);
    assert_eq!(hw.pic.pending_vector(), Some(0x09));
}
