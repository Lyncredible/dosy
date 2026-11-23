//! Program segment prefix (256 bytes at the start of a process's block).

use super::VERSION;
use crate::bus::{Bus, linear};

pub const INT20: u16 = 0x00;
/// Segment just past the process's memory.
pub const MEM_TOP: u16 = 0x02;
pub const CPM_CALL: u16 = 0x05;
pub const INT22: u16 = 0x0A;
pub const INT23: u16 = 0x0E;
pub const INT24: u16 = 0x12;
pub const PARENT: u16 = 0x16;
pub const JFT: u16 = 0x18;
pub const ENV: u16 = 0x2C;
pub const SAVED_SS_SP: u16 = 0x2E;
pub const JFT_SIZE: u16 = 0x32;
pub const JFT_PTR: u16 = 0x34;
pub const PREV_PSP: u16 = 0x38;
pub const DOS_VERSION: u16 = 0x40;
pub const DISPATCH: u16 = 0x50;
pub const FCB1: u16 = 0x5C;
pub const FCB2: u16 = 0x6C;
pub const TAIL: u16 = 0x80;

pub const SIZE: u16 = 0x100;
/// Paragraphs taken by a PSP.
pub const PARAS: u16 = SIZE / 16;
pub const JFT_LEN: u16 = 20;
/// Default JFT: stdin/stdout/stderr → SFT 1 (CON), stdaux → 0, stdprn → 2.
pub const DEFAULT_JFT: [u8; 5] = [1, 1, 1, 0, 2];

/// Build a PSP at `seg`. `mem_top` is the segment past the process's block;
/// INT 22h–24h are captured from the current IVT. The JFT gets the default
/// device handles (EXEC copies the parent's instead).
pub fn create_psp<B: Bus>(bus: &mut B, seg: u16, mem_top: u16, parent: u16, env: u16) {
    let at = |off: u16| linear(seg, off);
    for i in 0..SIZE {
        bus.write8(at(i), 0);
    }
    bus.write16(at(INT20), 0x20CD);
    bus.write16(at(MEM_TOP), mem_top);
    // CP/M-style far call; kept as the classic bytes, unsupported here.
    bus.write8(at(CPM_CALL), 0x9A);
    bus.write16(at(CPM_CALL + 1), 0xFEF0);
    bus.write16(at(CPM_CALL + 3), 0xF01D);
    for (off, vec) in [(INT22, 0x22u32), (INT23, 0x23), (INT24, 0x24)] {
        let v = bus.read32(vec * 4);
        bus.write32(at(off), v);
    }
    bus.write16(at(PARENT), parent);
    for i in 0..JFT_LEN {
        let h = DEFAULT_JFT.get(i as usize).copied().unwrap_or(0xFF);
        bus.write8(at(JFT + i), h);
    }
    bus.write16(at(ENV), env);
    bus.write16(at(JFT_SIZE), JFT_LEN);
    bus.write16(at(JFT_PTR), JFT);
    bus.write16(at(JFT_PTR + 2), seg);
    bus.write32(at(PREV_PSP), 0xFFFF_FFFF);
    bus.write16(at(DOS_VERSION), VERSION);
    // INT 21h; RETF
    for (i, b) in [0xCD, 0x21, 0xCB].into_iter().enumerate() {
        bus.write8(at(DISPATCH + i as u16), b);
    }
    for fcb in [FCB1, FCB2] {
        for i in 1..12 {
            bus.write8(at(fcb + i), b' ');
        }
    }
    bus.write8(at(TAIL + 1), 0x0D);
}
