//! 8042 keyboard controller (60h data, 64h status/command) with a set-1
//! scancode queue fed by the host.

use std::collections::VecDeque;

use super::{DeviceId, IoCtx, IoDevice};

/// Delay before the next byte reaches the output buffer after a read, so
/// handlers that read 60h more than once (or chain) see the same byte.
pub const BYTE_DELAY_NS: u64 = 1_000_000;
const EV_REFILL: u32 = 0;

// Status register (64h read).
const OBF: u8 = 0x01;
const SYS: u8 = 0x04;
const CMD: u8 = 0x08;
const UNLOCKED: u8 = 0x10;
// Command byte.
const CCB_IRQ1: u8 = 0x01;
const CCB_SYS: u8 = 0x04;
const CCB_KBD_OFF: u8 = 0x10;
const CCB_XLAT: u8 = 0x40;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Pending {
    /// Controller command waiting for its data byte on 60h.
    Ctrl(u8),
    /// Keyboard command (EDh, F3h) waiting for its argument.
    Kbd(u8),
}

#[derive(Clone, Debug)]
pub struct Kbc {
    /// Output buffer contents; stays readable after OBF clears.
    out: u8,
    full: bool,
    /// Controller/keyboard replies, delivered before scancodes.
    resp: VecDeque<u8>,
    keys: VecDeque<u8>,
    ccb: u8,
    status_cmd: bool,
    pending: Option<Pending>,
    /// Keyboard scanning enabled (F4h / F5h).
    scanning: bool,
    /// Output port (D0h/D1h); bit 1 = A20.
    outport: u8,
    a20_change: Option<bool>,
    refill_scheduled: bool,
    leds: u8,
}

impl Default for Kbc {
    fn default() -> Self {
        Self::new()
    }
}

impl Kbc {
    pub fn new() -> Self {
        Self {
            out: 0,
            full: false,
            resp: VecDeque::new(),
            keys: VecDeque::new(),
            ccb: CCB_IRQ1 | CCB_SYS | CCB_XLAT,
            status_cmd: false,
            pending: None,
            scanning: true,
            outport: 0x01,
            a20_change: None,
            refill_scheduled: false,
            leds: 0,
        }
    }

    /// Queue set-1 bytes from the host keyboard.
    /// Dropped while the keyboard has scanning disabled (F5h).
    pub fn push_scancodes(&mut self, codes: &[u8], ctx: &mut IoCtx) {
        if !self.scanning {
            return;
        }
        self.keys.extend(codes);
        self.refill(ctx);
    }

    /// An A20 change requested through the output port since the last call.
    pub fn take_a20_change(&mut self) -> Option<bool> {
        self.a20_change.take()
    }

    /// Keep the output port's A20 bit in step with the real gate.
    pub fn sync_a20(&mut self, on: bool) {
        self.outport = (self.outport & !0x02) | (on as u8) << 1;
    }

    /// Keyboard LED state from the last EDh command (bit 0 scroll, 1 num, 2 caps).
    pub fn leds(&self) -> u8 {
        self.leds
    }

    fn refill(&mut self, ctx: &mut IoCtx) {
        if self.full || self.refill_scheduled {
            return;
        }
        let next = match self.resp.pop_front() {
            Some(b) => b,
            None if self.scanning && self.ccb & CCB_KBD_OFF == 0 => match self.keys.pop_front() {
                Some(b) => b,
                None => return,
            },
            None => return,
        };
        self.out = next;
        self.full = true;
        if self.ccb & CCB_IRQ1 != 0 {
            ctx.irq.pulse(1);
        }
    }

    fn reply(&mut self, bytes: &[u8], ctx: &mut IoCtx) {
        self.resp.extend(bytes);
        self.refill(ctx);
    }

    fn command(&mut self, cmd: u8, ctx: &mut IoCtx) {
        match cmd {
            0x20 => self.reply(&[self.ccb], ctx),
            0x60 | 0xD1 | 0xD2 => self.pending = Some(Pending::Ctrl(cmd)),
            0xA7 | 0xA8 => {} // aux port disable/enable: no mouse yet
            0xA9 | 0xAB => self.reply(&[0x00], ctx),
            0xAA => {
                self.ccb |= CCB_SYS;
                self.reply(&[0x55], ctx);
            }
            0xAD => self.ccb |= CCB_KBD_OFF,
            0xAE => {
                self.ccb &= !CCB_KBD_OFF;
                self.refill(ctx);
            }
            0xC0 => self.reply(&[0xBF], ctx),
            0xD0 => self.reply(&[self.outport], ctx),
            0xDD => self.set_outport(self.outport & !0x02),
            0xDF => self.set_outport(self.outport | 0x02),
            0xE0 => self.reply(&[0x00], ctx),
            0xF0..=0xFF => {
                if cmd & 1 == 0 {
                    log::warn!("8042: CPU reset requested (ignored)");
                }
            }
            _ => log::debug!("8042: unknown command {cmd:#04x}"),
        }
    }

    fn set_outport(&mut self, v: u8) {
        if (v ^ self.outport) & 0x02 != 0 {
            self.a20_change = Some(v & 0x02 != 0);
        }
        self.outport = v;
        if v & 0x01 == 0 {
            log::warn!("8042: CPU reset via output port (ignored)");
        }
    }

    fn data(&mut self, val: u8, ctx: &mut IoCtx) {
        match self.pending.take() {
            Some(Pending::Ctrl(0x60)) => {
                self.ccb = val;
                self.refill(ctx);
            }
            Some(Pending::Ctrl(0xD1)) => self.set_outport(val),
            Some(Pending::Ctrl(_)) => {
                // D2h: as if the keyboard sent it.
                self.resp.push_back(val);
                self.refill(ctx);
            }
            Some(Pending::Kbd(0xED)) => {
                self.leds = val & 7;
                self.reply(&[0xFA], ctx);
            }
            Some(Pending::Kbd(_)) => self.reply(&[0xFA], ctx),
            None => self.keyboard_command(val, ctx),
        }
    }

    fn keyboard_command(&mut self, val: u8, ctx: &mut IoCtx) {
        // Any byte to the keyboard re-enables the interface.
        self.ccb &= !CCB_KBD_OFF;
        match val {
            0xED | 0xF3 => {
                self.pending = Some(Pending::Kbd(val));
                self.reply(&[0xFA], ctx);
            }
            0xEE => self.reply(&[0xEE], ctx),
            0xF2 => self.reply(&[0xFA, 0xAB, 0x41], ctx),
            0xF4 => {
                self.scanning = true;
                self.reply(&[0xFA], ctx);
            }
            0xF5 => {
                self.scanning = false;
                self.keys.clear();
                self.reply(&[0xFA], ctx);
            }
            0xFF => {
                self.keys.clear();
                self.scanning = true;
                self.reply(&[0xFA, 0xAA], ctx);
            }
            _ => self.reply(&[0xFA], ctx),
        }
    }
}

impl IoDevice for Kbc {
    fn read(&mut self, port: u16, ctx: &mut IoCtx) -> u8 {
        if port == 0x64 {
            let sys = if self.ccb & CCB_SYS != 0 { SYS } else { 0 };
            let cmd = if self.status_cmd { CMD } else { 0 };
            return (self.full as u8 * OBF) | sys | cmd | UNLOCKED;
        }
        if self.full {
            self.full = false;
            if !self.resp.is_empty() || !self.keys.is_empty() {
                self.refill_scheduled = true;
                ctx.sched
                    .schedule(ctx.now + BYTE_DELAY_NS, DeviceId::KBD, EV_REFILL);
            }
        }
        self.out
    }

    fn write(&mut self, port: u16, val: u8, ctx: &mut IoCtx) {
        self.status_cmd = port == 0x64;
        if port == 0x64 {
            self.pending = None;
            self.command(val, ctx);
        } else {
            self.data(val, ctx);
        }
    }

    fn event(&mut self, kind: u32, ctx: &mut IoCtx) {
        if kind == EV_REFILL {
            self.refill_scheduled = false;
            self.refill(ctx);
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::bus::Bus;
    use crate::hw::Hardware;

    use super::*;

    /// Deliver pending IRQ1s: returns bytes read by a minimal "ISR".
    fn isr_reads(hw: &mut Hardware, until_ns: u64) -> Vec<u8> {
        let mut got = vec![];
        loop {
            if hw.pic.pending_vector() == Some(0x09) {
                hw.pic.acknowledge();
                got.push(hw.io_in8(0x60));
                hw.io_out8(0x20, 0x20);
                continue;
            }
            match hw.sched.next_due() {
                Some(t) if t <= until_ns => {
                    hw.advance_to(t);
                    hw.fire_due();
                }
                _ => return got,
            }
        }
    }

    #[test]
    fn scancodes_arrive_in_order_one_irq_each() {
        let mut hw = Hardware::new();
        hw.push_scancodes(&[0x1E, 0x9E, 0xE0, 0x48]);
        assert_eq!(hw.io_in8(0x64) & OBF, OBF);
        assert_eq!(hw.pic.pending_vector(), Some(0x09));
        assert_eq!(isr_reads(&mut hw, 10_000_000), [0x1E, 0x9E, 0xE0, 0x48]);
        assert_eq!(hw.io_in8(0x64) & OBF, 0);
    }

    #[test]
    fn next_byte_waits_after_a_read() {
        let mut hw = Hardware::new();
        hw.push_scancodes(&[0x10, 0x90]);
        assert_eq!(hw.io_in8(0x60), 0x10);
        // A chained handler re-reads the same byte; no new IRQ yet.
        assert_eq!(hw.io_in8(0x60), 0x10);
        hw.pic.acknowledge();
        hw.io_out8(0x20, 0x20);
        assert_eq!(hw.pic.pending_vector(), None);
        hw.advance_to(BYTE_DELAY_NS);
        hw.fire_due();
        assert_eq!(hw.pic.pending_vector(), Some(0x09));
        assert_eq!(hw.io_in8(0x60), 0x90);
    }

    #[test]
    fn controller_commands() {
        let mut hw = Hardware::new();
        hw.io_out8(0x64, 0xAA);
        assert_eq!(hw.io_in8(0x60), 0x55);
        hw.io_out8(0x64, 0x20);
        assert_eq!(hw.io_in8(0x60), CCB_IRQ1 | CCB_SYS | CCB_XLAT);
        assert_eq!(hw.io_in8(0x64) & (SYS | CMD), SYS | CMD);
        // AD disables the keyboard: keys wait until AE.
        hw.io_out8(0x64, 0xAD);
        hw.push_scancodes(&[0x1E]);
        assert_eq!(hw.io_in8(0x64) & OBF, 0);
        hw.io_out8(0x64, 0xAE);
        assert_eq!(hw.io_in8(0x60), 0x1E);
        // Command byte without IRQ1: data arrives, no interrupt.
        hw.io_out8(0x64, 0x60);
        hw.io_out8(0x60, CCB_SYS);
        hw.pic.acknowledge();
        hw.io_out8(0x20, 0x20);
        hw.advance_to(10 * BYTE_DELAY_NS);
        hw.fire_due();
        hw.push_scancodes(&[0x30]);
        assert_eq!(hw.pic.pending_vector(), None);
        assert_eq!(hw.io_in8(0x60), 0x30);
    }

    #[test]
    fn keyboard_commands_ack() {
        let mut hw = Hardware::new();
        hw.io_out8(0x60, 0xED);
        let mut t = 0;
        let mut read = |hw: &mut Hardware| {
            t += BYTE_DELAY_NS;
            hw.advance_to(t);
            hw.fire_due();
            hw.io_in8(0x60)
        };
        assert_eq!(read(&mut hw), 0xFA);
        hw.io_out8(0x60, 0x04);
        assert_eq!(read(&mut hw), 0xFA);
        assert_eq!(hw.kbd.leds(), 0x04);
        hw.io_out8(0x60, 0xF2);
        assert_eq!(
            [read(&mut hw), read(&mut hw), read(&mut hw)],
            [0xFA, 0xAB, 0x41]
        );
        hw.io_out8(0x60, 0xF5);
        assert_eq!(read(&mut hw), 0xFA);
        hw.push_scancodes(&[0x1E]);
        assert_eq!(hw.io_in8(0x64) & OBF, 0, "scanning disabled");
        hw.io_out8(0x60, 0xF4);
        assert_eq!(read(&mut hw), 0xFA);
        hw.push_scancodes(&[0x1F]);
        assert_eq!(read(&mut hw), 0x1F);
    }

    #[test]
    fn output_port_a20_toggles_wrap() {
        let mut hw = Hardware::new();
        hw.write8(0x10_0000, 0x11);
        assert_eq!(hw.read8(0), 0x11, "A20 off wraps");
        hw.io_out8(0x64, 0xD1);
        hw.io_out8(0x60, 0xDF);
        assert!(hw.mem.a20());
        hw.write8(0x10_0000, 0x22);
        assert_eq!(hw.read8(0), 0x11);
        hw.io_out8(0x64, 0xD0);
        assert_eq!(hw.io_in8(0x60) & 0x02, 0x02);
        hw.io_out8(0x64, 0xDD);
        assert!(!hw.mem.a20());
        // Port 92h and the 8042 share the gate.
        hw.io_out8(0x92, 0x02);
        hw.advance_to(BYTE_DELAY_NS);
        hw.fire_due();
        hw.io_out8(0x64, 0xD0);
        assert_eq!(hw.io_in8(0x60) & 0x02, 0x02);
    }
}
