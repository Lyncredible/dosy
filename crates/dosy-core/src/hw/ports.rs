//! Port and event routing: one `match` arm per device.
//!
//! To add a device `foo` (a field of `Hardware` implementing `IoDevice`):
//! ```text
//! port_in:      0x40..=0x43 => self.foo.read(port, ctx!(self)),
//! port_out:     0x40..=0x43 => self.foo.write(port, val, ctx!(self)),
//! device_event: DeviceId::FOO => self.foo.event(ev.kind, ctx!(self, ev.at)),
//! ```

use super::{DeviceId, Event, Hardware, IoCtx, IoDevice};
use crate::memory::Mmio;

/// `&mut IoCtx` borrowing the hardware's clock, IRQ lines and scheduler.
macro_rules! ctx {
    ($hw:ident) => {
        ctx!($hw, $hw.now)
    };
    ($hw:ident, $now:expr) => {
        &mut IoCtx {
            now: $now,
            irq: &mut $hw.irq,
            sched: &mut $hw.sched,
        }
    };
}

impl<M: Mmio> Hardware<M> {
    pub(super) fn port_in(&mut self, port: u16) -> u8 {
        match port {
            0x20 | 0x21 | 0xA0 | 0xA1 => self.pic.read(port, ctx!(self)),
            0x40..=0x43 => self.pit.read(port, ctx!(self)),
            0x61 => self.port61_in(),
            _ => self.unclaimed_in(port),
        }
    }

    pub(super) fn port_out(&mut self, port: u16, val: u8) {
        match port {
            0x20 | 0x21 | 0xA0 | 0xA1 => self.pic.write(port, val, ctx!(self)),
            0x40..=0x43 => self.pit.write(port, val, ctx!(self)),
            0x61 => self.port61_out(val),
            _ => self.unclaimed_out(port, val),
        }
    }

    pub(super) fn device_event(&mut self, ev: Event) {
        match ev.dev {
            DeviceId::PIT => self.pit.event(ev.kind, ctx!(self, ev.at)),
            _ => log::warn!("event for unknown device {:?} kind {}", ev.dev, ev.kind),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hw::IoDevice;

    /// Echoes the last write, raises IRQ 3 and schedules an event on write.
    #[derive(Default)]
    struct Echo {
        last: u8,
        events: Vec<(u32, u64)>,
    }
    impl IoDevice for Echo {
        fn read(&mut self, _port: u16, _ctx: &mut IoCtx) -> u8 {
            self.last
        }
        fn write(&mut self, _port: u16, val: u8, ctx: &mut IoCtx) {
            self.last = val;
            ctx.irq.raise(3);
            ctx.sched.schedule(ctx.now + 100, DeviceId(200), val as u32);
        }
        fn event(&mut self, kind: u32, ctx: &mut IoCtx) {
            self.events.push((kind, ctx.now));
        }
    }

    #[test]
    fn ctx_macro_lends_clock_irqs_and_scheduler() {
        let mut hw = Hardware::new();
        let mut dev = Echo::default();
        hw.advance_to(1000);
        let hw = &mut hw;
        dev.write(0x300, 7, ctx!(hw));
        assert_eq!(dev.read(0x300, ctx!(hw)), 7);
        assert!(hw.irq.level(3));
        let ev = hw.sched.pop_due(u64::MAX).unwrap();
        assert_eq!((ev.at, ev.kind), (1100, 7));
        dev.event(ev.kind, ctx!(hw, ev.at));
        assert_eq!(dev.events, vec![(7, 1100)]);
    }
}
