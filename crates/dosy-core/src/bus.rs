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
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Flat {
        mem: Vec<u8>,
    }

    impl Bus for Flat {
        fn read8(&mut self, addr: Addr) -> u8 {
            self.mem[addr as usize]
        }
        fn write8(&mut self, addr: Addr, val: u8) {
            self.mem[addr as usize] = val;
        }
        fn io_in8(&mut self, _port: u16) -> u8 {
            0xFF
        }
        fn io_out8(&mut self, _port: u16, _val: u8) {}
    }

    #[test]
    fn default_wide_accessors_are_little_endian() {
        let mut bus = Flat { mem: vec![0; 16] };
        bus.write32(linear(0, 4), 0x1234_5678);
        assert_eq!(bus.read8(4), 0x78);
        assert_eq!(bus.read16(6), 0x1234);
        assert_eq!(bus.read32(4), 0x1234_5678);
        assert_eq!(linear(0xF000, 0xFFF0), 0xFFFF0);
    }
}
