//! BIOS Data Area (segment 0040h) offsets and accessors.

use crate::bus::{Addr, Bus};

pub const SEG: u16 = 0x0040;
pub const BASE: Addr = 0x400;

pub const EQUIPMENT: u16 = 0x10;
pub const MEMORY_KB: u16 = 0x13;
pub const KBD_FLAGS1: u16 = 0x17;
pub const KBD_FLAGS2: u16 = 0x18;
pub const KBD_HEAD: u16 = 0x1A;
pub const KBD_TAIL: u16 = 0x1C;
/// Default keyboard ring buffer (offsets within segment 40h).
pub const KBD_BUF_DEFAULT: u16 = 0x1E;
pub const KBD_BUF_DEFAULT_END: u16 = 0x3E;
pub const VIDEO_MODE: u16 = 0x49;
pub const VIDEO_COLS: u16 = 0x4A;
pub const VIDEO_PAGE_SIZE: u16 = 0x4C;
pub const VIDEO_PAGE_OFFSET: u16 = 0x4E;
pub const CURSOR_POS: u16 = 0x50; // 8 words, one per page
pub const CURSOR_SHAPE: u16 = 0x60;
pub const VIDEO_PAGE: u16 = 0x62;
pub const CRTC_BASE: u16 = 0x63;
pub const VIDEO_MODE_CTRL: u16 = 0x65;
pub const VIDEO_PALETTE: u16 = 0x66;
pub const TICKS: u16 = 0x6C;
pub const MIDNIGHT: u16 = 0x70;
pub const KBD_BUF_START: u16 = 0x80;
pub const KBD_BUF_END: u16 = 0x82;
pub const VIDEO_ROWS: u16 = 0x84; // rows - 1
pub const CHAR_HEIGHT: u16 = 0x85;
pub const VIDEO_CTL: u16 = 0x87;
pub const VIDEO_SWITCHES: u16 = 0x88;
pub const VGA_FLAGS: u16 = 0x89;
pub const KBD_FLAGS3: u16 = 0x96;
pub const KBD_FLAGS4: u16 = 0x97;

/// Ticks per day: the count wraps to 0 here and sets the midnight flag.
pub const TICKS_PER_DAY: u32 = 0x18_00B0;

pub fn read8<B: Bus>(bus: &mut B, off: u16) -> u8 {
    bus.read8(BASE + off as Addr)
}
pub fn write8<B: Bus>(bus: &mut B, off: u16, v: u8) {
    bus.write8(BASE + off as Addr, v)
}
pub fn read16<B: Bus>(bus: &mut B, off: u16) -> u16 {
    bus.read16(BASE + off as Addr)
}
pub fn write16<B: Bus>(bus: &mut B, off: u16, v: u16) {
    bus.write16(BASE + off as Addr, v)
}
pub fn read32<B: Bus>(bus: &mut B, off: u16) -> u32 {
    bus.read32(BASE + off as Addr)
}
pub fn write32<B: Bus>(bus: &mut B, off: u16, v: u32) {
    bus.write32(BASE + off as Addr, v)
}
