//! The CPU's view of the machine: linear memory plus I/O ports.

/// Linear (physical) address. Real mode: `(seg << 4) + off`; A20 masking is
/// the memory implementation's job, not the CPU's.
pub type Addr = u32;

/// Linear address of `seg:off` in real mode (no A20 masking).
#[inline]
pub fn linear(seg: u16, off: u16) -> Addr {
    ((seg as Addr) << 4) + off as Addr
}

/// Memory and I/O as seen by the CPU. Reads take `&mut self` because MMIO and
/// port reads may have side effects (e.g. VGA latches, PIC acknowledge).
///
/// 16/32-bit accessors are little-endian and byte-wise by default; implementors
/// may override them for speed. Word accesses wrap at the address space end
/// only as far as the implementor decides; the CPU handles segment wrap.
pub trait Bus {
    fn read8(&mut self, addr: Addr) -> u8;
    fn write8(&mut self, addr: Addr, val: u8);
    fn io_in8(&mut self, port: u16) -> u8;
    fn io_out8(&mut self, port: u16, val: u8);

    fn read16(&mut self, addr: Addr) -> u16 {
        u16::from_le_bytes([self.read8(addr), self.read8(addr.wrapping_add(1))])
    }
    fn write16(&mut self, addr: Addr, val: u16) {
        let [lo, hi] = val.to_le_bytes();
        self.write8(addr, lo);
        self.write8(addr.wrapping_add(1), hi);
    }
    fn read32(&mut self, addr: Addr) -> u32 {
        self.read16(addr) as u32 | (self.read16(addr.wrapping_add(2)) as u32) << 16
    }
    fn write32(&mut self, addr: Addr, val: u32) {
        self.write16(addr, val as u16);
        self.write16(addr.wrapping_add(2), (val >> 16) as u16);
    }
    fn io_in16(&mut self, port: u16) -> u16 {
        u16::from_le_bytes([self.io_in8(port), self.io_in8(port.wrapping_add(1))])
    }
    fn io_out16(&mut self, port: u16, val: u16) {
        let [lo, hi] = val.to_le_bytes();
        self.io_out8(port, lo);
        self.io_out8(port.wrapping_add(1), hi);
    }

    /// Host-side write that bypasses ROM write protection (BIOS/HLE setup).
    /// Never used by the CPU. Default: plain `write8`s.
    fn load_rom(&mut self, addr: Addr, bytes: &[u8]) {
        for (i, &b) in bytes.iter().enumerate() {
            self.write8(addr.wrapping_add(i as Addr), b);
        }
    }
}

/// Simple bus for tests: flat RAM covering 1 MiB + HMA (no A20 wrap, no MMIO),
/// scripted port inputs and a log of port outputs.
#[derive(Clone, Debug)]
pub struct FlatBus {
    pub mem: Vec<u8>,
    /// Value returned by `io_in8(port)`; unlisted ports read `0xFF`.
    pub io_in: std::collections::HashMap<u16, u8>,
    /// Every `io_out8` in order.
    pub io_out: Vec<(u16, u8)>,
}

impl FlatBus {
    /// Size of the RAM: 1 MiB plus the high memory area.
    pub const SIZE: usize = 0x10_FFF0;

    pub fn new() -> Self {
        Self {
            mem: vec![0; Self::SIZE],
            io_in: Default::default(),
            io_out: Vec::new(),
        }
    }

    /// Copy `bytes` into memory at `addr`.
    pub fn load(&mut self, addr: Addr, bytes: &[u8]) {
        let a = addr as usize;
        self.mem[a..a + bytes.len()].copy_from_slice(bytes);
    }
}

impl Default for FlatBus {
    fn default() -> Self {
        Self::new()
    }
}

impl Bus for FlatBus {
    fn read8(&mut self, addr: Addr) -> u8 {
        self.mem.get(addr as usize).copied().unwrap_or(0xFF)
    }
    fn write8(&mut self, addr: Addr, val: u8) {
        if let Some(b) = self.mem.get_mut(addr as usize) {
            *b = val;
        }
    }
    fn io_in8(&mut self, port: u16) -> u8 {
        self.io_in.get(&port).copied().unwrap_or(0xFF)
    }
    fn io_out8(&mut self, port: u16, val: u8) {
        self.io_out.push((port, val));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flat_bus_ports_are_scripted_and_logged() {
        let mut bus = FlatBus::new();
        bus.io_in.insert(0x60, 0x1E);
        assert_eq!(bus.io_in8(0x60), 0x1E);
        assert_eq!(bus.io_in8(0x61), 0xFF);
        bus.io_out16(0x20, 0x1220);
        assert_eq!(bus.io_out, vec![(0x20, 0x20), (0x21, 0x12)]);
    }

    #[test]
    fn default_wide_accessors_are_little_endian() {
        let mut bus = FlatBus::new();
        bus.write32(linear(0, 4), 0x1234_5678);
        assert_eq!(bus.read8(4), 0x78);
        assert_eq!(bus.read16(6), 0x1234);
        assert_eq!(bus.read32(4), 0x1234_5678);
        assert_eq!(linear(0xF000, 0xFFF0), 0xFFFF0);
    }
}
