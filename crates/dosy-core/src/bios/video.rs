//! INT 10h video BIOS. Everything goes through `Bus` (VGA ports and the
//! memory window), so it works on any bus; tests use the real VGA.
//!
//! Mode sets are table driven: [`mode_params`] maps a mode number to its
//! register set; graphics modes (026/027) add entries there.

use super::{ROM_SEGMENT, bda, set_vector};
use crate::bus::{Addr, Bus, linear};
use crate::cpu::{Cpu, Seg};
use crate::hw::vga::{FONT_STRIDE, bank_offset, font};

/// ROM font and table locations (`F000:xxxx`).
pub const ROM_FONT_8X16: u16 = 0xC000;
pub const ROM_FONT_8X8: u16 = 0xD000;
/// Upper 128 characters of the 8x8 font (INT 1Fh).
pub const ROM_FONT_8X8_HI: u16 = ROM_FONT_8X8 + 128 * 8;
/// IBM-compatible 6845 video parameter table (INT 1Dh).
pub const VIDEO_PARAMS: u16 = 0xF0A4;
/// INT 10h/1Bh static functionality table.
pub const FUNC_TABLE: u16 = 0xF100;

/// How a mode's DAC is initialised.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DacInit {
    /// EGA 64-colour palette in entries 0–63.
    Ega,
    /// Monochrome grey levels.
    Mono,
    /// Default 256-colour palette (mode 13h).
    Vga256,
}

/// Register set and BDA values for one BIOS mode.
#[derive(Clone, Copy, Debug)]
pub struct ModeParams {
    pub text: bool,
    pub cols: u16,
    pub rows: u8,
    pub char_height: u8,
    pub page_size: u16,
    /// BDA 65h (CGA mode control image).
    pub mode_ctrl: u8,
    pub crtc_base: u16,
    pub misc: u8,
    /// Sequencer registers 1–4.
    pub seq: [u8; 4],
    pub crtc: [u8; 25],
    /// Attribute controller registers 0–13h.
    pub ac: [u8; 20],
    pub gc: [u8; 9],
    pub dac: DacInit,
}

impl ModeParams {
    /// Physical address of the mode's memory.
    pub fn mem_base(&self) -> Addr {
        match (self.gc[6] >> 2) & 3 {
            0 | 1 => 0xA_0000,
            2 => 0xB_0000,
            _ => 0xB_8000,
        }
    }
}

const AC_COLOR_TEXT: [u8; 20] = [
    0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x14, 0x07, 0x38, 0x39, 0x3A, 0x3B, 0x3C, 0x3D, 0x3E, 0x3F,
    0x0C, 0x00, 0x0F, 0x08,
];
const GC_COLOR_TEXT: [u8; 9] = [0x00, 0x00, 0x00, 0x00, 0x00, 0x10, 0x0E, 0x00, 0xFF];

const MODE_40: ModeParams = ModeParams {
    text: true,
    cols: 40,
    rows: 25,
    char_height: 16,
    page_size: 0x0800,
    mode_ctrl: 0x28,
    crtc_base: 0x3D4,
    misc: 0x67,
    seq: [0x08, 0x03, 0x00, 0x02],
    crtc: [
        0x2D, 0x27, 0x28, 0x90, 0x2B, 0xA0, 0xBF, 0x1F, 0x00, 0x4F, 0x0D, 0x0E, 0x00, 0x00, 0x00,
        0x00, 0x9C, 0x8E, 0x8F, 0x14, 0x1F, 0x96, 0xB9, 0xA3, 0xFF,
    ],
    ac: AC_COLOR_TEXT,
    gc: GC_COLOR_TEXT,
    dac: DacInit::Ega,
};

const MODE_80: ModeParams = ModeParams {
    cols: 80,
    page_size: 0x1000,
    mode_ctrl: 0x29,
    seq: [0x00, 0x03, 0x00, 0x02],
    crtc: [
        0x5F, 0x4F, 0x50, 0x82, 0x55, 0x81, 0xBF, 0x1F, 0x00, 0x4F, 0x0D, 0x0E, 0x00, 0x00, 0x00,
        0x00, 0x9C, 0x8E, 0x8F, 0x28, 0x1F, 0x96, 0xB9, 0xA3, 0xFF,
    ],
    ..MODE_40
};

const MODE_7: ModeParams = ModeParams {
    crtc_base: 0x3B4,
    misc: 0x66,
    crtc: [
        0x5F, 0x4F, 0x50, 0x82, 0x55, 0x81, 0xBF, 0x1F, 0x00, 0x4F, 0x0D, 0x0E, 0x00, 0x00, 0x00,
        0x00, 0x9C, 0x8E, 0x8F, 0x28, 0x0F, 0x96, 0xB9, 0xA3, 0xFF,
    ],
    ac: [
        0x00, 0x08, 0x08, 0x08, 0x08, 0x08, 0x08, 0x08, 0x10, 0x18, 0x18, 0x18, 0x18, 0x18, 0x18,
        0x18, 0x0E, 0x00, 0x0F, 0x08,
    ],
    gc: [0x00, 0x00, 0x00, 0x00, 0x00, 0x10, 0x0A, 0x00, 0xFF],
    dac: DacInit::Mono,
    ..MODE_80
};

const MODE_13: ModeParams = ModeParams {
    text: false,
    cols: 40,
    rows: 25,
    char_height: 8,
    page_size: 0xFA00,
    mode_ctrl: 0x09,
    crtc_base: 0x3D4,
    misc: 0x63,
    seq: [0x01, 0x0F, 0x00, 0x0E],
    crtc: [
        0x5F, 0x4F, 0x50, 0x82, 0x54, 0x80, 0xBF, 0x1F, 0x00, 0x41, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x9C, 0x8E, 0x8F, 0x28, 0x40, 0x96, 0xB9, 0xA3, 0xFF,
    ],
    ac: [
        0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0A, 0x0B, 0x0C, 0x0D, 0x0E,
        0x0F, 0x41, 0x00, 0x0F, 0x00,
    ],
    gc: [0x00, 0x00, 0x00, 0x00, 0x00, 0x40, 0x05, 0x00, 0xFF],
    dac: DacInit::Vga256,
};

/// Register set for a BIOS mode; `None` = unsupported.
pub fn mode_params(mode: u8) -> Option<&'static ModeParams> {
    Some(match mode {
        0x00 | 0x01 => &MODE_40,
        0x02 | 0x03 => &MODE_80,
        0x07 => &MODE_7,
        0x13 => &MODE_13,
        _ => return None,
    })
}

// --- Port helpers ---

fn crtc_port<B: Bus>(bus: &mut B) -> u16 {
    match bda::read16(bus, bda::CRTC_BASE) {
        0x3B4 => 0x3B4,
        _ => 0x3D4,
    }
}

fn crtc_write<B: Bus>(bus: &mut B, i: u8, v: u8) {
    let p = crtc_port(bus);
    bus.io_out8(p, i);
    bus.io_out8(p + 1, v);
}

fn crtc_read<B: Bus>(bus: &mut B, i: u8) -> u8 {
    let p = crtc_port(bus);
    bus.io_out8(p, i);
    bus.io_in8(p + 1)
}

fn seq_write<B: Bus>(bus: &mut B, i: u8, v: u8) {
    bus.io_out8(0x3C4, i);
    bus.io_out8(0x3C5, v);
}

fn gc_write<B: Bus>(bus: &mut B, i: u8, v: u8) {
    bus.io_out8(0x3CE, i);
    bus.io_out8(0x3CF, v);
}

/// Attribute register write, leaving the palette address source on.
fn ac_write<B: Bus>(bus: &mut B, i: u8, v: u8) {
    reset_ac_flip_flop(bus);
    bus.io_out8(0x3C0, i | 0x20);
    bus.io_out8(0x3C0, v);
}

fn ac_read<B: Bus>(bus: &mut B, i: u8) -> u8 {
    reset_ac_flip_flop(bus);
    bus.io_out8(0x3C0, i | 0x20);
    let v = bus.io_in8(0x3C1);
    reset_ac_flip_flop(bus);
    v
}

/// Read input status 1 (3BAh or 3DAh, per the current CRTC base).
fn reset_ac_flip_flop<B: Bus>(bus: &mut B) {
    let p = crtc_port(bus) + 6;
    bus.io_in8(p);
}

fn dac_write<B: Bus>(bus: &mut B, i: u8, rgb: [u8; 3]) {
    bus.io_out8(0x3C8, i);
    for c in rgb {
        bus.io_out8(0x3C9, c);
    }
}

fn dac_read<B: Bus>(bus: &mut B, i: u8) -> [u8; 3] {
    bus.io_out8(0x3C7, i);
    [bus.io_in8(0x3C9), bus.io_in8(0x3C9), bus.io_in8(0x3C9)]
}

// --- Palettes ---

fn ega_color(i: u8) -> [u8; 3] {
    let c = |hi: u8, lo: u8| 0x2A * ((i >> hi) & 1) + 0x15 * ((i >> lo) & 1);
    [c(2, 5), c(1, 4), c(0, 3)]
}

/// The default VGA 256-colour palette: 16 EGA colours, a 16-step grey ramp,
/// 9 hue rings of 24 (3 values x 3 saturations), then black.
pub fn vga256_color(i: u8) -> [u8; 3] {
    const GREYS: [u8; 16] = [
        0x00, 0x05, 0x08, 0x0B, 0x0E, 0x11, 0x14, 0x18, 0x1C, 0x20, 0x24, 0x28, 0x2D, 0x32, 0x38,
        0x3F,
    ];
    // [lo, s1, s2, s3, hi] per value/saturation block.
    const RINGS: [[u8; 5]; 9] = [
        [0x00, 0x10, 0x1F, 0x2F, 0x3F],
        [0x1F, 0x27, 0x2F, 0x37, 0x3F],
        [0x2D, 0x31, 0x36, 0x3A, 0x3F],
        [0x00, 0x07, 0x0E, 0x15, 0x1C],
        [0x0E, 0x11, 0x15, 0x18, 0x1C],
        [0x14, 0x16, 0x18, 0x1A, 0x1C],
        [0x00, 0x04, 0x08, 0x0C, 0x10],
        [0x08, 0x0A, 0x0C, 0x0E, 0x10],
        [0x0B, 0x0C, 0x0D, 0x0F, 0x10],
    ];
    match i {
        0..=15 => ega_color(
            [
                0, 1, 2, 3, 4, 5, 0x14, 7, 0x38, 0x39, 0x3A, 0x3B, 0x3C, 0x3D, 0x3E, 0x3F,
            ][i as usize],
        ),
        16..=31 => [GREYS[i as usize - 16]; 3],
        32..=247 => {
            let v = RINGS[(i as usize - 32) / 24];
            let k = (i as usize - 32) % 24;
            let (lo, hi) = (v[0], v[4]);
            let up = |n: usize| v[n]; // lo, s1, s2, s3
            let down = |n: usize| v[4 - n]; // hi, s3, s2, s1
            match k / 4 {
                0 => [up(k % 4), lo, hi],   // R up, B high
                1 => [hi, lo, down(k % 4)], // B down
                2 => [hi, up(k % 4), lo],   // G up
                3 => [down(k % 4), hi, lo], // R down
                4 => [lo, hi, up(k % 4)],   // B up
                _ => [lo, down(k % 4), hi], // G down
            }
        }
        _ => [0; 3],
    }
}

fn load_dac<B: Bus>(bus: &mut B, init: DacInit) {
    bus.io_out8(0x3C6, 0xFF);
    bus.io_out8(0x3C8, 0);
    for i in 0..=255u8 {
        let rgb = match init {
            DacInit::Ega if i < 64 => ega_color(i),
            DacInit::Mono if i < 64 => [0x2A * ((i >> 3) & 1) + 0x15 * ((i >> 4) & 1); 3],
            DacInit::Vga256 => vga256_color(i),
            _ => [0; 3],
        };
        for c in rgb {
            bus.io_out8(0x3C9, c);
        }
    }
}

// --- Fonts ---

/// Load `count` glyphs of `height` rows into character map `block`,
/// starting at `first`, via plane 2 (as the IBM BIOS does).
pub fn load_font<B: Bus>(bus: &mut B, glyphs: &[u8], height: usize, block: u8, first: u8) {
    seq_write(bus, 2, 0x04);
    seq_write(bus, 4, 0x07);
    gc_write(bus, 4, 0x02);
    gc_write(bus, 5, 0x00);
    gc_write(bus, 6, 0x04);
    let base = 0xA_0000 + bank_offset(block as usize & 7) as Addr;
    for (i, g) in glyphs.chunks(height).enumerate() {
        let c = first as usize + i;
        if c > 255 {
            break;
        }
        for (r, &b) in g.iter().enumerate() {
            bus.write8(base + (c * FONT_STRIDE + r) as Addr, b);
        }
    }
    seq_write(bus, 2, 0x03);
    seq_write(bus, 4, 0x02);
    gc_write(bus, 4, 0x00);
    gc_write(bus, 5, 0x10);
    let mono = bda::read8(bus, bda::VIDEO_MODE) == 7;
    gc_write(bus, 6, if mono { 0x0A } else { 0x0E });
}

/// ROM fonts, the video parameter table and vectors 10h/1Dh/1Fh/43h.
pub fn install<B: Bus>(bus: &mut B) {
    let rom = |off: u16| linear(ROM_SEGMENT, off);
    bus.load_rom(rom(ROM_FONT_8X16), &font::FONT_8X16);
    bus.load_rom(rom(ROM_FONT_8X8), &font::FONT_8X8);
    bus.load_rom(rom(VIDEO_PARAMS), &VIDEO_PARAM_TABLE);
    // 1Bh functionality: modes 00–07 and 13h, 200/350/400 scan lines.
    let mut func = [0u8; 16];
    func[..4].copy_from_slice(&[0xFF, 0x00, 0x08, 0x00]); // modes 0-7, 13h
    func[7] = 0x07;
    bus.load_rom(rom(FUNC_TABLE), &func);
    set_vector(bus, 0x1D, ROM_SEGMENT, VIDEO_PARAMS);
    set_vector(bus, 0x1F, ROM_SEGMENT, ROM_FONT_8X8_HI);
    set_vector(bus, 0x43, ROM_SEGMENT, ROM_FONT_8X8);
    set_mode(bus, 0x03);
}

/// IBM PC 6845 parameter table: 4 x 16 CRTC sets, page sizes, columns,
/// mode control bytes.
#[rustfmt::skip]
const VIDEO_PARAM_TABLE: [u8; 88] = [
    0x38, 0x28, 0x2D, 0x0A, 0x1F, 0x06, 0x19, 0x1C, 0x02, 0x07, 0x06, 0x07, 0, 0, 0, 0,
    0x71, 0x50, 0x5A, 0x0A, 0x1F, 0x06, 0x19, 0x1C, 0x02, 0x07, 0x06, 0x07, 0, 0, 0, 0,
    0x38, 0x28, 0x2D, 0x0A, 0x7F, 0x06, 0x64, 0x70, 0x02, 0x01, 0x06, 0x07, 0, 0, 0, 0,
    0x61, 0x50, 0x52, 0x0F, 0x19, 0x06, 0x19, 0x19, 0x02, 0x0D, 0x0B, 0x0C, 0, 0, 0, 0,
    0x00, 0x08, 0x00, 0x10, 0x00, 0x40, 0x00, 0x40,
    0x28, 0x28, 0x50, 0x50, 0x28, 0x28, 0x50, 0x50,
    0x2C, 0x28, 0x2D, 0x29, 0x2A, 0x2E, 0x1E, 0x29,
];

// --- Mode set ---

/// AH=00h. Bit 7 of `mode` keeps video memory. Unsupported modes are
/// ignored (logged), as real BIOSes do.
pub fn set_mode<B: Bus>(bus: &mut B, mode: u8) {
    let keep = mode & 0x80 != 0;
    let m = mode & 0x7F;
    let Some(p) = mode_params(m) else {
        log::warn!("INT 10h: unsupported video mode {m:02X}h");
        return;
    };

    // Sequencer (held in reset), misc output, CRTC, GC, AC.
    seq_write(bus, 0, 0x01);
    for (i, &v) in p.seq.iter().enumerate() {
        seq_write(bus, i as u8 + 1, v);
    }
    bus.io_out8(0x3C2, p.misc);
    seq_write(bus, 0, 0x03);
    bda::write16(bus, bda::CRTC_BASE, p.crtc_base);
    crtc_write(bus, 0x11, p.crtc[0x11] & 0x7F);
    for (i, &v) in p.crtc.iter().enumerate() {
        crtc_write(bus, i as u8, v);
    }
    for (i, &v) in p.gc.iter().enumerate() {
        gc_write(bus, i as u8, v);
    }
    for (i, &v) in p.ac.iter().enumerate() {
        ac_write(bus, i as u8, v);
    }
    ac_write(bus, 0x14, 0);
    load_dac(bus, p.dac);

    // BDA.
    bda::write8(bus, bda::VIDEO_MODE, m);
    bda::write16(bus, bda::VIDEO_COLS, p.cols);
    bda::write16(bus, bda::VIDEO_PAGE_SIZE, p.page_size);
    bda::write16(bus, bda::VIDEO_PAGE_OFFSET, 0);
    for page in 0..8 {
        bda::write16(bus, bda::CURSOR_POS + 2 * page, 0);
    }
    bda::write16(bus, bda::CURSOR_SHAPE, 0x0607);
    bda::write8(bus, bda::VIDEO_PAGE, 0);
    bda::write8(bus, bda::VIDEO_MODE_CTRL, p.mode_ctrl);
    bda::write8(bus, bda::VIDEO_PALETTE, 0x30);
    bda::write8(bus, bda::VIDEO_ROWS, p.rows - 1);
    bda::write8(bus, bda::CHAR_HEIGHT, p.char_height);
    bda::write8(bus, bda::VIDEO_CTL, 0x60 | (mode & 0x80));
    let equip = bda::read16(bus, bda::EQUIPMENT) & !0x30;
    bda::write16(
        bus,
        bda::EQUIPMENT,
        equip | if m == 7 { 0x30 } else { 0x20 },
    );

    if p.text {
        load_font(bus, &font::FONT_8X16, 16, 0, 0);
        seq_write(bus, 3, 0);
    }
    if !keep {
        let base = p.mem_base();
        if p.text {
            for i in 0..0x4000u32 {
                bus.write16(base + 2 * i, 0x0720);
            }
        } else {
            for i in 0..0x1_0000u32 {
                bus.write8(base + i, 0);
            }
        }
    }
    sync_cursor(bus);
}

// --- Text helpers ---

fn cols<B: Bus>(bus: &mut B) -> u16 {
    bda::read16(bus, bda::VIDEO_COLS).max(1)
}

fn rows<B: Bus>(bus: &mut B) -> u16 {
    bda::read8(bus, bda::VIDEO_ROWS) as u16 + 1
}

fn text_base<B: Bus>(bus: &mut B) -> Addr {
    if bda::read8(bus, bda::VIDEO_MODE) == 7 {
        0xB_0000
    } else {
        0xB_8000
    }
}

fn is_text<B: Bus>(bus: &mut B) -> bool {
    mode_params(bda::read8(bus, bda::VIDEO_MODE)).is_none_or(|p| p.text)
}

fn cell_addr<B: Bus>(bus: &mut B, page: u8, col: u16, row: u16) -> Addr {
    let page_size = bda::read16(bus, bda::VIDEO_PAGE_SIZE) as Addr;
    let base = text_base(bus) + (page as Addr & 7) * page_size;
    base + (row as Addr * cols(bus) as Addr + col as Addr) * 2
}

/// `(col, row)` of the cursor on `page`.
pub fn cursor<B: Bus>(bus: &mut B, page: u8) -> (u16, u16) {
    let [col, row] = bda::read16(bus, bda::CURSOR_POS + 2 * (page as u16 & 7)).to_le_bytes();
    (col as u16, row as u16)
}

pub fn set_cursor<B: Bus>(bus: &mut B, page: u8, col: u16, row: u16) {
    let v = u16::from_le_bytes([col as u8, row as u8]);
    bda::write16(bus, bda::CURSOR_POS + 2 * (page as u16 & 7), v);
    if page & 7 == bda::read8(bus, bda::VIDEO_PAGE) {
        sync_cursor(bus);
    }
}

/// Program CRTC 0Eh/0Fh from the active page's BDA cursor.
fn sync_cursor<B: Bus>(bus: &mut B) {
    let page = bda::read8(bus, bda::VIDEO_PAGE);
    let (col, row) = cursor(bus, page);
    let start = bda::read16(bus, bda::VIDEO_PAGE_OFFSET) / 2;
    let addr = start.wrapping_add(row * cols(bus) + col);
    crtc_write(bus, 0x0E, (addr >> 8) as u8);
    crtc_write(bus, 0x0F, addr as u8);
}

/// AH=01h: CGA-style shape in CH/CL, scaled to the character height
/// (VGA cursor emulation).
fn set_cursor_shape<B: Bus>(bus: &mut B, cx: u16) {
    bda::write16(bus, bda::CURSOR_SHAPE, cx);
    let [end, start] = cx.to_le_bytes();
    if start & 0x60 == 0x20 {
        crtc_write(bus, 0x0A, 0x20);
        return;
    }
    let (mut s, mut e) = ((start & 0x1F) as u16, (end & 0x1F) as u16);
    let h = bda::read8(bus, bda::CHAR_HEIGHT) as u16;
    if h > 8 && s < 8 && e < 8 {
        if e >= s && e - s < 3 && e >= 6 {
            // Underline cursor: keep it at the bottom.
            let len = e - s;
            e = h - 2;
            s = e - len;
        } else {
            s = s * h / 8;
            e = ((e + 1) * h / 8).saturating_sub(1);
        }
    }
    crtc_write(bus, 0x0A, s as u8);
    crtc_write(bus, 0x0B, e as u8);
}

/// AH=05h.
fn set_page<B: Bus>(bus: &mut B, page: u8) {
    let page = page & 7;
    let off = bda::read16(bus, bda::VIDEO_PAGE_SIZE).wrapping_mul(page as u16);
    bda::write8(bus, bda::VIDEO_PAGE, page);
    bda::write16(bus, bda::VIDEO_PAGE_OFFSET, off);
    crtc_write(bus, 0x0C, ((off / 2) >> 8) as u8);
    crtc_write(bus, 0x0D, (off / 2) as u8);
    sync_cursor(bus);
}

/// AH=06h/07h on the active page: scroll the window `(left, top)..=(right,
/// bottom)` up (or down) by `n` lines (0 = clear), filling with `attr`.
pub fn scroll<B: Bus>(bus: &mut B, up: bool, n: u8, attr: u8, tl: (u16, u16), br: (u16, u16)) {
    if !is_text(bus) {
        log::debug!("INT 10h scroll in graphics mode: not implemented (026)");
        return;
    }
    let page = bda::read8(bus, bda::VIDEO_PAGE);
    let (left, top) = tl;
    let right = br.0.min(cols(bus) - 1);
    let bottom = br.1.min(rows(bus) - 1);
    if left > right || top > bottom {
        return;
    }
    let height = bottom - top + 1;
    let n = if n == 0 || n as u16 > height {
        height
    } else {
        n as u16
    };
    let blank = u16::from_le_bytes([b' ', attr]);
    for i in 0..height {
        let row = if up { top + i } else { bottom - i };
        let src = if up { row + n } else { row.wrapping_sub(n) };
        let src_ok = if up { src <= bottom } else { i + n < height };
        for col in left..=right {
            let v = if src_ok {
                let a = cell_addr(bus, page, col, src);
                bus.read16(a)
            } else {
                blank
            };
            let a = cell_addr(bus, page, col, row);
            bus.write16(a, v);
        }
    }
}

/// AH=0Eh teletype output on `page`: BEL, BS, CR, LF, wrap and scroll.
/// `attr` is the foreground for graphics modes; text keeps cell attributes.
pub fn teletype<B: Bus>(bus: &mut B, ch: u8, page: u8, attr: u8) {
    let _ = attr; // graphics-mode glyphs: 026
    let page = page & 7;
    let (cols, rows) = (cols(bus), rows(bus));
    let (mut col, mut row) = cursor(bus, page);
    match ch {
        0x07 => return, // BEL: speaker beep (034)
        0x08 => col = col.saturating_sub(1),
        b'\r' => col = 0,
        b'\n' => row += 1,
        _ => {
            if is_text(bus) {
                let a = cell_addr(bus, page, col, row);
                bus.write8(a, ch);
            }
            col += 1;
            if col >= cols {
                col = 0;
                row += 1;
            }
        }
    }
    if row >= rows {
        row = rows - 1;
        // Scroll with the attribute under the cursor (IBM behaviour).
        let a = cell_addr(bus, page, col, row);
        let fill = bus.read8(a + 1);
        if page == bda::read8(bus, bda::VIDEO_PAGE) {
            scroll(bus, true, 1, fill, (0, 0), (cols - 1, rows - 1));
        }
    }
    set_cursor(bus, page, col, row);
}

/// AH=09h/0Ah: write `ch` (and `attr`) `count` times from the cursor.
fn write_chars<B: Bus>(bus: &mut B, ch: u8, attr: Option<u8>, page: u8, count: u16) {
    if !is_text(bus) {
        log::debug!("INT 10h write char in graphics mode: not implemented (026)");
        return;
    }
    let (col, row) = cursor(bus, page);
    let start = cell_addr(bus, page, col, row);
    let last_row = rows(bus);
    let page_end = cell_addr(bus, page, 0, last_row);
    for i in 0..count as Addr {
        let a = start + 2 * i;
        if a >= page_end {
            break;
        }
        bus.write8(a, ch);
        if let Some(attr) = attr {
            bus.write8(a + 1, attr);
        }
    }
}

// --- INT 10h ---

pub fn int10<B: Bus>(cpu: &mut Cpu, bus: &mut B) {
    let r = cpu.regs;
    let (ah, al, bh, bl) = (r.ah(), r.al(), r.bh(), r.bl());
    match ah {
        0x00 => set_mode(bus, al),
        0x01 => set_cursor_shape(bus, r.cx()),
        0x02 => set_cursor(bus, bh, r.dl() as u16, r.dh() as u16),
        0x03 => {
            let (col, row) = cursor(bus, bh);
            cpu.regs.set_dx(u16::from_le_bytes([col as u8, row as u8]));
            cpu.regs.set_cx(bda::read16(bus, bda::CURSOR_SHAPE));
        }
        0x05 => set_page(bus, al),
        0x06 | 0x07 => {
            let tl = (r.cl() as u16, r.ch() as u16);
            let br = (r.dl() as u16, r.dh() as u16);
            scroll(bus, ah == 0x06, al, bh, tl, br);
        }
        0x08 => {
            let (col, row) = cursor(bus, bh);
            let a = cell_addr(bus, bh, col, row);
            cpu.regs.set_ax(bus.read16(a));
        }
        0x09 => write_chars(bus, al, Some(bl), bh, r.cx()),
        0x0A => write_chars(bus, al, None, bh, r.cx()),
        0x0E => {
            let page = bda::read8(bus, bda::VIDEO_PAGE);
            teletype(bus, al, page, bl);
        }
        0x0F => {
            let mode = bda::read8(bus, bda::VIDEO_MODE) | (bda::read8(bus, bda::VIDEO_CTL) & 0x80);
            cpu.regs.set_al(mode);
            cpu.regs.set_ah(cols(bus) as u8);
            cpu.regs.set_bh(bda::read8(bus, bda::VIDEO_PAGE));
        }
        0x10 => palette(cpu, bus),
        0x11 => font_fn(cpu, bus),
        0x12 if bl == 0x10 => {
            cpu.regs.set_bx(0x0003); // colour, 256 KiB
            cpu.regs
                .set_cx(bda::read8(bus, bda::VIDEO_SWITCHES) as u16 & 0x0F);
        }
        0x1A if al <= 1 => {
            if al == 0 {
                cpu.regs.set_bx(0x0008); // VGA + analog colour, no secondary
            }
            cpu.regs.set_al(0x1A);
        }
        0x1B if bl == 0 => state_info(cpu, bus),
        _ => log::debug!("INT 10h AX={:04X} BX={:04X}: unsupported", r.ax(), r.bx()),
    }
}

/// AH=10h palette and DAC functions.
fn palette<B: Bus>(cpu: &mut Cpu, bus: &mut B) {
    let r = cpu.regs;
    let es = cpu.sregs.get(Seg::Es);
    let table = |i: u16| linear(es, r.dx().wrapping_add(i));
    match r.al() {
        0x00 => ac_write(bus, r.bl() & 0x1F, r.bh()),
        0x01 => ac_write(bus, 0x11, r.bh()),
        0x02 => {
            for i in 0..17u16 {
                let v = bus.read8(table(i));
                ac_write(bus, if i == 16 { 0x11 } else { i as u8 }, v);
            }
        }
        0x03 => {
            let m = ac_read(bus, 0x10);
            ac_write(bus, 0x10, if r.bl() & 1 != 0 { m | 8 } else { m & !8 });
        }
        0x07 => {
            let v = ac_read(bus, r.bl() & 0x1F);
            cpu.regs.set_bh(v);
        }
        0x08 => {
            let v = ac_read(bus, 0x11);
            cpu.regs.set_bh(v);
        }
        0x09 => {
            for i in 0..17u16 {
                let v = ac_read(bus, if i == 16 { 0x11 } else { i as u8 });
                bus.write8(table(i), v);
            }
        }
        0x10 => dac_write(bus, r.bl(), [r.dh(), r.ch(), r.cl()]),
        0x12 => {
            for i in 0..r.cx() {
                let c = [0, 1, 2].map(|k| bus.read8(table(i * 3 + k)));
                dac_write(bus, r.bx().wrapping_add(i) as u8, c);
            }
        }
        0x15 => {
            let [red, g, b] = dac_read(bus, r.bl());
            cpu.regs.set_dh(red);
            cpu.regs.set_ch(g);
            cpu.regs.set_cl(b);
        }
        0x17 => {
            for i in 0..r.cx() {
                let c = dac_read(bus, r.bx().wrapping_add(i) as u8);
                for (k, v) in c.into_iter().enumerate() {
                    bus.write8(table(i * 3 + k as u16), v);
                }
            }
        }
        0x1A => cpu.regs.set_bx(0),
        al => log::debug!("INT 10h/10h AL={al:02X}: unsupported"),
    }
}

/// AH=11h character generator: ROM/user font loads and AL=30h info.
fn font_fn<B: Bus>(cpu: &mut Cpu, bus: &mut B) {
    let r = cpu.regs;
    let al = r.al();
    match al {
        0x00 | 0x10 => {
            let h = r.bh().max(1) as usize;
            let src = linear(cpu.sregs.get(Seg::Es), r.bp());
            let glyphs: Vec<u8> = (0..r.cx() as Addr * h as Addr)
                .map(|i| bus.read8(src + i))
                .collect();
            load_font(bus, &glyphs, h, r.bl(), r.dl());
            if al == 0x10 {
                set_char_height(bus, h as u8);
            }
        }
        0x01 | 0x11 | 0x04 | 0x14 => {
            // No 8x14 font: both load the 8x16 one.
            load_font(bus, &font::FONT_8X16, 16, r.bl(), 0);
            if al & 0x10 != 0 {
                set_char_height(bus, 16);
            }
        }
        0x02 | 0x12 => {
            load_font(bus, &font::FONT_8X8, 8, r.bl(), 0);
            if al == 0x12 {
                set_char_height(bus, 8);
            }
        }
        0x03 => seq_write(bus, 3, r.bl()),
        0x30 => {
            let off = match r.bh() {
                0x00 => bus.read16(0x1F * 4),
                0x01 => bus.read16(0x43 * 4),
                0x03 => ROM_FONT_8X8,
                0x04 => ROM_FONT_8X8_HI,
                _ => ROM_FONT_8X16, // 8x14 / 8x16 / 9-dot alternates
            };
            let seg = match r.bh() {
                0x00 => bus.read16(0x1F * 4 + 2),
                0x01 => bus.read16(0x43 * 4 + 2),
                _ => ROM_SEGMENT,
            };
            cpu.sregs.set(Seg::Es, seg);
            cpu.regs.set_bp(off);
            cpu.regs.set_cx(bda::read8(bus, bda::CHAR_HEIGHT) as u16);
            cpu.regs.set_dl(bda::read8(bus, bda::VIDEO_ROWS));
        }
        _ => log::debug!("INT 10h/11h AL={al:02X}: unsupported"),
    }
}

/// After a 11h/1xh font load: character height, rows and cursor.
fn set_char_height<B: Bus>(bus: &mut B, h: u8) {
    let max = crtc_read(bus, 0x09);
    crtc_write(bus, 0x09, (max & 0xE0) | (h - 1));
    let lines = 400u16;
    let rows = (lines / h as u16) as u8;
    bda::write8(bus, bda::CHAR_HEIGHT, h);
    bda::write8(bus, bda::VIDEO_ROWS, rows - 1);
    let page = (rows as u16 * cols(bus) * 2).next_multiple_of(0x100);
    bda::write16(bus, bda::VIDEO_PAGE_SIZE, page);
    crtc_write(bus, 0x0A, h - 2);
    crtc_write(bus, 0x0B, h - 1);
}

/// AH=1Bh: 64-byte state block at ES:DI.
fn state_info<B: Bus>(cpu: &mut Cpu, bus: &mut B) {
    let es = cpu.sregs.get(Seg::Es);
    let di = cpu.regs.di();
    let mut b = [0u8; 64];
    b[0..2].copy_from_slice(&FUNC_TABLE.to_le_bytes());
    b[2..4].copy_from_slice(&ROM_SEGMENT.to_le_bytes());
    b[4] = bda::read8(bus, bda::VIDEO_MODE);
    b[5..7].copy_from_slice(&bda::read16(bus, bda::VIDEO_COLS).to_le_bytes());
    b[7..9].copy_from_slice(&bda::read16(bus, bda::VIDEO_PAGE_SIZE).to_le_bytes());
    b[9..11].copy_from_slice(&bda::read16(bus, bda::VIDEO_PAGE_OFFSET).to_le_bytes());
    for i in 0..16u16 {
        b[11 + i as usize] = bda::read8(bus, bda::CURSOR_POS + i);
    }
    b[27..29].copy_from_slice(&bda::read16(bus, bda::CURSOR_SHAPE).to_le_bytes());
    b[29] = bda::read8(bus, bda::VIDEO_PAGE);
    b[30..32].copy_from_slice(&bda::read16(bus, bda::CRTC_BASE).to_le_bytes());
    b[32] = bda::read8(bus, bda::VIDEO_MODE_CTRL);
    b[33] = bda::read8(bus, bda::VIDEO_PALETTE);
    b[34] = bda::read8(bus, bda::VIDEO_ROWS) + 1;
    b[35] = bda::read8(bus, bda::CHAR_HEIGHT);
    b[37] = 0x08; // active display combination: VGA colour
    b[39] = 0x10; // colours: 16 (text) — 256 in 13h
    if bda::read8(bus, bda::VIDEO_MODE) == 0x13 {
        b[39] = 0x00;
        b[40] = 0x01;
    }
    b[41] = 8; // pages
    b[42] = 2; // 400 scan lines
    for (i, &v) in b.iter().enumerate() {
        bus.write8(linear(es, di.wrapping_add(i as u16)), v);
    }
    cpu.regs.set_al(0x1B);
}

#[cfg(test)]
mod tests;
