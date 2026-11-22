//! High-level emulation (HLE) callback contract shared by the CPU, the
//! machine loop and the BIOS / DOS / shell implementations.
//!
//! Guest code enters Rust through ROM stubs containing the callback opcode
//! `FE 38 lo hi`. The CPU reports it as a callback with that id; the machine
//! dispatches on [`HleId::subsystem`].

use crate::bus::{Addr, Bus, linear};

/// The callback opcode prefix (`FE /7` with ModRM `0x38`, unused on x86).
pub const CALLBACK_OPCODE: [u8; 2] = [0xFE, 0x38];

/// Segment holding all HLE ROM stubs.
pub const ROM_SEGMENT: u16 = 0xF000;

/// Callback id. The high byte selects the subsystem, so each subsystem
/// allocates ids in its own range without touching shared code.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct HleId(pub u16);

/// Owner of a callback id range.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Subsystem {
    /// `0x00xx`: BIOS interrupts.
    Bios,
    /// `0x01xx`: DOS kernel.
    Dos,
    /// `0x02xx`: built-in shell.
    Shell,
    /// `0x03xx`: resident drivers (mouse, XMS, EMS, ...).
    Driver,
    Unknown,
}

impl HleId {
    pub const BIOS_BASE: u16 = 0x0000;
    pub const DOS_BASE: u16 = 0x0100;
    pub const SHELL_BASE: u16 = 0x0200;
    pub const DRIVER_BASE: u16 = 0x0300;
    /// Exclusive upper bound; keeps stubs inside the ROM segment.
    pub const LIMIT: u16 = 0x0400;

    pub fn subsystem(self) -> Subsystem {
        match self.0 >> 8 {
            0 => Subsystem::Bios,
            1 => Subsystem::Dos,
            2 => Subsystem::Shell,
            3 => Subsystem::Driver,
            _ => Subsystem::Unknown,
        }
    }

    /// Fixed stub location `ROM_SEGMENT:stub_offset()`; 8 bytes per id.
    pub fn stub_offset(self) -> u16 {
        assert!(self.0 < Self::LIMIT, "HleId out of range: {:#06x}", self.0);
        0x1000 + 8 * self.0
    }
}

/// What a handler wants the machine to do next.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HleResult {
    /// Continue after the callback opcode.
    Done,
    /// Waiting for input: rewind to the callback, set IF, halt until the next
    /// interrupt, then run the callback again. Never block the host.
    Retry,
}

/// How a stub returns to its caller.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StubKind {
    /// `STI; callback; IRET` — interrupt handler that may `Retry`.
    IretSti,
    /// `callback; IRET` — interrupt handler, IF left as the INT set it.
    Iret,
    /// `callback; RETF` — far-call entry point (e.g. XMS driver).
    Retf,
    /// `L: callback; JMP L` — a resident loop (the shell process).
    Loop,
}

/// Machine code for a stub: padded to 8 bytes with `NOP`.
pub fn stub_bytes(id: HleId, kind: StubKind) -> [u8; 8] {
    let [lo, hi] = id.0.to_le_bytes();
    let [c0, c1] = CALLBACK_OPCODE;
    const NOP: u8 = 0x90;
    match kind {
        StubKind::IretSti => [0xFB, c0, c1, lo, hi, 0xCF, NOP, NOP],
        StubKind::Iret => [c0, c1, lo, hi, 0xCF, NOP, NOP, NOP],
        StubKind::Retf => [c0, c1, lo, hi, 0xCB, NOP, NOP, NOP],
        // JMP short -6 back to the callback opcode.
        StubKind::Loop => [c0, c1, lo, hi, 0xEB, 0xFA, NOP, NOP],
    }
}

/// Write the stub for `id` at its fixed ROM location; returns its linear
/// address. The entry point is `ROM_SEGMENT:id.stub_offset()`.
pub fn install_stub<B: Bus>(bus: &mut B, id: HleId, kind: StubKind) -> Addr {
    let addr = linear(ROM_SEGMENT, id.stub_offset());
    bus.load_rom(addr, &stub_bytes(id, kind));
    addr
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bus::FlatBus;

    #[test]
    fn ids_route_by_high_byte() {
        assert_eq!(HleId(0x0016).subsystem(), Subsystem::Bios);
        assert_eq!(HleId(HleId::DOS_BASE + 0x21).subsystem(), Subsystem::Dos);
        assert_eq!(HleId(0x0201).subsystem(), Subsystem::Shell);
        assert_eq!(HleId(0x0300).subsystem(), Subsystem::Driver);
    }

    #[test]
    fn stubs_are_written_at_fixed_offsets() {
        let mut bus = FlatBus::new();
        let addr = install_stub(&mut bus, HleId(0x0102), StubKind::Iret);
        assert_eq!(addr, 0xF0000 + 0x1000 + 8 * 0x0102);
        assert_eq!(bus.read16(addr), 0x38FE);
        assert_eq!(bus.read16(addr + 2), 0x0102);
        assert_eq!(bus.read8(addr + 4), 0xCF);
        // Loop stub jumps back to its own callback opcode.
        let jmp = stub_bytes(HleId(0x200), StubKind::Loop);
        assert_eq!(4 + 2 + jmp[5] as i8 as i32, 0);
    }
}
