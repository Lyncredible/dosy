//! PC BIOS as high-level emulation: BDA, IVT, ROM stubs and the core
//! interrupts. Callback ids are `0x00xx`, mostly the vector number.
//! INT 09h/16h (keyboard) and INT 10h (016) live in their own modules.

pub mod bda;

use crate::bus::{Addr, Bus, linear};
use crate::cpu::{Cpu, Seg, flags};
use crate::hle::{CALLBACK_OPCODE, HleId, HleResult, ROM_SEGMENT, StubKind, install_stub};

pub const INT08: HleId = HleId(HleId::BIOS_BASE + 0x08);
pub const INT11: HleId = HleId(HleId::BIOS_BASE + 0x11);
pub const INT12: HleId = HleId(HleId::BIOS_BASE + 0x12);
pub const INT15: HleId = HleId(HleId::BIOS_BASE + 0x15);
pub const INT19: HleId = HleId(HleId::BIOS_BASE + 0x19);
pub const INT1A: HleId = HleId(HleId::BIOS_BASE + 0x1A);
/// Default handler for unclaimed master IRQs (EOI to the master PIC).
pub const IRQ_MASTER: HleId = HleId(HleId::BIOS_BASE + 0xF0);
/// Default handler for unclaimed slave IRQs (EOI to both PICs).
pub const IRQ_SLAVE: HleId = HleId(HleId::BIOS_BASE + 0xF1);

/// `F000:FF53`: a bare `IRET` for unused vectors (as in the IBM BIOS).
pub const DUMMY_IRET: u16 = 0xFF53;
/// `F000:E6F5`: INT 15h/C0h system configuration table.
pub const CONFIG_TABLE: u16 = 0xE6F5;
/// AT model byte at `F000:FFFE`.
pub const MODEL: u8 = 0xFC;
/// Equipment word: 80x25 colour video, no FPU, no floppies.
pub const EQUIPMENT: u16 = 0x0020;
/// Master IMR after init: only timer, keyboard and cascade unmasked.
pub const MASTER_IMR: u8 = 0xF8;
pub const SLAVE_IMR: u8 = 0xFF;

fn rom(off: u16) -> Addr {
    linear(ROM_SEGMENT, off)
}

/// Point interrupt `vector` at `seg:off`.
pub fn set_vector<B: Bus>(bus: &mut B, vector: u8, seg: u16, off: u16) {
    let a = vector as Addr * 4;
    bus.write16(a, off);
    bus.write16(a + 2, seg);
}

/// Install a stub for `id` and point `vector` at it.
fn hook<B: Bus>(bus: &mut B, vector: u8, id: HleId, kind: StubKind) {
    install_stub(bus, id, kind);
    set_vector(bus, vector, ROM_SEGMENT, id.stub_offset());
}

/// Power-on: IVT, ROM stubs and tables, BDA, PIC and PIT. Leaves the CPU
/// alone; the machine decides where execution starts.
pub fn install<B: Bus>(bus: &mut B) {
    // ROM: dummy IRET, config table, date, model, reset vector.
    bus.load_rom(rom(DUMMY_IRET), &[0xCF]);
    bus.load_rom(
        rom(CONFIG_TABLE),
        &[8, 0, MODEL, 0x00, 0x01, 0x60, 0, 0, 0, 0],
    );
    bus.load_rom(rom(0xFFF5), b"01/01/92");
    bus.load_rom(rom(0xFFFE), &[MODEL]);
    let [lo, hi] = INT19.stub_offset().to_le_bytes();
    let [slo, shi] = ROM_SEGMENT.to_le_bytes();
    bus.load_rom(rom(0xFFF0), &[0xEA, lo, hi, slo, shi]);

    // IVT: everything to the dummy IRET, IRQs to EOI-only handlers.
    for v in 0..=255u8 {
        set_vector(bus, v, ROM_SEGMENT, DUMMY_IRET);
    }
    install_stub(bus, IRQ_MASTER, StubKind::Iret);
    install_stub(bus, IRQ_SLAVE, StubKind::Iret);
    for i in 0..8 {
        set_vector(bus, 0x08 + i, ROM_SEGMENT, IRQ_MASTER.stub_offset());
        set_vector(bus, 0x70 + i, ROM_SEGMENT, IRQ_SLAVE.stub_offset());
    }

    // INT 08h: callback (ticks + EOI), then chain INT 1Ch.
    let [c0, c1] = CALLBACK_OPCODE;
    let [lo, hi] = INT08.0.to_le_bytes();
    let int08 = [c0, c1, lo, hi, 0xCD, 0x1C, 0xCF, 0x90];
    bus.load_rom(rom(INT08.stub_offset()), &int08);
    set_vector(bus, 0x08, ROM_SEGMENT, INT08.stub_offset());
    hook(bus, 0x11, INT11, StubKind::Iret);
    hook(bus, 0x12, INT12, StubKind::Iret);
    hook(bus, 0x15, INT15, StubKind::Iret);
    hook(bus, 0x19, INT19, StubKind::Loop);
    hook(bus, 0x1A, INT1A, StubKind::IretSti);

    install_bda(bus);
    init_pic(bus);
    // PIT channel 0: mode 3, count 65536 (18.2 Hz).
    bus.io_out8(0x43, 0x36);
    bus.io_out8(0x40, 0x00);
    bus.io_out8(0x40, 0x00);
    let ticks = rtc_ticks(bus).unwrap_or(0);
    bda::write32(bus, bda::TICKS, ticks);
}

fn install_bda<B: Bus>(bus: &mut B) {
    for off in 0..0x100 {
        bda::write8(bus, off, 0);
    }
    bda::write16(bus, bda::EQUIPMENT, EQUIPMENT);
    bda::write16(bus, bda::MEMORY_KB, 640);
    bda::write16(bus, bda::KBD_HEAD, bda::KBD_BUF_DEFAULT);
    bda::write16(bus, bda::KBD_TAIL, bda::KBD_BUF_DEFAULT);
    bda::write16(bus, bda::KBD_BUF_START, bda::KBD_BUF_DEFAULT);
    bda::write16(bus, bda::KBD_BUF_END, bda::KBD_BUF_DEFAULT_END);
    bda::write8(bus, bda::KBD_FLAGS3, 0x10); // enhanced keyboard
    // Video: mode 3 on VGA (016 owns these from here on).
    bda::write8(bus, bda::VIDEO_MODE, 0x03);
    bda::write16(bus, bda::VIDEO_COLS, 80);
    bda::write16(bus, bda::VIDEO_PAGE_SIZE, 0x1000);
    bda::write16(bus, bda::CURSOR_SHAPE, 0x0607);
    bda::write16(bus, bda::CRTC_BASE, 0x3D4);
    bda::write8(bus, bda::VIDEO_MODE_CTRL, 0x29);
    bda::write8(bus, bda::VIDEO_PALETTE, 0x30);
    bda::write8(bus, bda::VIDEO_ROWS, 24);
    bda::write8(bus, bda::CHAR_HEIGHT, 16);
    bda::write8(bus, bda::VIDEO_CTL, 0x60);
    bda::write8(bus, bda::VIDEO_SWITCHES, 0x09);
    bda::write8(bus, bda::VGA_FLAGS, 0x11);
}

/// AT init sequence: vectors 08h/70h, cascade on IR2, 8086 mode.
fn init_pic<B: Bus>(bus: &mut B) {
    for (port, v) in [(0x20, 0x11), (0x21, 0x08), (0x21, 0x04), (0x21, 0x01)] {
        bus.io_out8(port, v);
    }
    for (port, v) in [(0xA0, 0x11), (0xA1, 0x70), (0xA1, 0x02), (0xA1, 0x01)] {
        bus.io_out8(port, v);
    }
    bus.io_out8(0x21, MASTER_IMR);
    bus.io_out8(0xA1, SLAVE_IMR);
}

/// Dispatch a BIOS callback.
pub fn handle<B: Bus>(id: HleId, cpu: &mut Cpu, bus: &mut B) -> HleResult {
    match id {
        INT08 => int08(bus),
        INT11 => cpu.regs.set_ax(bda::read16(bus, bda::EQUIPMENT)),
        INT12 => cpu.regs.set_ax(bda::read16(bus, bda::MEMORY_KB)),
        INT15 => int15(cpu, bus),
        INT19 => {
            log::warn!("INT 19h: no bootable disk; halting");
            cpu.halted = true;
        }
        INT1A => int1a(cpu, bus),
        IRQ_MASTER => bus.io_out8(0x20, 0x20),
        IRQ_SLAVE => {
            bus.io_out8(0xA0, 0x20);
            bus.io_out8(0x20, 0x20);
        }
        _ => log::warn!("unknown BIOS callback {:#06x}", id.0),
    }
    HleResult::Done
}

/// Set or clear `mask` in the FLAGS image that the entering `INT` pushed
/// (`SS:SP+4` inside an `Iret`/`IretSti` stub), and in the live flags.
pub fn set_caller_flag<B: Bus>(cpu: &mut Cpu, bus: &mut B, mask: u32, on: bool) {
    let sp = cpu.regs.sp().wrapping_add(4);
    let f = cpu.read_mem16(bus, Seg::Ss, sp) as u32;
    let f = if on { f | mask } else { f & !mask };
    cpu.write_mem16(bus, Seg::Ss, sp, f as u16);
    if on {
        cpu.flags |= mask;
    } else {
        cpu.flags &= !mask;
    }
}

fn set_cf<B: Bus>(cpu: &mut Cpu, bus: &mut B, cf: bool) {
    set_caller_flag(cpu, bus, flags::CF, cf);
}

/// Unsupported function: CF=1, AH=86h.
fn unsupported<B: Bus>(cpu: &mut Cpu, bus: &mut B, int: u8) {
    log::debug!("INT {int:02X}h AX={:04X}: unsupported", cpu.regs.ax());
    cpu.regs.set_ah(0x86);
    set_cf(cpu, bus, true);
}

fn int08<B: Bus>(bus: &mut B) {
    let mut t = bda::read32(bus, bda::TICKS).wrapping_add(1);
    if t >= bda::TICKS_PER_DAY {
        t = 0;
        bda::write8(bus, bda::MIDNIGHT, 1);
    }
    bda::write32(bus, bda::TICKS, t);
    bus.io_out8(0x20, 0x20);
}

fn int15<B: Bus>(cpu: &mut Cpu, bus: &mut B) {
    match cpu.regs.ah() {
        0xC0 => {
            cpu.sregs.set(Seg::Es, ROM_SEGMENT);
            cpu.regs.set_bx(CONFIG_TABLE);
            cpu.regs.set_ah(0);
            set_cf(cpu, bus, false);
        }
        0x24 => {
            let p92 = bus.io_in8(0x92);
            match cpu.regs.al() {
                0 => bus.io_out8(0x92, p92 & !0x03),
                1 => bus.io_out8(0x92, (p92 | 0x02) & !0x01),
                2 => cpu.regs.set_al((p92 >> 1) & 1),
                3 => cpu.regs.set_bx(0x0002), // fast A20 via port 92h
                _ => return unsupported(cpu, bus, 0x15),
            }
            cpu.regs.set_ah(0);
            set_cf(cpu, bus, false);
        }
        // Wait CX:DX us: returns at once (no wall clock in the core).
        0x86 => {
            cpu.regs.set_ah(0);
            set_cf(cpu, bus, false);
        }
        0x88 => {
            cpu.regs.set_ax(0);
            set_cf(cpu, bus, false);
        }
        _ => unsupported(cpu, bus, 0x15),
    }
}

fn cmos<B: Bus>(bus: &mut B, reg: u8) -> u8 {
    bus.io_out8(0x70, reg);
    bus.io_in8(0x71)
}

fn bcd(v: u8, max: u8) -> Option<u8> {
    let (hi, lo) = (v >> 4, v & 0xF);
    let n = hi * 10 + lo;
    (hi < 10 && lo < 10 && n <= max).then_some(n)
}

/// RTC time (BCD) as `(h, m, s)` registers, or `None` if not running.
fn rtc_time<B: Bus>(bus: &mut B) -> Option<(u8, u8, u8)> {
    let (h, m, s) = (cmos(bus, 4), cmos(bus, 2), cmos(bus, 0));
    bcd(h, 23)?;
    bcd(m, 59)?;
    bcd(s, 59)?;
    Some((h, m, s))
}

/// Tick count for the RTC's time of day.
fn rtc_ticks<B: Bus>(bus: &mut B) -> Option<u32> {
    let (h, m, s) = rtc_time(bus)?;
    let secs = bcd(h, 23)? as u64 * 3600 + bcd(m, 59)? as u64 * 60 + bcd(s, 59)? as u64;
    Some((secs * 1_193_182 / 65_536) as u32)
}

fn int1a<B: Bus>(cpu: &mut Cpu, bus: &mut B) {
    match cpu.regs.ah() {
        0x00 => {
            let t = bda::read32(bus, bda::TICKS);
            cpu.regs.set_cx((t >> 16) as u16);
            cpu.regs.set_dx(t as u16);
            cpu.regs.set_al(bda::read8(bus, bda::MIDNIGHT));
            bda::write8(bus, bda::MIDNIGHT, 0);
        }
        0x01 => {
            let t = (cpu.regs.cx() as u32) << 16 | cpu.regs.dx() as u32;
            bda::write32(bus, bda::TICKS, t);
            bda::write8(bus, bda::MIDNIGHT, 0);
        }
        0x02 => match rtc_time(bus) {
            Some((h, m, s)) => {
                cpu.regs.set_ch(h);
                cpu.regs.set_cl(m);
                cpu.regs.set_dh(s);
                cpu.regs.set_dl(cmos(bus, 0x0B) & 1);
                set_cf(cpu, bus, false);
            }
            None => set_cf(cpu, bus, true),
        },
        0x04 => {
            let (c, y, mo, d) = (cmos(bus, 0x32), cmos(bus, 9), cmos(bus, 8), cmos(bus, 7));
            let ok = bcd(c, 99).is_some()
                && bcd(y, 99).is_some()
                && bcd(mo, 12).is_some_and(|m| m >= 1)
                && bcd(d, 31).is_some_and(|d| d >= 1);
            if ok {
                cpu.regs.set_ch(c);
                cpu.regs.set_cl(y);
                cpu.regs.set_dh(mo);
                cpu.regs.set_dl(d);
            }
            set_cf(cpu, bus, !ok);
        }
        _ => set_cf(cpu, bus, true),
    }
}

#[cfg(test)]
mod tests;
