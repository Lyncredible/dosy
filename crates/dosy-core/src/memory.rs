//! Guest RAM: 1 MiB + HMA, A20 gate, write-protected ROM and the VGA window.

use crate::bus::Addr;

/// RAM size: 1 MiB plus the high memory area (`FFFF:0010–FFFF:FFFF`).
pub const RAM_SIZE: usize = 0x10_FFF0;

/// Physical range forwarded to [`Mmio`] (VGA graphics/text window).
pub const MMIO_START: Addr = 0xA_0000;
pub const MMIO_END: Addr = 0xC_0000; // exclusive

/// System BIOS ROM, write-protected at power-on.
pub const BIOS_ROM_START: Addr = 0xF_0000;
pub const BIOS_ROM_END: Addr = 0x10_0000; // exclusive

/// ROM protection granularity.
pub const ROM_PAGE: Addr = 0x1000;
const ROM_PAGES: usize = 0x10_0000 / ROM_PAGE as usize;

/// Handler for the `A0000–BFFFF` window (the VGA implements it). Addresses
/// are physical (A20 already applied). Returning `None` / `false` falls back
/// to plain RAM.
pub trait Mmio {
    fn read8(&mut self, addr: Addr) -> Option<u8>;
    fn write8(&mut self, addr: Addr, val: u8) -> bool;
}

/// No MMIO device: the window is plain RAM.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoMmio;

impl Mmio for NoMmio {
    #[inline]
    fn read8(&mut self, _addr: Addr) -> Option<u8> {
        None
    }
    #[inline]
    fn write8(&mut self, _addr: Addr, _val: u8) -> bool {
        false
    }
}

#[derive(Clone)]
pub struct Memory {
    ram: Vec<u8>,
    /// `!0` when A20 is enabled, otherwise clears bit 20 (wrap at 1 MiB).
    a20_mask: Addr,
    rom: [bool; ROM_PAGES],
}

impl Default for Memory {
    fn default() -> Self {
        Self::new()
    }
}

impl Memory {
    /// Zeroed RAM, A20 disabled, `F0000–FFFFF` write-protected.
    pub fn new() -> Self {
        let mut m = Self {
            ram: vec![0; RAM_SIZE],
            a20_mask: !0x10_0000,
            rom: [false; ROM_PAGES],
        };
        m.set_rom(BIOS_ROM_START, BIOS_ROM_END, true);
        m
    }

    pub fn a20(&self) -> bool {
        self.a20_mask == !0
    }

    pub fn set_a20(&mut self, on: bool) {
        self.a20_mask = if on { !0 } else { !0x10_0000 };
    }

    /// Apply the A20 gate to a linear address.
    #[inline]
    pub fn phys(&self, addr: Addr) -> Addr {
        addr & self.a20_mask
    }

    /// Mark `[start, end)` (page-aligned, below 1 MiB) as ROM or RAM.
    pub fn set_rom(&mut self, start: Addr, end: Addr, rom: bool) {
        assert!(
            start % ROM_PAGE == 0 && end % ROM_PAGE == 0 && end <= 0x10_0000,
            "ROM range must be page-aligned below 1 MiB"
        );
        for p in (start / ROM_PAGE)..(end / ROM_PAGE) {
            self.rom[p as usize] = rom;
        }
    }

    #[inline]
    pub fn is_rom(&self, phys: Addr) -> bool {
        phys < 0x10_0000 && self.rom[(phys / ROM_PAGE) as usize]
    }

    /// Guest read. Unbacked addresses read `0xFF` (open bus).
    #[inline]
    pub fn read8<M: Mmio>(&mut self, addr: Addr, mmio: &mut M) -> u8 {
        let a = self.phys(addr);
        if a.wrapping_sub(MMIO_START) < MMIO_END - MMIO_START {
            if let Some(v) = mmio.read8(a) {
                return v;
            }
        }
        self.ram.get(a as usize).copied().unwrap_or(0xFF)
    }

    /// Guest write. ROM and unbacked addresses ignore writes.
    #[inline]
    pub fn write8<M: Mmio>(&mut self, addr: Addr, val: u8, mmio: &mut M) {
        let a = self.phys(addr);
        if a.wrapping_sub(MMIO_START) < MMIO_END - MMIO_START && mmio.write8(a, val) {
            return;
        }
        if self.is_rom(a) {
            return;
        }
        if let Some(b) = self.ram.get_mut(a as usize) {
            *b = val;
        }
    }

    /// Host write to physical RAM, bypassing ROM protection and MMIO.
    pub fn load_rom(&mut self, phys: Addr, bytes: &[u8]) {
        let a = phys as usize;
        self.ram[a..a + bytes.len()].copy_from_slice(bytes);
    }

    /// Host view of physical RAM.
    pub fn ram(&self) -> &[u8] {
        &self.ram
    }

    /// Mutable host view of physical RAM (bypasses ROM protection).
    pub fn ram_mut(&mut self) -> &mut [u8] {
        &mut self.ram
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a20_wraps_at_1mib_when_off() {
        let mut m = Memory::new();
        let mut io = NoMmio;
        assert!(!m.a20());
        m.write8(0x10_0010, 0x42, &mut io); // FFFF:0020
        assert_eq!(m.read8(0x10, &mut io), 0x42);
        m.set_a20(true);
        assert_eq!(m.read8(0x10_0010, &mut io), 0);
        m.write8(0x10_0010, 0x99, &mut io);
        assert_eq!(m.read8(0x10_0010, &mut io), 0x99);
        assert_eq!(m.read8(0x10, &mut io), 0x42);
        // Beyond the HMA is open bus.
        assert_eq!(m.read8(0x10_FFF0, &mut io), 0xFF);
        m.write8(0x20_0000, 1, &mut io);
    }

    #[test]
    fn rom_ignores_guest_writes_but_not_load_rom() {
        let mut m = Memory::new();
        let mut io = NoMmio;
        m.write8(0xF_FFF0, 0xEA, &mut io);
        assert_eq!(m.read8(0xF_FFF0, &mut io), 0);
        m.load_rom(0xF_FFF0, &[0xEA, 0x5B]);
        assert_eq!(m.read8(0xF_FFF1, &mut io), 0x5B);
        m.write8(0xE_FFFF, 7, &mut io);
        assert_eq!(m.read8(0xE_FFFF, &mut io), 7);
        m.set_rom(0xC_0000, 0xC_8000, true);
        m.write8(0xC_0000, 7, &mut io);
        assert_eq!(m.read8(0xC_0000, &mut io), 0);
    }

    struct Window {
        writes: Vec<(Addr, u8)>,
    }
    impl Mmio for Window {
        fn read8(&mut self, addr: Addr) -> Option<u8> {
            (addr < 0xB_0000).then_some(0xAB)
        }
        fn write8(&mut self, addr: Addr, val: u8) -> bool {
            self.writes.push((addr, val));
            addr < 0xB_0000
        }
    }

    #[test]
    fn mmio_window_is_routed() {
        let mut m = Memory::new();
        let mut w = Window { writes: vec![] };
        m.write8(0xA_0000, 1, &mut w);
        m.write8(0xB_8000, 2, &mut w);
        m.write8(0x9_FFFF, 3, &mut w);
        m.write8(0xC_0000, 4, &mut w);
        assert_eq!(w.writes, vec![(0xA_0000, 1), (0xB_8000, 2)]);
        assert_eq!(m.read8(0xA_0000, &mut w), 0xAB);
        assert_eq!(m.ram()[0xA_0000], 0);
        // Declined accesses fall through to RAM.
        assert_eq!(m.read8(0xB_8000, &mut w), 2);
        assert_eq!(m.read8(0x9_FFFF, &mut w), 3);
        // A20 is applied before routing: 1 MiB + A0000 wraps into the window.
        assert_eq!(m.read8(0x1A_0000, &mut w), 0xAB);
    }
}
