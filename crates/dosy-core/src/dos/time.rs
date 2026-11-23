//! Date (kept by the kernel) and time (BIOS tick count) for INT 21h/2Ah–2Dh.

use crate::bus::Bus;

/// BDA tick count (dword) and midnight-rollover flag.
pub const BDA_TICKS: u32 = 0x46C;
pub const BDA_MIDNIGHT: u32 = 0x470;
/// BIOS ticks per day (~18.2 Hz).
pub const TICKS_PER_DAY: u64 = 0x1800B0;
const CS_PER_DAY: u64 = 8_640_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Date {
    pub year: u16,
    pub month: u8,
    pub day: u8,
}

impl Default for Date {
    fn default() -> Self {
        Self {
            year: 1980,
            month: 1,
            day: 1,
        }
    }
}

fn is_leap(y: u16) -> bool {
    (y.is_multiple_of(4) && !y.is_multiple_of(100)) || y.is_multiple_of(400)
}

fn days_in_month(y: u16, m: u8) -> u8 {
    match m {
        2 if is_leap(y) => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

impl Date {
    /// A date DOS accepts (1980–2099).
    pub fn new(year: u16, month: u8, day: u8) -> Option<Self> {
        let ok = (1980..=2099).contains(&year)
            && (1..=12).contains(&month)
            && day >= 1
            && day <= days_in_month(year, month);
        ok.then_some(Self { year, month, day })
    }

    /// 0 = Sunday.
    pub fn weekday(&self) -> u8 {
        const T: [u16; 12] = [0, 3, 2, 5, 0, 3, 5, 1, 4, 6, 2, 4];
        let y = self.year - (self.month < 3) as u16;
        let d = y + y / 4 - y / 100 + y / 400 + T[self.month as usize - 1] + self.day as u16;
        (d % 7) as u8
    }

    pub fn next_day(&mut self) {
        if self.day < days_in_month(self.year, self.month) {
            self.day += 1;
        } else if self.month < 12 {
            self.day = 1;
            self.month += 1;
        } else {
            *self = Self {
                year: self.year + 1,
                month: 1,
                day: 1,
            };
        }
    }
}

/// Time of day `(hour, minute, second, hundredths)` from the BDA ticks.
pub fn get_time<B: Bus>(bus: &mut B) -> (u8, u8, u8, u8) {
    let ticks = bus.read32(BDA_TICKS) as u64 % TICKS_PER_DAY;
    let cs = ticks * CS_PER_DAY / TICKS_PER_DAY;
    (
        (cs / 360_000) as u8,
        (cs / 6_000 % 60) as u8,
        (cs / 100 % 60) as u8,
        (cs % 100) as u8,
    )
}

/// Set the BDA ticks; false if the time is invalid.
pub fn set_time<B: Bus>(bus: &mut B, h: u8, m: u8, s: u8, cs: u8) -> bool {
    if h > 23 || m > 59 || s > 59 || cs > 99 {
        return false;
    }
    let total = ((h as u64 * 60 + m as u64) * 60 + s as u64) * 100 + cs as u64;
    // Round up so reading back never lands in the previous second.
    let ticks = (total * TICKS_PER_DAY).div_ceil(CS_PER_DAY);
    bus.write32(BDA_TICKS, ticks as u32);
    bus.write8(BDA_MIDNIGHT, 0);
    true
}
