//! The real machine's [`Bus`]: memory, devices, IRQ lines and the scheduler.

pub mod cmos;
pub mod kbd;
pub mod pic;
pub mod pit;
mod ports;
pub mod sched;
pub mod vga;

use std::collections::HashSet;

pub use cmos::Cmos;
pub use kbd::Kbc;
pub use pic::Pic;
pub use pit::Pit;
pub use sched::{DeviceId, Event, Scheduler};

use crate::bus::{Addr, Bus};
use crate::memory::Memory;
pub use vga::Vga;

/// Default CPU speed (config `cycles`).
pub const DEFAULT_CYCLES_PER_MS: u32 = 3000;

/// IRQ line levels as driven by devices. `Hardware` forwards rising edges
/// to the (edge-triggered) PIC after each device call, so a `pulse` within
/// one call still counts.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct IrqLines {
    level: u16,
    rising: u16,
}

impl IrqLines {
    pub fn raise(&mut self, irq: u8) {
        self.set(irq, true);
    }
    pub fn lower(&mut self, irq: u8) {
        self.set(irq, false);
    }
    /// Raise then lower: one edge.
    pub fn pulse(&mut self, irq: u8) {
        self.raise(irq);
        self.lower(irq);
    }
    pub fn set(&mut self, irq: u8, high: bool) {
        let bit = 1u16 << irq;
        if high && self.level & bit == 0 {
            self.rising |= bit;
        }
        if high {
            self.level |= bit;
        } else {
            self.level &= !bit;
        }
    }
    pub fn level(&self, irq: u8) -> bool {
        self.level & (1 << irq) != 0
    }
    /// Current levels (bit n = IRQ n).
    pub fn levels(&self) -> u16 {
        self.level
    }
    /// Lines that rose since the last call; clears the mask.
    pub fn take_rising(&mut self) -> u16 {
        std::mem::take(&mut self.rising)
    }
}

/// What a device sees during a port access or a scheduled event.
pub struct IoCtx<'a> {
    /// Emulated ns (for events: the event's due time).
    pub now: u64,
    pub irq: &'a mut IrqLines,
    pub sched: &'a mut Scheduler,
}

/// A port-mapped device.
pub trait IoDevice {
    fn read(&mut self, port: u16, ctx: &mut IoCtx) -> u8;
    fn write(&mut self, port: u16, val: u8, ctx: &mut IoCtx);
    /// A scheduled event for this device fired.
    fn event(&mut self, _kind: u32, _ctx: &mut IoCtx) {}
}

/// The PC: memory, devices as concrete fields, IRQ lines and the scheduler.
/// The VGA handles the `A0000–BFFFF` window.
pub struct Hardware {
    pub mem: Memory,
    pub vga: Vga,
    pub irq: IrqLines,
    pub sched: Scheduler,
    pub pic: Pic,
    pub pit: Pit,
    pub kbd: Kbc,
    pub cmos: Cmos,
    /// Port 61h bits 0–3 as last written.
    port61: u8,
    now: u64,
    cycles_per_ms: u32,
    /// Sub-ns remainder of `cycles * 1e6 / cycles_per_ms`.
    cycle_rem: u64,
    warned_ports: HashSet<u16>,
}

impl Default for Hardware {
    fn default() -> Self {
        Self::new()
    }
}

impl Hardware {
    /// Devices at power-on. The scheduler starts empty; call
    /// [`start_video`](Self::start_video) to begin frame timing.
    pub fn new() -> Self {
        Self {
            mem: Memory::new(),
            vga: Vga::new(),
            irq: IrqLines::default(),
            sched: Scheduler::new(),
            pic: Pic::new(),
            pit: Pit::new(),
            kbd: Kbc::new(),
            cmos: Cmos::new(),
            port61: 0,
            now: 0,
            cycles_per_ms: DEFAULT_CYCLES_PER_MS,
            cycle_rem: 0,
            warned_ports: HashSet::new(),
        }
    }

    /// Schedule the VGA's vretrace events (frames, ~70 Hz). `Machine::new`
    /// calls this; device tests that need an idle scheduler don't.
    pub fn start_video(&mut self) {
        self.vga.start(&mut self.sched);
    }

    /// Emulated time in ns.
    pub fn now(&self) -> u64 {
        self.now
    }

    pub fn cycles_per_ms(&self) -> u32 {
        self.cycles_per_ms
    }

    /// Change CPU speed. Already-elapsed time is unaffected.
    pub fn set_cycles_per_ms(&mut self, n: u32) {
        assert!(n > 0, "cycles_per_ms must be positive");
        if n != self.cycles_per_ms {
            self.cycles_per_ms = n;
            self.cycle_rem = 0;
        }
    }

    /// Advance emulated time by `n` CPU cycles (exact, no drift).
    pub fn advance_cycles(&mut self, n: u64) {
        let cpm = self.cycles_per_ms as u64;
        let total = n * 1_000_000 + self.cycle_rem;
        self.now += total / cpm;
        self.cycle_rem = total % cpm;
    }

    /// Advance emulated time directly (e.g. HLT skipping to the next event).
    pub fn advance_to(&mut self, ns: u64) {
        if ns > self.now {
            self.now = ns;
            self.cycle_rem = 0;
        }
    }

    /// Cycles from now until `ns` (rounded up); 0 if already reached.
    pub fn cycles_until(&self, ns: u64) -> u64 {
        let ns = ns.saturating_sub(self.now) as u128;
        let rem = self.cycle_rem as u128;
        let need = (ns * self.cycles_per_ms as u128).saturating_sub(rem);
        need.div_ceil(1_000_000) as u64
    }

    /// Fire all events due at or before `now`, in order.
    pub fn fire_due(&mut self) {
        while let Some(ev) = self.sched.pop_due(self.now) {
            self.device_event(ev);
            self.sync_irqs();
        }
    }

    /// Forward rising IRQ edges to the PIC.
    fn sync_irqs(&mut self) {
        let mut rising = self.irq.take_rising();
        while rising != 0 {
            self.pic.raise(rising.trailing_zeros() as u8);
            rising &= rising - 1;
        }
    }

    /// Port 61h: bit 0 PIT ch2 gate, bit 1 speaker enable, bit 4 refresh
    /// toggle (~15 us), bit 5 PIT ch2 output.
    fn port61_in(&mut self) -> u8 {
        let refresh = ((self.now / 15_085) & 1) as u8;
        let out2 = self.pit.ch2_out(self.now) as u8;
        (self.port61 & 0x0F) | refresh << 4 | out2 << 5
    }

    fn port61_out(&mut self, val: u8) {
        self.port61 = val & 0x0F;
        self.pit.set_ch2_gate(val & 1 != 0, self.now);
    }

    /// Port 61h bit 1: speaker data enable.
    pub fn speaker_enabled(&self) -> bool {
        self.port61 & 0x02 != 0
    }

    fn unclaimed_in(&mut self, port: u16) -> u8 {
        self.warn_port(port, "read");
        0xFF
    }

    fn unclaimed_out(&mut self, port: u16, val: u8) {
        let _ = val;
        self.warn_port(port, "write");
    }

    fn warn_port(&mut self, port: u16, what: &str) {
        if self.warned_ports.insert(port) {
            log::warn!("unclaimed I/O port {port:#06x} ({what})");
        }
    }
}

impl Bus for Hardware {
    #[inline]
    fn read8(&mut self, addr: Addr) -> u8 {
        self.mem.read8(addr, &mut self.vga)
    }
    #[inline]
    fn write8(&mut self, addr: Addr, val: u8) {
        self.mem.write8(addr, val, &mut self.vga)
    }
    fn io_in8(&mut self, port: u16) -> u8 {
        let v = self.port_in(port);
        self.sync_irqs();
        v
    }
    fn io_out8(&mut self, port: u16, val: u8) {
        self.port_out(port, val);
        self.sync_irqs();
    }
    fn load_rom(&mut self, addr: Addr, bytes: &[u8]) {
        self.mem.load_rom(addr, bytes);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hle::{HleId, StubKind, install_stub};

    #[test]
    fn cycles_to_ns_follows_cycles_per_ms() {
        let mut hw = Hardware::new();
        hw.advance_cycles(3000);
        assert_eq!(hw.now(), 1_000_000);
        hw.advance_cycles(1); // 333.33 ns, remainder carried
        hw.advance_cycles(2);
        assert_eq!(hw.now(), 1_001_000);
        hw.set_cycles_per_ms(6000);
        hw.advance_cycles(3000);
        assert_eq!(hw.now(), 1_501_000);
        hw.set_cycles_per_ms(7);
        for _ in 0..7 {
            hw.advance_cycles(1);
        }
        assert_eq!(hw.now(), 2_501_000);
    }

    #[test]
    fn cycles_until_reaches_deadline() {
        let mut hw = Hardware::new();
        hw.advance_cycles(1);
        let n = hw.cycles_until(1_000_000);
        assert_eq!(n, 2999);
        hw.advance_cycles(n);
        assert!(hw.now() >= 1_000_000);
        assert_eq!(hw.cycles_until(5), 0);
    }

    #[test]
    fn unclaimed_ports_read_ff() {
        let mut hw = Hardware::new();
        assert_eq!(hw.io_in8(0x1234), 0xFF);
        assert_eq!(hw.io_in8(0x1234), 0xFF);
        hw.io_out8(0x1234, 1);
        assert_eq!(hw.io_in16(0x300), 0xFFFF);
    }

    #[test]
    fn bus_applies_a20_rom_and_load_rom() {
        let mut hw = Hardware::new();
        hw.write8(0x10_0000, 5);
        assert_eq!(hw.read8(0), 5);
        let addr = install_stub(&mut hw, HleId(0x0010), StubKind::Iret);
        assert_eq!(hw.read16(addr), 0x38FE);
        hw.write8(addr, 0);
        assert_eq!(hw.read8(addr), 0xFE);
    }

    #[test]
    fn irq_lines_track_changes() {
        let mut l = IrqLines::default();
        l.raise(0);
        l.raise(0);
        l.raise(12);
        assert_eq!(l.take_rising(), 1 | 1 << 12);
        assert_eq!(l.take_rising(), 0);
        l.lower(0);
        assert!(!l.level(0) && l.level(12));
        assert_eq!(l.levels(), 1 << 12);
        assert_eq!(l.take_rising(), 0);
        l.pulse(3);
        l.raise(12);
        assert_eq!((l.take_rising(), l.levels()), (1 << 3, 1 << 12));
    }

    #[test]
    fn port_92_controls_a20() {
        let mut hw = Hardware::new();
        assert_eq!(hw.io_in8(0x92), 0);
        hw.io_out8(0x92, 0x02);
        assert!(hw.mem.a20());
        assert_eq!(hw.io_in8(0x92), 0x02);
        hw.write8(0x10_0000, 1);
        assert_eq!(hw.read8(0), 0);
    }

    #[test]
    fn load_rom_is_robust_at_boundaries() {
        let mut hw = Hardware::new();
        hw.load_rom(0xF_FFFF, &[0xAA, 0xBB]);
        assert_eq!(hw.read8(0), 0xBB, "A20 off wraps per byte");
        hw.load_rom(crate::memory::RAM_SIZE as Addr - 1, &[1, 2, 3]);
    }

    #[test]
    fn irq_edges_reach_the_pic() {
        let mut hw = Hardware::new();
        hw.irq.pulse(1);
        hw.irq.raise(14);
        hw.io_out8(0x21, 0x00); // any device call syncs
        assert_eq!(hw.pic.acknowledge(), Some(0x09));
        hw.io_out8(0x20, 0x20);
        assert_eq!(hw.pic.acknowledge(), Some(0x76));
        assert_eq!(hw.io_in8(0x21), 0x00);
    }

    #[test]
    fn fire_due_pops_events_up_to_now() {
        let mut hw = Hardware::new();
        hw.sched.schedule(500, DeviceId(200), 0);
        hw.sched.schedule(2_000_000, DeviceId(200), 0);
        hw.advance_cycles(3000);
        hw.fire_due();
        assert_eq!(hw.sched.next_due(), Some(2_000_000));
        hw.advance_to(hw.sched.next_due().unwrap());
        hw.fire_due();
        assert!(hw.sched.is_empty());
    }
}
