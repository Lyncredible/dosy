//! 8259A programmable interrupt controllers: master (20h/21h) and slave
//! (A0h/A1h) cascaded on master IR2. Edge triggered: a rising IRQ line
//! latches IRR until the CPU acknowledges it.

use super::{IoCtx, IoDevice};

/// Cascade input on the master.
const CASCADE: u8 = 2;

#[derive(Clone, Debug)]
struct Chip {
    irr: u8,
    isr: u8,
    imr: u8,
    base: u8,
    /// Lowest-priority level; the highest is `lowest + 1` (mod 8).
    lowest: u8,
    /// Next init byte expected: 0 = initialized, 2..=4 = ICW2..ICW4.
    icw: u8,
    single: bool,
    need_icw4: bool,
    auto_eoi: bool,
    read_isr: bool,
    poll: bool,
}

impl Chip {
    fn new(base: u8) -> Self {
        Self {
            irr: 0,
            isr: 0,
            imr: 0,
            base,
            lowest: 7,
            icw: 0,
            single: false,
            need_icw4: true,
            auto_eoi: false,
            read_isr: false,
            poll: false,
        }
    }

    /// Levels in priority order, highest first.
    fn by_priority(&self) -> impl Iterator<Item = u8> + use<> {
        let first = (self.lowest + 1) & 7;
        (0..8).map(move |i| (first + i) & 7)
    }

    /// Highest-priority unmasked request not blocked by an in-service level
    /// of equal or higher priority. `extra` adds request bits (cascade).
    fn best(&self, extra: u8) -> Option<u8> {
        let req = (self.irr | extra) & !self.imr;
        for l in self.by_priority() {
            if self.isr & (1 << l) != 0 {
                return None;
            }
            if req & (1 << l) != 0 {
                return Some(l);
            }
        }
        None
    }

    /// INTA for `level`: move it from IRR to ISR (unless auto-EOI).
    fn ack(&mut self, level: u8) {
        self.irr &= !(1 << level);
        // Rotation in AEOI mode is not modelled.
        if !self.auto_eoi {
            self.isr |= 1 << level;
        }
    }

    fn eoi_nonspecific(&mut self) -> Option<u8> {
        let l = self.by_priority().find(|l| self.isr & (1 << l) != 0)?;
        self.isr &= !(1 << l);
        Some(l)
    }

    fn write(&mut self, a0: bool, val: u8) {
        if !a0 && val & 0x10 != 0 {
            // ICW1.
            *self = Self {
                base: self.base,
                single: val & 0x02 != 0,
                need_icw4: val & 0x01 != 0,
                icw: 2,
                ..Self::new(self.base)
            };
            if val & 0x08 != 0 {
                log::warn!("8259: level-triggered mode not supported");
            }
            return;
        }
        if a0 && self.icw != 0 {
            match self.icw {
                2 => {
                    self.base = val & 0xF8;
                    self.icw = if !self.single {
                        3
                    } else if self.need_icw4 {
                        4
                    } else {
                        0
                    };
                }
                3 => self.icw = if self.need_icw4 { 4 } else { 0 },
                _ => {
                    self.auto_eoi = val & 0x02 != 0;
                    self.icw = 0;
                }
            }
            return;
        }
        if a0 {
            self.imr = val; // OCW1
        } else if val & 0x08 == 0 {
            self.ocw2(val);
        } else {
            // OCW3.
            if val & 0x02 != 0 {
                self.read_isr = val & 0x01 != 0;
            }
            self.poll = val & 0x04 != 0;
            if val & 0x40 != 0 {
                log::warn!("8259: special mask mode not supported");
            }
        }
    }

    fn ocw2(&mut self, val: u8) {
        let level = val & 7;
        match val >> 5 {
            0b001 => {
                self.eoi_nonspecific();
            }
            0b011 => self.isr &= !(1 << level),
            0b101 => {
                if let Some(l) = self.eoi_nonspecific() {
                    self.lowest = l;
                }
            }
            0b111 => {
                self.isr &= !(1 << level);
                self.lowest = level;
            }
            0b110 => self.lowest = level,
            _ => {} // rotate-in-AEOI on/off, no-op
        }
    }

    fn read(&mut self, a0: bool) -> u8 {
        if a0 {
            self.imr
        } else if self.read_isr {
            self.isr
        } else {
            self.irr
        }
    }
}

/// Master + slave pair.
#[derive(Clone, Debug)]
pub struct Pic {
    master: Chip,
    slave: Chip,
}

impl Default for Pic {
    fn default() -> Self {
        Self::new()
    }
}

impl Pic {
    /// State after the PC BIOS init: vectors 08h and 70h, nothing masked.
    pub fn new() -> Self {
        Self {
            master: Chip::new(0x08),
            slave: Chip::new(0x70),
        }
    }

    /// Rising edge on IRQ 0–15.
    pub fn raise(&mut self, irq: u8) {
        match irq {
            0..=7 => self.master.irr |= 1 << irq,
            8..=15 => self.slave.irr |= 1 << (irq - 8),
            _ => {}
        }
    }

    fn cascade_req(&self) -> u8 {
        if self.slave.best(0).is_some() {
            1 << CASCADE
        } else {
            0
        }
    }

    /// Vector the CPU would get if it acknowledged now.
    pub fn pending_vector(&self) -> Option<u8> {
        let l = self.master.best(self.cascade_req())?;
        if l == CASCADE && self.master.irr & (1 << CASCADE) == 0 {
            let s = self.slave.best(0)?;
            return Some(self.slave.base | s);
        }
        Some(self.master.base | l)
    }

    /// INTA cycle: returns the vector and marks it in service.
    pub fn acknowledge(&mut self) -> Option<u8> {
        let l = self.master.best(self.cascade_req())?;
        if l == CASCADE && self.master.irr & (1 << CASCADE) == 0 {
            let s = self.slave.best(0)?;
            self.master.ack(CASCADE);
            self.slave.ack(s);
            return Some(self.slave.base | s);
        }
        self.master.ack(l);
        Some(self.master.base | l)
    }

    fn chip(&mut self, port: u16) -> &mut Chip {
        if port & 0x80 != 0 {
            &mut self.slave
        } else {
            &mut self.master
        }
    }

    /// Poll command result: `0x80 | level` with the request acknowledged.
    fn poll(&mut self, slave: bool) -> u8 {
        let chip = if slave {
            &mut self.slave
        } else {
            &mut self.master
        };
        chip.poll = false;
        match chip.best(0) {
            Some(l) => {
                chip.ack(l);
                0x80 | l
            }
            None => 0,
        }
    }
}

impl IoDevice for Pic {
    fn read(&mut self, port: u16, _ctx: &mut IoCtx) -> u8 {
        let a0 = port & 1 != 0;
        if !a0 && self.chip(port).poll {
            return self.poll(port & 0x80 != 0);
        }
        self.chip(port).read(a0)
    }

    fn write(&mut self, port: u16, val: u8, _ctx: &mut IoCtx) {
        self.chip(port).write(port & 1 != 0, val);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hw::{IrqLines, Scheduler};

    fn out(pic: &mut Pic, port: u16, val: u8) {
        let (mut irq, mut sched) = (IrqLines::default(), Scheduler::new());
        let ctx = &mut IoCtx {
            now: 0,
            irq: &mut irq,
            sched: &mut sched,
        };
        pic.write(port, val, ctx);
    }

    fn inp(pic: &mut Pic, port: u16) -> u8 {
        let (mut irq, mut sched) = (IrqLines::default(), Scheduler::new());
        let ctx = &mut IoCtx {
            now: 0,
            irq: &mut irq,
            sched: &mut sched,
        };
        pic.read(port, ctx)
    }

    /// The AT BIOS sequence: ICW1 11h, ICW2 base, ICW3 04h/02h, ICW4 01h.
    fn bios_init(pic: &mut Pic, master: u8, slave: u8) {
        for (port, v) in [(0x20, 0x11), (0x21, master), (0x21, 0x04), (0x21, 0x01)] {
            out(pic, port, v);
        }
        for (port, v) in [(0xA0, 0x11), (0xA1, slave), (0xA1, 0x02), (0xA1, 0x01)] {
            out(pic, port, v);
        }
    }

    #[test]
    fn init_sequence_sets_vectors_and_clears_mask() {
        let mut pic = Pic::new();
        out(&mut pic, 0x21, 0xFF);
        bios_init(&mut pic, 0x50, 0x68);
        assert_eq!(inp(&mut pic, 0x21), 0, "ICW1 clears IMR");
        pic.raise(1);
        assert_eq!(pic.pending_vector(), Some(0x51));
        pic.raise(12);
        assert_eq!(pic.acknowledge(), Some(0x51));
        out(&mut pic, 0x20, 0x20);
        assert_eq!(pic.acknowledge(), Some(0x6C));
    }

    #[test]
    fn single_mode_without_icw4_skips_icw3() {
        let mut pic = Pic::new();
        out(&mut pic, 0x20, 0x12); // single, no ICW4
        out(&mut pic, 0x21, 0x20);
        out(&mut pic, 0x21, 0x01); // OCW1, not ICW3
        assert_eq!(inp(&mut pic, 0x21), 0x01);
        pic.raise(1);
        assert_eq!(pic.acknowledge(), Some(0x21));
    }

    #[test]
    fn priority_and_nesting() {
        let mut pic = Pic::new();
        pic.raise(3);
        pic.raise(1);
        assert_eq!(pic.acknowledge(), Some(0x09));
        // IR3 is lower priority than in-service IR1: blocked.
        assert_eq!(pic.pending_vector(), None);
        pic.raise(0);
        assert_eq!(pic.acknowledge(), Some(0x08), "higher priority nests");
        out(&mut pic, 0x20, 0x20); // EOI clears IR0
        assert_eq!(pic.pending_vector(), None);
        out(&mut pic, 0x20, 0x20); // EOI clears IR1
        assert_eq!(pic.acknowledge(), Some(0x0B));
        assert_eq!(pic.acknowledge(), None);
    }

    #[test]
    fn masking() {
        let mut pic = Pic::new();
        out(&mut pic, 0x21, 0x01);
        pic.raise(0);
        assert_eq!(pic.pending_vector(), None);
        out(&mut pic, 0x21, 0x00);
        assert_eq!(pic.acknowledge(), Some(0x08));
        // Masking the cascade input hides the slave.
        out(&mut pic, 0x20, 0x20);
        out(&mut pic, 0x21, 0x04);
        pic.raise(8);
        assert_eq!(pic.pending_vector(), None);
        out(&mut pic, 0x21, 0x00);
        out(&mut pic, 0xA1, 0x01);
        assert_eq!(pic.pending_vector(), None);
        out(&mut pic, 0xA1, 0x00);
        assert_eq!(pic.pending_vector(), Some(0x70));
    }

    #[test]
    fn specific_eoi_and_isr_irr_reads() {
        let mut pic = Pic::new();
        pic.raise(4);
        pic.raise(6);
        assert_eq!(inp(&mut pic, 0x20), 0x50, "IRR by default");
        pic.acknowledge();
        out(&mut pic, 0x20, 0x0B); // OCW3: read ISR
        assert_eq!(inp(&mut pic, 0x20), 0x10);
        out(&mut pic, 0x20, 0x66); // specific EOI for IR6 (not in service)
        assert_eq!(inp(&mut pic, 0x20), 0x10);
        out(&mut pic, 0x20, 0x64); // specific EOI for IR4
        assert_eq!(inp(&mut pic, 0x20), 0x00);
        out(&mut pic, 0x20, 0x0A); // back to IRR
        assert_eq!(inp(&mut pic, 0x20), 0x40);
    }

    #[test]
    fn cascade_vectors_and_eoi_on_both_chips() {
        let mut pic = Pic::new();
        pic.raise(14);
        pic.raise(3);
        assert_eq!(pic.acknowledge(), Some(0x76), "IR2 (cascade) beats IR3");
        out(&mut pic, 0x20, 0x0B);
        out(&mut pic, 0xA0, 0x0B);
        assert_eq!(inp(&mut pic, 0x20), 0x04);
        assert_eq!(inp(&mut pic, 0xA0), 0x40);
        pic.raise(9);
        // Master IR2 in service blocks further slave requests (fully nested).
        assert_eq!(pic.pending_vector(), None);
        out(&mut pic, 0xA0, 0x20);
        out(&mut pic, 0x20, 0x20);
        assert_eq!(pic.acknowledge(), Some(0x71));
        out(&mut pic, 0xA0, 0x20);
        out(&mut pic, 0x20, 0x20);
        assert_eq!(pic.acknowledge(), Some(0x0B));
    }

    #[test]
    fn rotation_auto_eoi_and_poll() {
        let mut pic = Pic::new();
        out(&mut pic, 0x20, 0xC3); // set priority: IR3 lowest, IR4 highest
        pic.raise(2);
        pic.raise(5);
        assert_eq!(pic.pending_vector(), Some(0x0D));

        let mut pic = Pic::new();
        for (port, v) in [(0x20, 0x13), (0x21, 0x08), (0x21, 0x03)] {
            out(&mut pic, port, v); // single, ICW4 with auto-EOI
        }
        pic.raise(5);
        pic.raise(6);
        assert_eq!(pic.acknowledge(), Some(0x0D));
        assert_eq!(pic.acknowledge(), Some(0x0E), "no ISR left behind");

        let mut pic = Pic::new();
        pic.raise(3);
        out(&mut pic, 0x20, 0x0C); // poll
        assert_eq!(inp(&mut pic, 0x20), 0x83);
        assert_eq!(inp(&mut pic, 0x20), 0x00, "IRR read after poll");
        out(&mut pic, 0x20, 0x0B);
        assert_eq!(inp(&mut pic, 0x20), 0x08);
    }
}
