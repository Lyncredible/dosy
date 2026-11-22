//! 8254 programmable interval timer (40h–43h). Counters are computed lazily
//! from emulated time; channel 0 schedules IRQ0 edges, channel 2's gate and
//! output belong to port 61h / the PC speaker.

use super::{DeviceId, IoCtx, IoDevice};

/// Input clock in Hz.
pub const PIT_HZ: u64 = 1_193_182;

/// Scheduler event kind: channel 0 output rising edge.
const EV_CH0: u32 = 0;

/// PIT ticks elapsed at `ns`.
pub fn ns_to_ticks(ns: u64) -> u64 {
    (ns as u128 * PIT_HZ as u128 / 1_000_000_000) as u64
}

/// First ns at which `tick` has elapsed.
pub fn ticks_to_ns(tick: u64) -> u64 {
    (tick as u128 * 1_000_000_000).div_ceil(PIT_HZ as u128) as u64
}

#[derive(Clone, Debug)]
struct Channel {
    mode: u8,
    /// 1 = lobyte, 2 = hibyte, 3 = lobyte then hibyte.
    access: u8,
    bcd: bool,
    /// Count in ticks, 1..=65536 (a written 0 means 65536).
    reload: u64,
    /// Tick at which the current count started.
    start: u64,
    counting: bool,
    /// Gate low in a gated mode: elapsed ticks frozen here.
    paused: Option<u64>,
    /// Modes 2/3: new count taking effect at the given period boundary.
    pending: Option<(u64, u64)>,
    gate: bool,
    /// Control word written, count not yet loaded.
    null: bool,
    write_lo: Option<u8>,
    read_hi: bool,
    latch: Option<u16>,
    status: Option<u8>,
}

impl Channel {
    fn new() -> Self {
        Self {
            mode: 0,
            access: 3,
            bcd: false,
            reload: 0x10000,
            start: 0,
            counting: false,
            paused: None,
            pending: None,
            gate: true,
            null: true,
            write_lo: None,
            read_hi: false,
            latch: None,
            status: None,
        }
    }

    fn periodic(&self) -> bool {
        matches!(self.mode, 2 | 3)
    }

    fn apply_pending(&mut self, now: u64) {
        if let Some((at, n)) = self.pending {
            if now >= at && self.paused.is_none() {
                self.start = at;
                self.reload = n;
                self.pending = None;
            }
        }
    }

    fn elapsed(&self, now: u64) -> u64 {
        self.paused.unwrap_or(now.saturating_sub(self.start))
    }

    fn count(&self, now: u64) -> u16 {
        if !self.counting {
            return self.reload as u16;
        }
        let (n, e) = (self.reload, self.elapsed(now));
        let c = match self.mode {
            2 => n - e % n,
            3 => {
                let first = n.div_ceil(2);
                let phase = e % n;
                let local = if phase < first { phase } else { phase - first };
                (n - 2 * local).max(2) & !1
            }
            _ => (n.wrapping_sub(e)) & 0xFFFF,
        };
        c as u16
    }

    fn out(&self, now: u64) -> bool {
        if !self.counting {
            return self.mode != 0 && self.mode != 1;
        }
        let (n, e) = (self.reload, self.elapsed(now));
        match self.mode {
            0 => e >= n,
            1 => e >= n,
            2 => self.paused.is_some() || e % n != n - 1,
            3 => self.paused.is_some() || e % n < n.div_ceil(2),
            _ => e != n, // 4, 5: one-tick low strobe at terminal count
        }
    }

    /// Next tick strictly after `now` at which OUT rises, if any.
    fn next_rise(&self, now: u64) -> Option<u64> {
        if !self.counting || self.paused.is_some() {
            return None;
        }
        let (n, e) = (self.reload, now.saturating_sub(self.start));
        match self.mode {
            2 | 3 => Some(self.start + (e / n + 1) * n),
            0 | 1 => (e < n).then_some(self.start + n),
            _ => (e <= n).then_some(self.start + n + 1),
        }
    }

    fn load(&mut self, n: u64, now: u64) {
        self.null = false;
        if self.periodic() && self.counting && self.paused.is_none() {
            let e = now - self.start;
            self.pending = Some((self.start + (e / self.reload + 1) * self.reload, n));
            return;
        }
        self.reload = n;
        self.start = now;
        self.pending = None;
        // Modes 1 and 5 wait for a gate trigger.
        self.counting = !matches!(self.mode, 1 | 5);
        self.paused = (!self.gate && self.counting).then_some(0);
    }

    fn control(&mut self, val: u8) {
        let mode = (val >> 1) & 7;
        *self = Self {
            mode: if mode >= 6 { mode - 4 } else { mode },
            access: (val >> 4) & 3,
            bcd: val & 1 != 0,
            gate: self.gate,
            reload: self.reload,
            ..Self::new()
        };
        if self.bcd {
            log::warn!("8254: BCD counting not supported");
        }
    }

    fn latch_count(&mut self, now: u64) {
        if self.latch.is_none() {
            self.latch = Some(self.count(now));
            self.read_hi = false;
        }
    }

    fn status_byte(&self, now: u64) -> u8 {
        (self.out(now) as u8) << 7
            | (self.null as u8) << 6
            | self.access << 4
            | self.mode << 1
            | self.bcd as u8
    }

    fn read(&mut self, now: u64) -> u8 {
        if let Some(s) = self.status.take() {
            return s;
        }
        let v = self.latch.unwrap_or_else(|| self.count(now));
        let [lo, hi] = v.to_le_bytes();
        match self.access {
            1 => {
                self.latch = None;
                lo
            }
            2 => {
                self.latch = None;
                hi
            }
            _ => {
                self.read_hi = !self.read_hi;
                if self.read_hi {
                    lo
                } else {
                    self.latch = None;
                    hi
                }
            }
        }
    }

    /// Returns true if a count was loaded.
    fn write(&mut self, val: u8, now: u64) -> bool {
        let v = match self.access {
            1 => val as u64,
            2 => (val as u64) << 8,
            _ => match self.write_lo.take() {
                None => {
                    self.write_lo = Some(val);
                    if self.mode == 0 {
                        self.counting = false; // first byte stops mode 0
                    }
                    return false;
                }
                Some(lo) => lo as u64 | (val as u64) << 8,
            },
        };
        self.load(if v == 0 { 0x10000 } else { v }, now);
        true
    }

    fn set_gate(&mut self, high: bool, now: u64) {
        if high == self.gate {
            return;
        }
        self.gate = high;
        if self.null {
            return;
        }
        if high {
            match self.mode {
                1 | 5 | 2 | 3 => {
                    self.counting = true;
                    self.start = now;
                    self.paused = None;
                    if let Some((_, n)) = self.pending.take() {
                        self.reload = n;
                    }
                }
                _ => {
                    if let Some(e) = self.paused.take() {
                        self.start = now - e;
                    }
                }
            }
        } else if self.counting && !matches!(self.mode, 1 | 5) {
            self.apply_pending(now);
            self.paused = Some(self.elapsed(now));
        }
    }
}

#[derive(Clone, Debug)]
pub struct Pit {
    ch: [Channel; 3],
}

impl Default for Pit {
    fn default() -> Self {
        Self::new()
    }
}

impl Pit {
    /// Power-on: all channels unprogrammed; ch2 gate low (port 61h = 0).
    /// The BIOS programs ch0 (mode 3, count 0).
    pub fn new() -> Self {
        let mut ch2 = Channel::new();
        ch2.gate = false;
        Self {
            ch: [Channel::new(), Channel::new(), ch2],
        }
    }

    /// Channel 2 output (speaker / port 61h bit 5).
    pub fn ch2_out(&mut self, now_ns: u64) -> bool {
        let now = ns_to_ticks(now_ns);
        self.ch[2].apply_pending(now);
        self.ch[2].out(now)
    }

    pub fn ch2_gate(&self) -> bool {
        self.ch[2].gate
    }

    /// Channel 2 count in ticks (for the speaker's frequency).
    pub fn ch2_reload(&self) -> u64 {
        self.ch[2].reload
    }

    /// Port 61h bit 0.
    pub fn set_ch2_gate(&mut self, high: bool, now_ns: u64) {
        self.ch[2].set_gate(high, ns_to_ticks(now_ns));
    }

    fn reschedule_ch0(&mut self, ctx: &mut IoCtx) {
        ctx.sched.cancel(DeviceId::PIT, EV_CH0);
        if let Some(t) = self.ch[0].next_rise(ns_to_ticks(ctx.now)) {
            ctx.sched.schedule(ticks_to_ns(t), DeviceId::PIT, EV_CH0);
        }
    }

    fn read_back(&mut self, val: u8, now: u64) {
        for (i, c) in self.ch.iter_mut().enumerate() {
            if val & (2 << i) == 0 {
                continue;
            }
            c.apply_pending(now);
            if val & 0x20 == 0 {
                c.latch_count(now);
            }
            if val & 0x10 == 0 && c.status.is_none() {
                c.status = Some(c.status_byte(now));
            }
        }
    }
}

impl IoDevice for Pit {
    fn read(&mut self, port: u16, ctx: &mut IoCtx) -> u8 {
        let now = ns_to_ticks(ctx.now);
        match port & 3 {
            3 => 0xFF,
            i => {
                let c = &mut self.ch[i as usize];
                c.apply_pending(now);
                c.read(now)
            }
        }
    }

    fn write(&mut self, port: u16, val: u8, ctx: &mut IoCtx) {
        let now = ns_to_ticks(ctx.now);
        let sc = match port & 3 {
            3 => {
                let sc = (val >> 6) as usize;
                if sc == 3 {
                    self.read_back(val, now);
                    return;
                }
                let c = &mut self.ch[sc];
                c.apply_pending(now);
                if (val >> 4) & 3 == 0 {
                    c.latch_count(now);
                    return;
                }
                c.control(val);
                sc
            }
            i => {
                let c = &mut self.ch[i as usize];
                c.apply_pending(now);
                c.write(val, now);
                i as usize
            }
        };
        if sc == 0 {
            self.reschedule_ch0(ctx);
        }
    }

    fn event(&mut self, kind: u32, ctx: &mut IoCtx) {
        if kind == EV_CH0 {
            self.ch[0].apply_pending(ns_to_ticks(ctx.now));
            ctx.irq.pulse(0);
            self.reschedule_ch0(ctx);
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::bus::Bus;
    use crate::hw::Hardware;

    use super::*;

    fn at_tick(hw: &mut Hardware, t: u64) {
        hw.advance_to(ticks_to_ns(t));
    }

    fn program(hw: &mut Hardware, ch: u16, mode: u8, count: u16) {
        hw.io_out8(0x43, (ch as u8) << 6 | 0x30 | mode << 1);
        hw.io_out8(0x40 + ch, count as u8);
        hw.io_out8(0x40 + ch, (count >> 8) as u8);
    }

    fn latch(hw: &mut Hardware, ch: u16) -> u16 {
        hw.io_out8(0x43, (ch as u8) << 6);
        u16::from_le_bytes([hw.io_in8(0x40 + ch), hw.io_in8(0x40 + ch)])
    }

    /// Run events until `end_ns`, returning tick times of IRQ0 deliveries.
    fn irq0_ticks(hw: &mut Hardware, end_ns: u64) -> Vec<u64> {
        let mut out = vec![];
        while let Some(t) = hw.sched.next_due().filter(|&t| t <= end_ns) {
            hw.advance_to(t);
            hw.fire_due();
            if hw.pic.acknowledge() == Some(0x08) {
                out.push(ns_to_ticks(hw.now()));
                hw.io_out8(0x20, 0x20);
            }
        }
        out
    }

    #[test]
    fn tick_conversion_round_trips() {
        assert_eq!(ticks_to_ns(0), 0);
        for t in [1, 65536, PIT_HZ, 10 * PIT_HZ + 7] {
            assert_eq!(ns_to_ticks(ticks_to_ns(t)), t);
            assert_eq!(ns_to_ticks(ticks_to_ns(t) - 1), t - 1);
        }
    }

    #[test]
    fn default_mode3_is_18_2_hz() {
        for mode in [2, 3] {
            let mut hw = Hardware::new();
            program(&mut hw, 0, mode, 0);
            let ticks = irq0_ticks(&mut hw, 1_000_000_000);
            assert_eq!(ticks.len(), 18, "mode {mode}");
            assert!(ticks.windows(2).all(|w| w[1] - w[0] == 65536));
            // 10 s → 182.065 periods.
            let more = irq0_ticks(&mut hw, 10_000_000_000);
            assert_eq!(ticks.len() + more.len(), 182);
            let hz = PIT_HZ as f64 / 65536.0;
            assert!((hz - 18.2065).abs() < 1e-4);
        }
    }

    #[test]
    fn counter_readback_mid_period() {
        let mut hw = Hardware::new();
        program(&mut hw, 0, 2, 1000);
        at_tick(&mut hw, 250);
        assert_eq!(latch(&mut hw, 0), 750);
        at_tick(&mut hw, 1999);
        assert_eq!(latch(&mut hw, 0), 1);
        hw.io_out8(0x61, 0x01); // ch2 gate high
        program(&mut hw, 2, 3, 1000);
        at_tick(&mut hw, 1999 + 250);
        assert_eq!(latch(&mut hw, 2), 500);
        assert!(hw.pit.ch2_out(hw.now()));
        at_tick(&mut hw, 1999 + 750);
        assert!(!hw.pit.ch2_out(hw.now()));
        // Latch holds its value while the counter runs on.
        hw.io_out8(0x43, 0x00);
        at_tick(&mut hw, 3000);
        assert_eq!(u16::from_le_bytes([hw.io_in8(0x40), hw.io_in8(0x40)]), 251);
        // Lobyte-only access.
        hw.io_out8(0x43, 0x94); // ch2, lobyte, mode 2
        hw.io_out8(0x42, 100);
        at_tick(&mut hw, 3040);
        hw.io_out8(0x43, 0x80);
        assert_eq!(hw.io_in8(0x42), 60);
    }

    #[test]
    fn reprogramming_mid_count_takes_effect_at_reload() {
        let mut hw = Hardware::new();
        program(&mut hw, 0, 2, 1000);
        at_tick(&mut hw, 250);
        hw.io_out8(0x40, 0xF4); // 500 = 0x01F4
        hw.io_out8(0x40, 0x01);
        at_tick(&mut hw, 600);
        assert_eq!(latch(&mut hw, 0), 400, "old count still running");
        let ticks = irq0_ticks(&mut hw, ticks_to_ns(2100));
        assert_eq!(ticks, [1000, 1500, 2000]);
        at_tick(&mut hw, 2100);
        assert_eq!(latch(&mut hw, 0), 400);
        // A new control word restarts immediately.
        program(&mut hw, 0, 2, 300);
        assert_eq!(irq0_ticks(&mut hw, ticks_to_ns(2700)), [2400, 2700]);
    }

    #[test]
    fn mode0_one_shot_irq() {
        let mut hw = Hardware::new();
        at_tick(&mut hw, 10);
        program(&mut hw, 0, 0, 100);
        at_tick(&mut hw, 60);
        assert_eq!(latch(&mut hw, 0), 50);
        assert_eq!(irq0_ticks(&mut hw, u64::MAX), [110]);
        assert!(hw.sched.is_empty());
        at_tick(&mut hw, 120);
        assert_eq!(
            latch(&mut hw, 0),
            0xFFF6,
            "counter wraps after terminal count"
        );
        // Read-back status: OUT high, mode 0, lobyte/hibyte.
        hw.io_out8(0x43, 0xE2);
        assert_eq!(hw.io_in8(0x40), 0x80 | 0x30);
    }

    #[test]
    fn ch2_gate_starts_low_and_restarts_mode2() {
        let mut hw = Hardware::new();
        assert!(!hw.pit.ch2_gate());
        program(&mut hw, 2, 2, 100);
        at_tick(&mut hw, 50);
        assert_eq!(latch(&mut hw, 2), 100, "gate low holds the count");
        hw.io_out8(0x61, 0x01);
        at_tick(&mut hw, 80);
        assert_eq!(latch(&mut hw, 2), 70);
        // Refresh toggle (bit 4) flips every ~15 us.
        let a = hw.io_in8(0x61) & 0x10;
        hw.advance_to(hw.now() + 15_085);
        assert_ne!(hw.io_in8(0x61) & 0x10, a);
    }

    #[test]
    fn ch2_gate_controls_counting() {
        let mut hw = Hardware::new();
        hw.io_out8(0x61, 0x01);
        program(&mut hw, 2, 0, 100);
        at_tick(&mut hw, 30);
        hw.io_out8(0x61, 0x00); // gate low: pause
        at_tick(&mut hw, 500);
        assert_eq!(latch(&mut hw, 2), 70);
        assert_eq!(hw.io_in8(0x61) & 0x20, 0);
        hw.io_out8(0x61, 0x01);
        at_tick(&mut hw, 570);
        assert_eq!(latch(&mut hw, 2), 0);
        assert_eq!(
            hw.io_in8(0x61) & 0x23,
            0x21,
            "OUT2 high, gate bit read back"
        );
    }
}
