//! INT 09h (keyboard IRQ: set-1 scancodes from port 60h into the BDA ring
//! buffer) and INT 16h (keyboard services).

use super::{bda, set_caller_flag};
use crate::bus::{Addr, Bus};
use crate::cpu::{Cpu, flags};
use crate::hle::HleResult;

// KBD_FLAGS1 (40:17).
const RSHIFT: u8 = 0x01;
const LSHIFT: u8 = 0x02;
const CTRL: u8 = 0x04;
const ALT: u8 = 0x08;
const SCROLL: u8 = 0x10;
const NUM: u8 = 0x20;
const CAPS: u8 = 0x40;
const INSERT: u8 = 0x80;
// KBD_FLAGS2 (40:18).
const LCTRL_DOWN: u8 = 0x01;
const LALT_DOWN: u8 = 0x02;
const SYSRQ_DOWN: u8 = 0x04;
// KBD_FLAGS3 (40:96).
const LAST_E1: u8 = 0x01;
const LAST_E0: u8 = 0x02;
const RCTRL_DOWN: u8 = 0x04;
const RALT_DOWN: u8 = 0x08;

const NONE: u16 = 0;

/// Scancode → `[normal, shift, ctrl, alt]` scan/ASCII words. ASCII `F0h`
/// marks keys only the enhanced functions (INT 16h/1xh) report.
#[rustfmt::skip]
static TABLE: [[u16; 4]; 0x59] = [
    [NONE, NONE, NONE, NONE],
    [0x011B, 0x011B, 0x011B, 0x01F0], // Esc
    [0x0231, 0x0221, NONE, 0x7800],   // 1 !
    [0x0332, 0x0340, 0x0300, 0x7900], // 2 @
    [0x0433, 0x0423, NONE, 0x7A00],   // 3 #
    [0x0534, 0x0524, NONE, 0x7B00],   // 4 $
    [0x0635, 0x0625, NONE, 0x7C00],   // 5 %
    [0x0736, 0x075E, 0x071E, 0x7D00], // 6 ^
    [0x0837, 0x0826, NONE, 0x7E00],   // 7 &
    [0x0938, 0x092A, NONE, 0x7F00],   // 8 *
    [0x0A39, 0x0A28, NONE, 0x8000],   // 9 (
    [0x0B30, 0x0B29, NONE, 0x8100],   // 0 )
    [0x0C2D, 0x0C5F, 0x0C1F, 0x8200], // - _
    [0x0D3D, 0x0D2B, NONE, 0x8300],   // = +
    [0x0E08, 0x0E08, 0x0E7F, 0x0EF0], // Backspace
    [0x0F09, 0x0F00, 0x9400, NONE],   // Tab
    [0x1071, 0x1051, 0x1011, 0x1000], // Q
    [0x1177, 0x1157, 0x1117, 0x1100], // W
    [0x1265, 0x1245, 0x1205, 0x1200], // E
    [0x1372, 0x1352, 0x1312, 0x1300], // R
    [0x1474, 0x1454, 0x1414, 0x1400], // T
    [0x1579, 0x1559, 0x1519, 0x1500], // Y
    [0x1675, 0x1655, 0x1615, 0x1600], // U
    [0x1769, 0x1749, 0x1709, 0x1700], // I
    [0x186F, 0x184F, 0x180F, 0x1800], // O
    [0x1970, 0x1950, 0x1910, 0x1900], // P
    [0x1A5B, 0x1A7B, 0x1A1B, 0x1AF0], // [ {
    [0x1B5D, 0x1B7D, 0x1B1D, 0x1BF0], // ] }
    [0x1C0D, 0x1C0D, 0x1C0A, 0x1CF0], // Enter
    [NONE, NONE, NONE, NONE],         // LCtrl
    [0x1E61, 0x1E41, 0x1E01, 0x1E00], // A
    [0x1F73, 0x1F53, 0x1F13, 0x1F00], // S
    [0x2064, 0x2044, 0x2004, 0x2000], // D
    [0x2166, 0x2146, 0x2106, 0x2100], // F
    [0x2267, 0x2247, 0x2207, 0x2200], // G
    [0x2368, 0x2348, 0x2308, 0x2300], // H
    [0x246A, 0x244A, 0x240A, 0x2400], // J
    [0x256B, 0x254B, 0x250B, 0x2500], // K
    [0x266C, 0x264C, 0x260C, 0x2600], // L
    [0x273B, 0x273A, NONE, 0x27F0],   // ; :
    [0x2827, 0x2822, NONE, 0x28F0],   // ' "
    [0x2960, 0x297E, NONE, 0x29F0],   // ` ~
    [NONE, NONE, NONE, NONE],         // LShift
    [0x2B5C, 0x2B7C, 0x2B1C, 0x2BF0], // \ |
    [0x2C7A, 0x2C5A, 0x2C1A, 0x2C00], // Z
    [0x2D78, 0x2D58, 0x2D18, 0x2D00], // X
    [0x2E63, 0x2E43, 0x2E03, 0x2E00], // C
    [0x2F76, 0x2F56, 0x2F16, 0x2F00], // V
    [0x3062, 0x3042, 0x3002, 0x3000], // B
    [0x316E, 0x314E, 0x310E, 0x3100], // N
    [0x326D, 0x324D, 0x320D, 0x3200], // M
    [0x332C, 0x333C, NONE, 0x33F0],   // , <
    [0x342E, 0x343E, NONE, 0x34F0],   // . >
    [0x352F, 0x353F, NONE, 0x35F0],   // / ?
    [NONE, NONE, NONE, NONE],         // RShift
    [0x372A, 0x372A, 0x9600, 0x37F0], // keypad *
    [NONE, NONE, NONE, NONE],         // LAlt
    [0x3920, 0x3920, 0x3920, 0x3920], // Space
    [NONE, NONE, NONE, NONE],         // CapsLock
    [0x3B00, 0x5400, 0x5E00, 0x6800], // F1
    [0x3C00, 0x5500, 0x5F00, 0x6900], // F2
    [0x3D00, 0x5600, 0x6000, 0x6A00], // F3
    [0x3E00, 0x5700, 0x6100, 0x6B00], // F4
    [0x3F00, 0x5800, 0x6200, 0x6C00], // F5
    [0x4000, 0x5900, 0x6300, 0x6D00], // F6
    [0x4100, 0x5A00, 0x6400, 0x6E00], // F7
    [0x4200, 0x5B00, 0x6500, 0x6F00], // F8
    [0x4300, 0x5C00, 0x6600, 0x7000], // F9
    [0x4400, 0x5D00, 0x6700, 0x7100], // F10
    [NONE, NONE, NONE, NONE],         // NumLock
    [NONE, NONE, NONE, NONE],         // ScrollLock
    [0x4700, 0x4737, 0x7700, NONE],   // keypad 7 Home
    [0x4800, 0x4838, 0x8D00, NONE],   // keypad 8 Up
    [0x4900, 0x4939, 0x8400, NONE],   // keypad 9 PgUp
    [0x4A2D, 0x4A2D, 0x8E00, 0x4AF0], // keypad -
    [0x4B00, 0x4B34, 0x7300, NONE],   // keypad 4 Left
    [0x4CF0, 0x4C35, 0x8F00, NONE],   // keypad 5
    [0x4D00, 0x4D36, 0x7400, NONE],   // keypad 6 Right
    [0x4E2B, 0x4E2B, 0x9000, 0x4EF0], // keypad +
    [0x4F00, 0x4F31, 0x7500, NONE],   // keypad 1 End
    [0x5000, 0x5032, 0x9100, NONE],   // keypad 2 Down
    [0x5100, 0x5133, 0x7600, NONE],   // keypad 3 PgDn
    [0x5200, 0x5230, 0x9200, NONE],   // keypad 0 Ins
    [0x5300, 0x532E, 0x9300, NONE],   // keypad . Del
    [NONE, NONE, NONE, NONE],         // SysRq
    [NONE, NONE, NONE, NONE],
    [0x565C, 0x567C, NONE, NONE],     // 102nd key
    [0x8500, 0x8700, 0x8900, 0x8B00], // F11
    [0x8600, 0x8800, 0x8A00, 0x8C00], // F12
];

fn is_letter(scan: u8) -> bool {
    matches!(scan, 0x10..=0x19 | 0x1E..=0x26 | 0x2C..=0x32)
}

fn is_keypad(scan: u8) -> bool {
    matches!(scan, 0x47..=0x53) && !matches!(scan, 0x4A | 0x4E)
}

/// Translate a make code (without E0) given the shift flags; `None` for
/// keys that produce nothing in that state.
pub fn translate(scan: u8, e0: bool, f1: u8) -> Option<u16> {
    let shift = f1 & (LSHIFT | RSHIFT) != 0;
    let (ctrl, alt) = (f1 & CTRL != 0, f1 & ALT != 0);
    if e0 {
        return match scan {
            0x1C => Some([0xE00D, 0xE00D, 0xE00A, 0xA600][col(ctrl, alt, shift)]),
            0x35 => Some([0xE02F, 0xE02F, 0x9500, 0xA400][col(ctrl, alt, shift)]),
            0x47..=0x53 if is_keypad(scan) => {
                let s = scan as u16;
                Some(if alt {
                    (s + 0x50) << 8
                } else if ctrl {
                    TABLE[scan as usize][2] & 0xFF00 | 0xE0
                } else {
                    s << 8 | 0xE0
                })
            }
            _ => None,
        };
    }
    let row = TABLE.get(scan as usize)?;
    let mut c = col(ctrl, alt, shift);
    if c <= 1 {
        let upper = if is_letter(scan) {
            shift ^ (f1 & CAPS != 0)
        } else if is_keypad(scan) {
            shift ^ (f1 & NUM != 0)
        } else {
            shift
        };
        c = upper as usize;
    }
    let code = row[c];
    (code != NONE).then_some(code)
}

fn col(ctrl: bool, alt: bool, shift: bool) -> usize {
    if alt {
        3
    } else if ctrl {
        2
    } else {
        shift as usize
    }
}

/// Append a scan/ASCII word to the BDA ring buffer; false if full.
pub fn push_key<B: Bus>(bus: &mut B, code: u16) -> bool {
    let (start, end) = (
        bda::read16(bus, bda::KBD_BUF_START),
        bda::read16(bus, bda::KBD_BUF_END),
    );
    let (head, tail) = (
        bda::read16(bus, bda::KBD_HEAD),
        bda::read16(bus, bda::KBD_TAIL),
    );
    let mut next = tail.wrapping_add(2);
    if next >= end {
        next = start;
    }
    if next == head {
        return false;
    }
    bus.write16(bda::BASE + tail as Addr, code);
    bda::write16(bus, bda::KBD_TAIL, next);
    true
}

fn peek_raw<B: Bus>(bus: &mut B) -> Option<u16> {
    let (head, tail) = (
        bda::read16(bus, bda::KBD_HEAD),
        bda::read16(bus, bda::KBD_TAIL),
    );
    (head != tail).then(|| bus.read16(bda::BASE + head as Addr))
}

/// Take the next key as INT 16h/00 would (`None` when the buffer is empty).
/// For HLE code that reads the keyboard directly (the shell).
pub fn read_key<B: Bus>(bus: &mut B) -> Option<u16> {
    let k = next_key(bus, false)?;
    pop_raw(bus);
    Some(k)
}

fn pop_raw<B: Bus>(bus: &mut B) {
    let (start, end) = (
        bda::read16(bus, bda::KBD_BUF_START),
        bda::read16(bus, bda::KBD_BUF_END),
    );
    let mut head = bda::read16(bus, bda::KBD_HEAD).wrapping_add(2);
    if head >= end {
        head = start;
    }
    bda::write16(bus, bda::KBD_HEAD, head);
}

/// Next key as the standard (`enhanced = false`) or enhanced functions see
/// it. Standard calls drop enhanced-only keys from the buffer.
fn next_key<B: Bus>(bus: &mut B, enhanced: bool) -> Option<u16> {
    loop {
        let code = peek_raw(bus)?;
        let (scan, ascii) = ((code >> 8) as u8, code as u8);
        if enhanced {
            return Some(if ascii == 0xF0 && scan != 0 {
                code & 0xFF00
            } else {
                code
            });
        }
        if scan == 0xE0 {
            // Keypad Enter / slash look like the main keys.
            let main = if ascii == b'/' { 0x35 } else { 0x1C };
            return Some(main << 8 | ascii as u16);
        }
        if ascii == 0xF0 || scan > 0x84 {
            pop_raw(bus);
            continue;
        }
        return Some(if ascii == 0xE0 && scan != 0 {
            code & 0xFF00
        } else {
            code
        });
    }
}

/// Remove and return the next key (standard functions' view), if any.
pub fn read_key<B: Bus>(bus: &mut B) -> Option<u16> {
    let k = next_key(bus, false)?;
    pop_raw(bus);
    Some(k)
}

/// The next key without removing it.
pub fn peek_key<B: Bus>(bus: &mut B) -> Option<u16> {
    next_key(bus, false)
}

/// Empty the keyboard buffer.
pub fn flush<B: Bus>(bus: &mut B) {
    let tail = bda::read16(bus, bda::KBD_TAIL);
    bda::write16(bus, bda::KBD_HEAD, tail);
}

/// BDA break flag (bit 7 of 0040:0071), set by Ctrl-Break.
pub const BREAK_FLAG: u16 = 0x71;

/// INT 09h body: one scancode from port 60h, then EOI. Returns true on
/// Ctrl-Break: the caller then issues INT 1Bh.
pub fn int09<B: Bus>(bus: &mut B) -> bool {
    let code = bus.io_in8(0x60);
    let brk = process_scancode(bus, code);
    bus.io_out8(0x20, 0x20);
    brk
}

fn process_scancode<B: Bus>(bus: &mut B, code: u8) -> bool {
    let mut f1 = bda::read8(bus, bda::KBD_FLAGS1);
    let mut f2 = bda::read8(bus, bda::KBD_FLAGS2);
    let mut f3 = bda::read8(bus, bda::KBD_FLAGS3);
    match code {
        0xE0 => {
            bda::write8(bus, bda::KBD_FLAGS3, f3 | LAST_E0);
            return false;
        }
        0xE1 => {
            bda::write8(bus, bda::KBD_FLAGS3, f3 | LAST_E1);
            return false;
        }
        _ => {}
    }
    let e0 = f3 & LAST_E0 != 0;
    f3 &= !LAST_E0;
    let (make, scan) = (code & 0x80 == 0, code & 0x7F);
    if f3 & LAST_E1 != 0 {
        // Pause: E1 1D 45 E1 9D C5. Swallow the sequence.
        if scan == 0x45 {
            f3 &= !LAST_E1;
        }
        bda::write8(bus, bda::KBD_FLAGS3, f3);
        return false;
    }
    let mut key = None;
    let mut brk = false;
    match (scan, e0) {
        (0x2A | 0x36, true) => {} // fake shifts around grey keys
        (0x2A, false) => set(&mut f1, LSHIFT, make),
        (0x36, false) => set(&mut f1, RSHIFT, make),
        (0x1D, _) => {
            if e0 {
                set(&mut f3, RCTRL_DOWN, make);
            } else {
                set(&mut f2, LCTRL_DOWN, make);
            }
            set(&mut f1, CTRL, f2 & LCTRL_DOWN != 0 || f3 & RCTRL_DOWN != 0);
        }
        (0x38, _) => {
            if e0 {
                set(&mut f3, RALT_DOWN, make);
            } else {
                set(&mut f2, LALT_DOWN, make);
            }
            set(&mut f1, ALT, f2 & LALT_DOWN != 0 || f3 & RALT_DOWN != 0);
        }
        (0x3A, false) => toggle(&mut f1, &mut f2, CAPS, make),
        (0x45, false) => toggle(&mut f1, &mut f2, NUM, make),
        // Ctrl-Break (E0 46 with Ctrl; plain 46 with Ctrl on old BIOSes).
        (0x46, _) if f1 & CTRL != 0 => {
            if make {
                brk = true;
                flush(bus);
                push_key(bus, 0x0000);
                let f = bda::read8(bus, BREAK_FLAG);
                bda::write8(bus, BREAK_FLAG, f | 0x80);
            }
        }
        (0x46, false) => toggle(&mut f1, &mut f2, SCROLL, make),
        (0x54, false) => set(&mut f2, SYSRQ_DOWN, make),
        _ if make => {
            let numpad_ins = scan == 0x52 && (e0 || f1 & (NUM | LSHIFT | RSHIFT) == 0);
            if numpad_ins && f1 & (CTRL | ALT) == 0 {
                if f2 & INSERT == 0 {
                    f1 ^= INSERT;
                }
                f2 |= INSERT;
            }
            key = translate(scan, e0, f1);
        }
        _ => {
            if scan == 0x52 {
                f2 &= !INSERT;
            }
        }
    }
    bda::write8(bus, bda::KBD_FLAGS1, f1);
    bda::write8(bus, bda::KBD_FLAGS2, f2);
    bda::write8(bus, bda::KBD_FLAGS3, f3);
    bda::write8(bus, bda::KBD_FLAGS4, (f1 >> 4) & 7);
    if let Some(k) = key {
        if !push_key(bus, k) {
            log::debug!("keyboard buffer full; dropped {k:04X}");
        }
    }
    brk
}

fn set(f: &mut u8, bit: u8, on: bool) {
    if on {
        *f |= bit;
    } else {
        *f &= !bit;
    }
}

/// Lock keys: toggle on the first make, track "held" in FLAGS2 (same bit).
fn toggle(f1: &mut u8, f2: &mut u8, bit: u8, make: bool) {
    if make {
        if *f2 & bit == 0 {
            *f1 ^= bit;
        }
        *f2 |= bit;
    } else {
        *f2 &= !bit;
    }
}

/// INT 16h.
pub fn int16<B: Bus>(cpu: &mut Cpu, bus: &mut B) -> HleResult {
    let ah = cpu.regs.ah();
    match ah {
        0x00 | 0x10 => match next_key(bus, ah == 0x10) {
            Some(k) => {
                pop_raw(bus);
                cpu.regs.set_ax(k);
            }
            None => return HleResult::Retry,
        },
        0x01 | 0x11 => {
            let k = next_key(bus, ah == 0x11);
            if let Some(k) = k {
                cpu.regs.set_ax(k);
            }
            set_caller_flag(cpu, bus, flags::ZF, k.is_none());
        }
        0x02 => cpu.regs.set_al(bda::read8(bus, bda::KBD_FLAGS1)),
        0x12 => {
            let (f1, f2, f3) = (
                bda::read8(bus, bda::KBD_FLAGS1),
                bda::read8(bus, bda::KBD_FLAGS2),
                bda::read8(bus, bda::KBD_FLAGS3),
            );
            let hi = (f2 & (LCTRL_DOWN | LALT_DOWN))
                | (f3 & (RCTRL_DOWN | RALT_DOWN))
                | (f2 & (SCROLL | NUM | CAPS))
                | if f2 & SYSRQ_DOWN != 0 { 0x80 } else { 0 };
            cpu.regs.set_ax((hi as u16) << 8 | f1 as u16);
        }
        0x05 => {
            let ok = push_key(bus, cpu.regs.cx());
            cpu.regs.set_al(!ok as u8);
        }
        0x03 => {} // typematic rate: ignored
        0x09 => cpu.regs.set_al(0x00),
        _ => log::debug!("INT 16h AH={ah:02X}: unsupported"),
    }
    HleResult::Done
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bios::install;
    use crate::bus::{FlatBus, linear};
    use crate::cpu::Seg;

    fn setup() -> (Cpu, FlatBus) {
        let mut bus = FlatBus::new();
        install(&mut bus);
        bus.io_out.clear();
        let mut cpu = Cpu::new();
        cpu.sregs.set(Seg::Ss, 0x9000);
        cpu.regs.set_sp(0xFFF0);
        (cpu, bus)
    }

    fn feed(bus: &mut FlatBus, codes: &[u8]) {
        for &c in codes {
            bus.io_in.insert(0x60, c);
            int09(bus);
        }
    }

    fn int16_ax(cpu: &mut Cpu, bus: &mut FlatBus, ax: u16) -> HleResult {
        cpu.regs.set_sp(0xFFF0);
        cpu.regs.set_ax(ax);
        int16(cpu, bus)
    }

    /// Drain the buffer through INT 16h/00 (or /10 if `enhanced`).
    fn drain(cpu: &mut Cpu, bus: &mut FlatBus, enhanced: bool) -> Vec<u16> {
        let ah = if enhanced { 0x1000 } else { 0 };
        std::iter::from_fn(|| (int16_ax(cpu, bus, ah) == HleResult::Done).then(|| cpu.regs.ax()))
            .collect()
    }

    fn keys(codes: &[u8], enhanced: bool) -> Vec<u16> {
        let (mut cpu, mut bus) = setup();
        feed(&mut bus, codes);
        drain(&mut cpu, &mut bus, enhanced)
    }

    #[test]
    fn translation_table() {
        let table: &[(&[u8], u16)] = &[
            (&[0x1E, 0x9E], 0x1E61),                   // a
            (&[0x2A, 0x1E, 0xAA], 0x1E41),             // A
            (&[0x36, 0x03, 0xB6], 0x0340),             // @
            (&[0x3A, 0xBA, 0x1E], 0x1E41),             // caps: A
            (&[0x3A, 0xBA, 0x2A, 0x1E], 0x1E61),       // caps+shift: a
            (&[0x3A, 0xBA, 0x02], 0x0231),             // caps leaves digits
            (&[0x1D, 0x1E, 0x9D], 0x1E01),             // ctrl-A
            (&[0xE0, 0x1D, 0x2E, 0xE0, 0x9D], 0x2E03), // right ctrl-C
            (&[0x38, 0x1E, 0xB8], 0x1E00),             // alt-A
            (&[0x1C], 0x1C0D),                         // Enter
            (&[0x01], 0x011B),                         // Esc
            (&[0x3B], 0x3B00),                         // F1
            (&[0x2A, 0x3B], 0x5400),                   // shift-F1
            (&[0x48], 0x4800),                         // keypad up
            (&[0x45, 0xC5, 0x48], 0x4838),             // numlock: 8
            (&[0x39], 0x3920),                         // space
        ];
        for &(codes, want) in table {
            assert_eq!(keys(codes, false), [want], "{codes:02X?}");
        }
    }

    #[test]
    fn extended_keys() {
        // Grey arrows: 48E0 for enhanced reads, 4800 for standard ones.
        assert_eq!(keys(&[0xE0, 0x48, 0xE0, 0xC8], true), [0x48E0]);
        assert_eq!(keys(&[0xE0, 0x48], false), [0x4800]);
        assert_eq!(
            keys(&[0xE0, 0x2A, 0xE0, 0x4B], true),
            [0x4BE0],
            "fake shift ignored"
        );
        assert_eq!(keys(&[0x38, 0xE0, 0x48], true), [0x9800], "alt-up");
        assert_eq!(keys(&[0xE0, 0x1C], true), [0xE00D], "keypad enter");
        assert_eq!(keys(&[0xE0, 0x35], false), [0x352F], "keypad /");
        assert_eq!(keys(&[0xE0, 0x35], true), [0xE02F]);
        assert_eq!(keys(&[0xE0, 0x1C], false), [0x1C0D]);
        // Enhanced-only keys are dropped by standard reads.
        assert_eq!(keys(&[0x57], true), [0x8500]);
        assert_eq!(keys(&[0x57, 0x1E], false), [0x1E61]);
        assert_eq!(keys(&[0x38, 0x1A], true), [0x1A00]);
        assert_eq!(keys(&[0x38, 0x1A], false), []);
        // Pause sequence produces nothing.
        assert_eq!(
            keys(&[0xE1, 0x1D, 0x45, 0xE1, 0x9D, 0xC5, 0x1E], false),
            [0x1E61]
        );
    }

    #[test]
    fn shift_flags_and_eoi() {
        let (mut cpu, mut bus) = setup();
        feed(&mut bus, &[0x2A, 0x1D, 0x3A, 0xBA]);
        assert_eq!(bus.io_out, [(0x20, 0x20); 4]);
        int16_ax(&mut cpu, &mut bus, 0x0200);
        assert_eq!(cpu.regs.al(), LSHIFT | CTRL | CAPS);
        int16_ax(&mut cpu, &mut bus, 0x1200);
        assert_eq!(cpu.regs.ah(), LCTRL_DOWN);
        assert_eq!(bda::read8(&mut bus, bda::KBD_FLAGS4), 0x04, "caps LED");
        feed(&mut bus, &[0xAA, 0x9D]);
        int16_ax(&mut cpu, &mut bus, 0x0200);
        assert_eq!(cpu.regs.al(), CAPS);
        // Insert toggles.
        feed(&mut bus, &[0xE0, 0x52, 0xE0, 0xD2]);
        int16_ax(&mut cpu, &mut bus, 0x0200);
        assert_eq!(cpu.regs.al(), CAPS | INSERT);
    }

    #[test]
    fn buffer_holds_15_keys() {
        let (mut cpu, mut bus) = setup();
        for _ in 0..20 {
            feed(&mut bus, &[0x1E, 0x9E]);
        }
        assert_eq!(drain(&mut cpu, &mut bus, false).len(), 15);
        // The ring wraps.
        feed(&mut bus, &[0x30]);
        assert_eq!(drain(&mut cpu, &mut bus, false), [0x3062]);
        cpu.regs.set_cx(0x2C7A);
        int16_ax(&mut cpu, &mut bus, 0x0500);
        assert_eq!(cpu.regs.al(), 0);
        assert_eq!(drain(&mut cpu, &mut bus, true), [0x2C7A]);
    }

    #[test]
    fn int16_get_retries_then_returns() {
        let (mut cpu, mut bus) = setup();
        assert_eq!(int16_ax(&mut cpu, &mut bus, 0x0000), HleResult::Retry);
        assert_eq!(int16_ax(&mut cpu, &mut bus, 0x1000), HleResult::Retry);
        feed(&mut bus, &[0x1E]);
        assert_eq!(int16_ax(&mut cpu, &mut bus, 0x0000), HleResult::Done);
        assert_eq!(cpu.regs.ax(), 0x1E61);
    }

    #[test]
    fn int16_peek_sets_zf() {
        let (mut cpu, mut bus) = setup();
        let zf = |bus: &mut FlatBus| bus.read16(linear(0x9000, 0xFFF4)) as u32 & flags::ZF != 0;
        int16_ax(&mut cpu, &mut bus, 0x0100);
        assert!(zf(&mut bus));
        feed(&mut bus, &[0x1F]);
        int16_ax(&mut cpu, &mut bus, 0x1100);
        assert!(!zf(&mut bus));
        assert_eq!(cpu.regs.ax(), 0x1F73);
        assert_eq!(
            drain(&mut cpu, &mut bus, false),
            [0x1F73],
            "peek keeps the key"
        );
    }
}
