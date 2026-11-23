//! MC146818 CMOS RAM and RTC (70h index, 71h data). The clock is an
//! injected base time (Unix seconds, local) advanced by emulated `now`.

use super::{IoCtx, IoDevice};

const REG_A: usize = 0x0A;
const REG_B: usize = 0x0B;
const REG_C: usize = 0x0C;
const REG_D: usize = 0x0D;
/// Update-in-progress window at the end of each second.
const UIP_NS: u64 = 244_000;
const B_BINARY: u8 = 0x04;
const B_24H: u8 = 0x02;

/// Civil date from days since 1970-01-01 (proleptic Gregorian).
fn civil(days: i64) -> (i64, u8, u8) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u8;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u8;
    let y = yoe + era * 400 + (m <= 2) as i64;
    (y, m, d)
}

/// Days since 1970-01-01 for a civil date.
fn days_from_civil(y: i64, m: u8, d: u8) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let m = m as i64;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Time {
    year: i64,
    month: u8,
    day: u8,
    hour: u8,
    min: u8,
    sec: u8,
}

impl Time {
    fn from_unix(t: i64) -> Self {
        let (days, s) = (t.div_euclid(86_400), t.rem_euclid(86_400));
        let (year, month, day) = civil(days);
        Self {
            year,
            month,
            day,
            hour: (s / 3600) as u8,
            min: (s / 60 % 60) as u8,
            sec: (s % 60) as u8,
        }
    }

    fn to_unix(self) -> i64 {
        days_from_civil(self.year, self.month, self.day) * 86_400
            + self.hour as i64 * 3600
            + self.min as i64 * 60
            + self.sec as i64
    }
}

#[derive(Clone, Debug)]
pub struct Cmos {
    ram: [u8; 128],
    index: usize,
    /// RTC seconds at `now = 0`.
    base: i64,
}

impl Default for Cmos {
    fn default() -> Self {
        Self::new()
    }
}

impl Cmos {
    /// 1980-01-01 00:00:00, 640 KiB base memory, no extended memory.
    pub fn new() -> Self {
        let mut ram = [0u8; 128];
        ram[REG_A] = 0x26; // 32.768 kHz, 1024 Hz periodic rate
        ram[REG_B] = B_24H;
        ram[REG_D] = 0x80; // RAM valid
        ram[0x10] = 0x00; // no floppies
        ram[0x14] = 0x00; // equipment: EGA/VGA, no FPU, no floppies
        ram[0x15] = 0x80; // base memory 640 KiB
        ram[0x16] = 0x02;
        let mut c = Self {
            ram,
            index: 0,
            base: 315_532_800,
        };
        c.update_checksum();
        c
    }

    /// Set the clock so it reads `unix_secs` at emulated time `now_ns`.
    pub fn set_time(&mut self, unix_secs: i64, now_ns: u64) {
        self.base = unix_secs - (now_ns / 1_000_000_000) as i64;
    }

    /// Current RTC time in Unix seconds.
    pub fn time(&self, now_ns: u64) -> i64 {
        self.base + (now_ns / 1_000_000_000) as i64
    }

    fn update_checksum(&mut self) {
        let sum: u16 = self.ram[0x10..=0x2D].iter().map(|&b| b as u16).sum();
        self.ram[0x2E..=0x2F].copy_from_slice(&sum.to_be_bytes());
    }

    fn encode(&self, v: u8) -> u8 {
        if self.ram[REG_B] & B_BINARY != 0 {
            v
        } else {
            ((v / 10) << 4) | (v % 10)
        }
    }

    fn decode(&self, v: u8) -> u8 {
        if self.ram[REG_B] & B_BINARY != 0 {
            v
        } else {
            (v >> 4) * 10 + (v & 0xF)
        }
    }

    fn encode_hour(&self, h: u8) -> u8 {
        if self.ram[REG_B] & B_24H != 0 {
            return self.encode(h);
        }
        let h12 = match h % 12 {
            0 => 12,
            x => x,
        };
        self.encode(h12) | if h >= 12 { 0x80 } else { 0 }
    }

    fn decode_hour(&self, v: u8) -> u8 {
        if self.ram[REG_B] & B_24H != 0 {
            return self.decode(v);
        }
        let h = self.decode(v & 0x7F) % 12;
        if v & 0x80 != 0 { h + 12 } else { h }
    }

    fn read_reg(&mut self, reg: usize, now: u64) -> u8 {
        let t = Time::from_unix(self.time(now));
        match reg {
            0x00 => self.encode(t.sec),
            0x02 => self.encode(t.min),
            0x04 => self.encode_hour(t.hour),
            0x06 => {
                let dow = (days_from_civil(t.year, t.month, t.day) + 4).rem_euclid(7);
                self.encode(dow as u8 + 1) // 1 = Sunday
            }
            0x07 => self.encode(t.day),
            0x08 => self.encode(t.month),
            0x09 => self.encode(t.year.rem_euclid(100) as u8),
            0x32 => self.encode(t.year.div_euclid(100) as u8),
            REG_A => {
                let uip = now % 1_000_000_000 >= 1_000_000_000 - UIP_NS;
                self.ram[REG_A] & 0x7F | (uip as u8) << 7
            }
            REG_C => std::mem::take(&mut self.ram[REG_C]),
            _ => self.ram[reg],
        }
    }

    fn write_reg(&mut self, reg: usize, val: u8, now: u64) {
        let mut t = Time::from_unix(self.time(now));
        match reg {
            0x00 => t.sec = self.decode(val).min(59),
            0x02 => t.min = self.decode(val).min(59),
            0x04 => t.hour = self.decode_hour(val).min(23),
            0x07 => t.day = self.decode(val).clamp(1, 31),
            0x08 => t.month = self.decode(val).clamp(1, 12),
            0x09 => t.year = t.year.div_euclid(100) * 100 + self.decode(val) as i64 % 100,
            0x32 => t.year = self.decode(val) as i64 * 100 + t.year.rem_euclid(100),
            0x01 | 0x03 | 0x05 | 0x06 => return, // alarms, day of week: ignored
            REG_A => {
                self.ram[REG_A] = val & 0x7F;
                return;
            }
            REG_C | REG_D => return, // read-only
            _ => {
                self.ram[reg] = val;
                if (0x10..=0x2D).contains(&reg) {
                    self.update_checksum();
                }
                return;
            }
        }
        self.set_time(t.to_unix(), now);
    }
}

impl IoDevice for Cmos {
    fn read(&mut self, port: u16, ctx: &mut IoCtx) -> u8 {
        if port & 1 == 0 {
            return 0xFF;
        }
        self.read_reg(self.index, ctx.now)
    }

    fn write(&mut self, port: u16, val: u8, ctx: &mut IoCtx) {
        if port & 1 == 0 {
            self.index = (val & 0x7F) as usize; // bit 7: NMI mask, ignored
        } else {
            self.write_reg(self.index, val, ctx.now);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bus::Bus;
    use crate::hw::Hardware;

    fn reg(hw: &mut Hardware, r: u8) -> u8 {
        hw.io_out8(0x70, r);
        hw.io_in8(0x71)
    }

    #[test]
    fn calendar_round_trips() {
        for t in [0i64, 315_532_800, 951_782_400, 1_700_000_000, 4_102_444_799] {
            assert_eq!(Time::from_unix(t).to_unix(), t);
        }
        let t = Time::from_unix(951_782_400); // 2000-02-29
        assert_eq!((t.year, t.month, t.day), (2000, 2, 29));
    }

    #[test]
    fn time_advances_with_emulated_time() {
        let mut hw = Hardware::new();
        // 1999-12-31 23:59:58 (a Friday).
        hw.cmos.set_time(946_684_798, 0);
        let regs = |hw: &mut Hardware| [0, 2, 4, 6, 7, 8, 9, 0x32].map(|r| reg(hw, r));
        assert_eq!(
            regs(&mut hw),
            [0x58, 0x59, 0x23, 0x06, 0x31, 0x12, 0x99, 0x19]
        );
        hw.advance_to(2_500_000_000);
        // 2000-01-01 00:00:00.5, a Saturday.
        assert_eq!(
            regs(&mut hw),
            [0x00, 0x00, 0x00, 0x07, 0x01, 0x01, 0x00, 0x20]
        );
    }

    #[test]
    fn status_registers_and_uip() {
        let mut hw = Hardware::new();
        assert_eq!(reg(&mut hw, 0x0A), 0x26);
        assert_eq!(reg(&mut hw, 0x0B), 0x02);
        assert_eq!(reg(&mut hw, 0x0D), 0x80);
        hw.advance_to(999_900_000);
        assert_eq!(reg(&mut hw, 0x0A), 0xA6, "update in progress");
        assert_eq!(reg(&mut hw, 0x15), 0x80);
        assert_eq!(reg(&mut hw, 0x16), 0x02);
    }

    #[test]
    fn writes_set_the_clock_and_formats() {
        let mut hw = Hardware::new();
        hw.io_out8(0x70, 0x04);
        hw.io_out8(0x71, 0x15); // 15:xx
        hw.io_out8(0x70, 0x09);
        hw.io_out8(0x71, 0x94);
        assert_eq!(reg(&mut hw, 4), 0x15);
        assert_eq!((reg(&mut hw, 9), reg(&mut hw, 0x32)), (0x94, 0x19));
        hw.advance_to(3_000_000_000);
        assert_eq!(reg(&mut hw, 0), 0x03);
        // Binary, 12-hour mode.
        hw.io_out8(0x70, 0x0B);
        hw.io_out8(0x71, B_BINARY);
        assert_eq!(reg(&mut hw, 4), 0x80 | 3, "3 PM");
        assert_eq!(reg(&mut hw, 0), 3);
        // Plain RAM bytes keep the checksum valid.
        hw.io_out8(0x70, 0x10);
        hw.io_out8(0x71, 0x40);
        let sum = u16::from_be_bytes([reg(&mut hw, 0x2E), reg(&mut hw, 0x2F)]);
        assert_eq!(sum, (0x10..=0x2D).map(|r| reg(&mut hw, r) as u16).sum());
        assert_eq!(hw.io_in8(0x70), 0xFF);
    }
}
