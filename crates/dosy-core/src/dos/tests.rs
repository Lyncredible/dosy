use super::*;
use crate::bus::FlatBus;
use crate::cpu::regs::{AX, BX, CX, DX};

const STACK_SEG: u16 = 0x9000;
const STACK_SP: u16 = 0xFFF0;

fn setup() -> (Dos, Cpu, FlatBus) {
    let mut bus = FlatBus::new();
    let mut dos = Dos::new();
    dos.install(&mut bus);
    let mut cpu = Cpu::new();
    cpu.sregs.set(Seg::Ss, STACK_SEG);
    (dos, cpu, bus)
}

/// Run INT 21h with the given AX/BX/CX/DX; the IRET frame is rebuilt at
/// SS:SP each call. Returns the CF stored in that frame.
fn int21(
    dos: &mut Dos,
    cpu: &mut Cpu,
    bus: &mut FlatBus,
    ax: u16,
    bx: u16,
    cx: u16,
    dx: u16,
) -> bool {
    for (i, v) in [(AX, ax), (BX, bx), (CX, cx), (DX, dx)] {
        cpu.regs.set16(i, v);
    }
    cpu.sregs.set(Seg::Ss, STACK_SEG);
    cpu.regs.set_sp(STACK_SP);
    let frame = linear(STACK_SEG, STACK_SP);
    bus.write16(frame + 4, 0x0202);
    assert_eq!(dos.callback(ID_INT21, cpu, bus), HleResult::Done);
    let cf = bus.read16(frame + 4) & 1 != 0;
    assert_eq!(cf, cpu.flags & flags::CF != 0);
    cf
}

fn free_paras(bus: &mut FlatBus) -> u32 {
    mcb::walk(bus, FIRST_MCB)
        .unwrap()
        .iter()
        .filter(|m| m.owner == mcb::FREE)
        .map(|m| m.size as u32)
        .sum()
}

#[test]
fn install_leaves_600k_free_and_hooks_vectors() {
    let (_, _, mut bus) = setup();
    let largest = mcb::largest_free(&mcb::walk(&mut bus, FIRST_MCB).unwrap());
    assert!(
        largest as u32 * 16 >= 600 * 1024,
        "only {largest:#x} paragraphs"
    );
    let chain = mcb::walk(&mut bus, FIRST_MCB).unwrap();
    let last = chain.last().unwrap();
    assert!(last.is_last());
    assert_eq!(last.next(), 0xA000);
    for id in [ID_INT20, ID_INT21, ID_INT2F] {
        let v = (id.0 & 0xFF) as u32 * 4;
        assert_eq!(bus.read16(v), id.stub_offset());
        assert_eq!(bus.read16(v + 2), ROM_SEGMENT);
        let stub = linear(ROM_SEGMENT, id.stub_offset());
        assert!(
            bus.mem[stub as usize..stub as usize + 6]
                .windows(2)
                .any(|w| w == [0xFE, 0x38])
        );
    }
}

#[test]
fn root_psp_and_environment() {
    let (mut dos, mut cpu, mut bus) = setup();
    int21(&mut dos, &mut cpu, &mut bus, 0x6200, 0, 0, 0);
    let root = cpu.regs.bx();
    assert_eq!(root, dos.root_psp);
    let p = |off| linear(root, off);
    assert_eq!(bus.read16(p(psp::INT20)), 0x20CD);
    assert_eq!(bus.read16(p(psp::MEM_TOP)), root + ROOT_PARAS);
    assert_eq!(bus.read16(p(psp::PARENT)), root);
    assert_eq!(
        &bus.mem[p(psp::JFT) as usize..][..6],
        &[1, 1, 1, 0, 2, 0xFF]
    );
    assert_eq!(bus.read16(p(psp::DISPATCH)), 0x21CD);
    assert_eq!(bus.read8(p(psp::TAIL + 1)), 0x0D);
    // INT 22h captured from the IVT.
    assert_eq!(bus.read32(p(psp::INT22)), bus.read32(0x22 * 4));

    let env = bus.read16(p(psp::ENV));
    let m = mcb::read(&mut bus, env - 1);
    assert_eq!((m.owner, m.size), (root, ENV_PARAS));
    let text = &bus.mem[linear(env, 0) as usize..][..64];
    assert!(text.starts_with(b"COMSPEC=Z:\\COMMAND.COM\0PATH=Z:\\\0\0\x01\0Z:\\COMMAND.COM\0"));

    // DTA defaults to PSP:80h.
    int21(&mut dos, &mut cpu, &mut bus, 0x2F00, 0, 0, 0);
    assert_eq!((cpu.sregs.get(Seg::Es), cpu.regs.bx()), (root, 0x80));
}

#[test]
fn create_psp_fields() {
    let mut bus = FlatBus::new();
    psp::create_psp(&mut bus, 0x1000, 0x9FFF, 0x0800, 0x0F00);
    let p = |off| linear(0x1000, off);
    assert_eq!(bus.read16(p(0)), 0x20CD);
    assert_eq!(bus.read16(p(psp::MEM_TOP)), 0x9FFF);
    assert_eq!(bus.read16(p(psp::PARENT)), 0x0800);
    assert_eq!(bus.read16(p(psp::ENV)), 0x0F00);
    assert_eq!(bus.read16(p(psp::JFT_PTR + 2)), 0x1000);
    assert_eq!(bus.read16(p(psp::DOS_VERSION)), VERSION);
}

#[test]
fn allocate_free_and_coalesce() {
    let (mut dos, mut cpu, mut bus) = setup();
    let total = free_paras(&mut bus);
    let alloc = |dos: &mut Dos, cpu: &mut Cpu, bus: &mut FlatBus, n| {
        assert!(!int21(dos, cpu, bus, 0x4800, n, 0, 0));
        cpu.regs.ax()
    };
    let a = alloc(&mut dos, &mut cpu, &mut bus, 0x100);
    let b = alloc(&mut dos, &mut cpu, &mut bus, 0x100);
    let c = alloc(&mut dos, &mut cpu, &mut bus, 0x100);
    assert_eq!(b, a + 0x101);
    assert_eq!(c, b + 0x101);
    assert_eq!(mcb::read(&mut bus, a - 1).owner, dos.root_psp);

    // Free a and b: they merge, so a 0x201-paragraph block fits at a.
    for seg in [b, a] {
        cpu.sregs.set(Seg::Es, seg);
        assert!(!int21(&mut dos, &mut cpu, &mut bus, 0x4900, 0, 0, 0));
    }
    assert_eq!(alloc(&mut dos, &mut cpu, &mut bus, 0x201), a);

    // Too large: error 8 and BX = largest free block.
    assert!(int21(&mut dos, &mut cpu, &mut bus, 0x4800, 0xFFFF, 0, 0));
    assert_eq!(cpu.regs.ax(), ERR_NO_MEMORY);
    let largest = cpu.regs.bx();
    assert_eq!(largest as u32, total - 3 * 0x101);
    assert_eq!(alloc(&mut dos, &mut cpu, &mut bus, largest), c + 0x101);

    // Freeing everything restores the original free total.
    for seg in [a, c, c + 0x101] {
        cpu.sregs.set(Seg::Es, seg);
        assert!(!int21(&mut dos, &mut cpu, &mut bus, 0x4900, 0, 0, 0));
    }
    assert_eq!(free_paras(&mut bus), total);
    assert_eq!(mcb::walk(&mut bus, FIRST_MCB).unwrap().len(), 3);
}

#[test]
fn bad_block_and_destroyed_chain() {
    let (mut dos, mut cpu, mut bus) = setup();
    cpu.sregs.set(Seg::Es, 0x5000);
    assert!(int21(&mut dos, &mut cpu, &mut bus, 0x4900, 0, 0, 0));
    assert_eq!(cpu.regs.ax(), ERR_BAD_BLOCK);
    assert!(int21(&mut dos, &mut cpu, &mut bus, 0x4A00, 0x10, 0, 0));
    assert_eq!(cpu.regs.ax(), ERR_BAD_BLOCK);

    assert!(!int21(&mut dos, &mut cpu, &mut bus, 0x4800, 0x10, 0, 0));
    let a = cpu.regs.ax();
    bus.write8(linear(a - 1, 0), b'X');
    assert!(int21(&mut dos, &mut cpu, &mut bus, 0x4800, 0x10, 0, 0));
    assert_eq!(cpu.regs.ax(), ERR_MCB_DESTROYED);
}

#[test]
fn resize_shrinks_grows_and_reports_max() {
    let (mut dos, mut cpu, mut bus) = setup();
    assert!(!int21(&mut dos, &mut cpu, &mut bus, 0x4800, 0x100, 0, 0));
    let a = cpu.regs.ax();
    assert!(!int21(&mut dos, &mut cpu, &mut bus, 0x4800, 0x100, 0, 0));
    let b = cpu.regs.ax();
    cpu.sregs.set(Seg::Es, a);
    assert!(!int21(&mut dos, &mut cpu, &mut bus, 0x4A00, 0x80, 0, 0));
    assert_eq!(mcb::read(&mut bus, a - 1).size, 0x80);
    // The gap left behind is free and can be reclaimed.
    assert!(!int21(&mut dos, &mut cpu, &mut bus, 0x4A00, 0x100, 0, 0));
    assert_eq!(mcb::read(&mut bus, a - 1).size, 0x100);
    assert!(int21(&mut dos, &mut cpu, &mut bus, 0x4A00, 0x101, 0, 0));
    assert_eq!((cpu.regs.ax(), cpu.regs.bx()), (ERR_NO_MEMORY, 0x100));

    // The top block can grow into all remaining memory.
    cpu.sregs.set(Seg::Es, b);
    assert!(int21(&mut dos, &mut cpu, &mut bus, 0x4A00, 0xFFFF, 0, 0));
    let max = cpu.regs.bx();
    assert_eq!(max, 0xA000 - b);
    assert_eq!(
        mcb::read(&mut bus, b - 1).size,
        0x100,
        "unchanged on failure"
    );
    assert!(!int21(&mut dos, &mut cpu, &mut bus, 0x4A00, max, 0, 0));
    assert!(mcb::read(&mut bus, b - 1).is_last());
    assert_eq!(free_paras(&mut bus), 0);
}

#[test]
fn allocation_strategies() {
    let (mut dos, mut cpu, mut bus) = setup();
    let mut segs = Vec::new();
    for n in [0x200, 0x10, 0x100, 0x10] {
        assert!(!int21(&mut dos, &mut cpu, &mut bus, 0x4800, n, 0, 0));
        segs.push(cpu.regs.ax());
    }
    // Free holes of 0x200 and 0x100 paragraphs.
    for &seg in &[segs[0], segs[2]] {
        cpu.sregs.set(Seg::Es, seg);
        assert!(!int21(&mut dos, &mut cpu, &mut bus, 0x4900, 0, 0, 0));
    }
    assert!(!int21(&mut dos, &mut cpu, &mut bus, 0x5801, 1, 0, 0));
    assert!(!int21(&mut dos, &mut cpu, &mut bus, 0x5800, 0, 0, 0));
    assert_eq!(cpu.regs.ax(), 1);
    assert!(!int21(&mut dos, &mut cpu, &mut bus, 0x4800, 0x80, 0, 0));
    assert_eq!(cpu.regs.ax(), segs[2], "best fit");

    assert!(!int21(&mut dos, &mut cpu, &mut bus, 0x5801, 2, 0, 0));
    assert!(!int21(&mut dos, &mut cpu, &mut bus, 0x4800, 0x80, 0, 0));
    assert_eq!(cpu.regs.ax(), 0xA000 - 0x80, "last fit takes the top");

    assert!(!int21(&mut dos, &mut cpu, &mut bus, 0x5801, 0, 0, 0));
    assert!(!int21(&mut dos, &mut cpu, &mut bus, 0x4800, 0x80, 0, 0));
    assert_eq!(cpu.regs.ax(), segs[0], "first fit");

    assert!(int21(&mut dos, &mut cpu, &mut bus, 0x5801, 3, 0, 0));
    assert_eq!(cpu.regs.ax(), ERR_FUNCTION);
}

#[test]
fn vectors_version_psp_dta_break() {
    let (mut dos, mut cpu, mut bus) = setup();
    cpu.sregs.set(Seg::Ds, 0x1234);
    int21(&mut dos, &mut cpu, &mut bus, 0x2560, 0, 0, 0x5678);
    assert_eq!(bus.read32(0x60 * 4), 0x1234_5678);
    int21(&mut dos, &mut cpu, &mut bus, 0x3560, 0, 0, 0);
    assert_eq!((cpu.sregs.get(Seg::Es), cpu.regs.bx()), (0x1234, 0x5678));

    int21(&mut dos, &mut cpu, &mut bus, 0x3000, 0xFFFF, 0xFFFF, 0);
    assert_eq!((cpu.regs.al(), cpu.regs.ah()), (5, 0));
    assert_eq!((cpu.regs.bh(), cpu.regs.cx()), (0xFF, 0));

    int21(&mut dos, &mut cpu, &mut bus, 0x5000, 0x2000, 0, 0);
    int21(&mut dos, &mut cpu, &mut bus, 0x5100, 0, 0, 0);
    assert_eq!(cpu.regs.bx(), 0x2000);
    int21(&mut dos, &mut cpu, &mut bus, 0x6200, 0, 0, 0);
    assert_eq!(cpu.regs.bx(), 0x2000);

    int21(&mut dos, &mut cpu, &mut bus, 0x1A00, 0, 0, 0x0040);
    int21(&mut dos, &mut cpu, &mut bus, 0x2F00, 0, 0, 0);
    assert_eq!((cpu.sregs.get(Seg::Es), cpu.regs.bx()), (0x1234, 0x0040));

    int21(&mut dos, &mut cpu, &mut bus, 0x3301, 0, 0, 1);
    int21(&mut dos, &mut cpu, &mut bus, 0x3300, 0, 0, 0);
    assert_eq!(cpu.regs.dl(), 1);
    int21(&mut dos, &mut cpu, &mut bus, 0x3302, 0, 0, 0);
    assert_eq!(cpu.regs.dl(), 1);
    int21(&mut dos, &mut cpu, &mut bus, 0x3300, 0, 0, 0xFF);
    assert_eq!(cpu.regs.dl(), 0);

    int21(&mut dos, &mut cpu, &mut bus, 0x3400, 0, 0, 0);
    assert_eq!(
        (cpu.sregs.get(Seg::Es), cpu.regs.bx()),
        (DOS_SEG, SDA_OFF + 1)
    );
    int21(&mut dos, &mut cpu, &mut bus, 0x5200, 0, 0, 0);
    let lol = linear(cpu.sregs.get(Seg::Es), cpu.regs.bx());
    assert_eq!(bus.read16(lol - 2), FIRST_MCB);
}

#[test]
fn date_and_time() {
    let (mut dos, mut cpu, mut bus) = setup();
    int21(&mut dos, &mut cpu, &mut bus, 0x2A00, 0, 0, 0);
    assert_eq!(
        (cpu.regs.cx(), cpu.regs.dx(), cpu.regs.al()),
        (1980, 0x0101, 2)
    );

    int21(&mut dos, &mut cpu, &mut bus, 0x2B00, 0, 2024, 0x021D);
    assert_eq!(cpu.regs.al(), 0);
    int21(&mut dos, &mut cpu, &mut bus, 0x2B00, 0, 2023, 0x021D);
    assert_eq!(cpu.regs.al(), 0xFF, "2023-02-29 is invalid");
    bus.write8(time::BDA_MIDNIGHT, 1);
    int21(&mut dos, &mut cpu, &mut bus, 0x2A00, 0, 0, 0);
    // 2024-03-01 is a Friday.
    assert_eq!(
        (cpu.regs.cx(), cpu.regs.dx(), cpu.regs.al()),
        (2024, 0x0301, 5)
    );
    assert_eq!(bus.read8(time::BDA_MIDNIGHT), 0);

    let mut d = Date::new(2099, 12, 31).unwrap();
    d.next_day();
    assert_eq!((d.year, d.month, d.day), (2100, 1, 1));

    int21(&mut dos, &mut cpu, &mut bus, 0x2D00, 0, 0x0C22, 0x3800);
    assert_eq!(cpu.regs.al(), 0);
    int21(&mut dos, &mut cpu, &mut bus, 0x2C00, 0, 0, 0);
    assert_eq!((cpu.regs.ch(), cpu.regs.cl(), cpu.regs.dh()), (12, 34, 56));
    assert!(cpu.regs.dl() < 6);
    int21(&mut dos, &mut cpu, &mut bus, 0x2D00, 0, 0x1800, 0);
    assert_eq!(cpu.regs.al(), 0xFF);

    // Midnight from the BIOS tick count.
    bus.write32(time::BDA_TICKS, 0);
    int21(&mut dos, &mut cpu, &mut bus, 0x2C00, 0, 0, 0);
    assert_eq!((cpu.regs.cx(), cpu.regs.dx()), (0, 0));
}

#[test]
fn terminate_returns_to_parent_and_frees_memory() {
    let (mut dos, mut cpu, mut bus) = setup();
    let root = dos.root_psp;
    let before = free_paras(&mut bus);

    // Hand-built child: env + PSP block owned by the child.
    assert!(!int21(&mut dos, &mut cpu, &mut bus, 0x4800, 0x10, 0, 0));
    let env = cpu.regs.ax();
    assert!(!int21(&mut dos, &mut cpu, &mut bus, 0x4800, 0x1000, 0, 0));
    let child = cpu.regs.ax();
    for seg in [env, child] {
        let mut m = mcb::read(&mut bus, seg - 1);
        m.owner = child;
        mcb::write(&mut bus, &m);
    }
    psp::create_psp(&mut bus, child, child + 0x1000, root, env);
    bus.write32(linear(child, psp::INT22), 0x2000_0123);
    bus.write32(linear(child, psp::INT23), 0x3000_0456);
    dos.processes.push(Process {
        psp: child,
        ss: 0x8000,
        sp: 0x0100,
    });
    dos.set_current_psp(&mut bus, child);

    int21(&mut dos, &mut cpu, &mut bus, 0x4C42, 0, 0, 0);
    assert_eq!((cpu.sregs.get(Seg::Cs), cpu.ip), (0x2000, 0x0123));
    assert_eq!((cpu.sregs.get(Seg::Ss), cpu.regs.sp()), (0x8000, 0x0100));
    assert_eq!(bus.read32(0x23 * 4), 0x3000_0456);
    assert_eq!(dos.current_psp(&mut bus), root);
    assert_eq!(free_paras(&mut bus), before);
    assert!(dos.processes.is_empty());

    assert!(!int21(&mut dos, &mut cpu, &mut bus, 0x4D00, 0, 0, 0));
    assert_eq!(cpu.regs.ax(), 0x0042);
    int21(&mut dos, &mut cpu, &mut bus, 0x4D00, 0, 0, 0);
    assert_eq!(cpu.regs.ax(), 0, "return code is read once");

    // With no child running, terminate is ignored.
    let cs = cpu.sregs.get(Seg::Cs);
    int21(&mut dos, &mut cpu, &mut bus, 0x4C00, 0, 0, 0);
    assert_eq!(cpu.sregs.get(Seg::Cs), cs);
}

#[test]
fn unknown_function_sets_carry() {
    let (mut dos, mut cpu, mut bus) = setup();
    assert!(int21(&mut dos, &mut cpu, &mut bus, 0x7700, 0, 0, 0));
    assert_eq!(cpu.regs.ax(), ERR_FUNCTION);
    // Successful MCB calls clear CF in the IRET frame again.
    assert!(!int21(&mut dos, &mut cpu, &mut bus, 0x5800, 0, 0, 0));
}
