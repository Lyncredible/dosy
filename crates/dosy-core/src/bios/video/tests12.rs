//! Mode 12h: planar write/read modes, latches, 16-colour rendering and the
//! BIOS pixel/text functions.

use super::*;
use crate::bios;
use crate::hw::Hardware;

const PLANE: usize = 0x10000;

fn hw12() -> Hardware {
    let mut hw = Hardware::new();
    bios::install(&mut hw);
    set_mode(&mut hw, 0x12);
    hw
}

fn gc(hw: &mut Hardware, i: u8, v: u8) {
    hw.io_out8(0x3CE, i);
    hw.io_out8(0x3CF, v);
}

fn planes(hw: &Hardware, off: usize) -> [u8; 4] {
    std::array::from_fn(|p| hw.vga.vram[p * PLANE + off])
}

#[test]
fn mode_set_gives_640x480_16_colour_frame() {
    let mut hw = hw12();
    assert_eq!(bda::read8(&mut hw, bda::VIDEO_MODE), 0x12);
    let f = hw.vga.render();
    assert_eq!((f.width, f.height), (640, 480));
    assert!(f.pixels.iter().all(|&p| p == 0));
}

#[test]
fn write_mode_0_set_reset_and_bit_mask() {
    let mut hw = hw12();
    gc(&mut hw, 1, 0x0F); // enable set/reset on all planes
    gc(&mut hw, 0, 0x05); // colour 5: planes 0 and 2
    gc(&mut hw, 8, 0xF0);
    hw.read8(0xA0000); // latches = 0
    hw.write8(0xA0000, 0x00);
    assert_eq!(planes(&hw, 0), [0xF0, 0x00, 0xF0, 0x00]);
    // Without set/reset the (rotated) CPU byte goes to every plane.
    gc(&mut hw, 1, 0x00);
    gc(&mut hw, 8, 0xFF);
    gc(&mut hw, 3, 0x01); // rotate right 1
    hw.write8(0xA0001, 0x81);
    assert_eq!(planes(&hw, 1), [0xC0; 4]);
}

#[test]
fn write_mode_1_copies_latches() {
    let mut hw = hw12();
    for (p, v) in [0x11u8, 0x22, 0x44, 0x88].iter().enumerate() {
        hw.vga.vram[p * PLANE + 10] = *v;
    }
    gc(&mut hw, 5, 0x01);
    hw.read8(0xA000A);
    hw.write8(0xA0020, 0xFF);
    assert_eq!(planes(&hw, 0x20), [0x11, 0x22, 0x44, 0x88]);
}

#[test]
fn write_modes_2_and_3_and_xor() {
    let mut hw = hw12();
    // Mode 2: CPU bits 0..3 are the colour, bit mask selects pixels.
    gc(&mut hw, 5, 0x02);
    gc(&mut hw, 8, 0x0F);
    hw.read8(0xA0000);
    hw.write8(0xA0000, 0x0A);
    assert_eq!(planes(&hw, 0), [0x00, 0x0F, 0x00, 0x0F]);
    // XOR with the latches flips those pixels back.
    gc(&mut hw, 3, 0x18);
    hw.read8(0xA0000);
    hw.write8(0xA0000, 0x0A);
    assert_eq!(planes(&hw, 0), [0; 4]);
    gc(&mut hw, 3, 0x00);
    // Mode 3: set/reset colour, CPU byte (rotated) ANDed into the mask.
    gc(&mut hw, 5, 0x03);
    gc(&mut hw, 0, 0x0C);
    gc(&mut hw, 8, 0xFF);
    hw.read8(0xA0001);
    hw.write8(0xA0001, 0x3C);
    assert_eq!(planes(&hw, 1), [0x00, 0x00, 0x3C, 0x3C]);
}

#[test]
fn read_modes() {
    let mut hw = hw12();
    hw.vga.vram[5] = 0xFF; // plane 0
    hw.vga.vram[2 * PLANE + 5] = 0x0F; // plane 2
    // Read mode 0: read map select.
    gc(&mut hw, 4, 2);
    assert_eq!(hw.read8(0xA0005), 0x0F);
    // Read mode 1: pixels equal to colour 5 (planes 0+2) -> low nibble.
    gc(&mut hw, 5, 0x08);
    gc(&mut hw, 2, 0x05);
    gc(&mut hw, 7, 0x0F);
    assert_eq!(hw.read8(0xA0005), 0x0F);
    // Colour don't care: ignore plane 2 -> every pixel with plane 0 set.
    gc(&mut hw, 7, 0x0B);
    assert_eq!(hw.read8(0xA0005), 0xFF);
}

#[test]
fn bios_pixels_and_rendering() {
    let mut hw = hw12();
    put_pixel_planar(&mut hw, 3, 5, 0x0C);
    put_pixel_planar(&mut hw, 639, 479, 0x01);
    assert_eq!(get_pixel_planar(&mut hw, 3, 5), 0x0C);
    assert_eq!(get_pixel_planar(&mut hw, 4, 5), 0);
    assert_eq!(get_pixel_planar(&mut hw, 639, 479), 0x01);
    let f = hw.vga.render();
    // Attribute 0Ch maps to DAC 3Ch in the default palette.
    assert_eq!(f.pixels[5 * 640 + 3], 0x3C);
    assert_eq!(f.pixels[5 * 640 + 4], 0);
    assert_eq!(f.pixels[479 * 640 + 639], 0x01);
    // Bit 7 XORs the pixel.
    put_pixel_planar(&mut hw, 3, 5, 0x8C);
    assert_eq!(get_pixel_planar(&mut hw, 3, 5), 0);
    // The GC is left in its default state.
    assert_eq!(hw.vga.gc, MODE_12.gc);
}

#[test]
fn teletype_draws_glyphs_and_scrolls() {
    let mut hw = hw12();
    teletype(&mut hw, b'H', 0, 0x0E);
    let lit = |hw: &mut Hardware, row: u16| {
        let mut n = 0;
        for y in row * 16..row * 16 + 16 {
            for x in 0..8 {
                if get_pixel_planar(hw, x, y) == 0x0E {
                    n += 1;
                }
            }
        }
        n
    };
    let first = lit(&mut hw, 0);
    assert!(first > 10, "glyph drawn: {first}");
    // Scroll up one text row: the glyph moves off row 0.
    scroll(&mut hw, true, 1, 0, (0, 0), (79, 29));
    assert_eq!(lit(&mut hw, 0), 0);
    // Scroll down: it comes back (copied via latches).
    teletype(&mut hw, b'\r', 0, 0);
    set_cursor(&mut hw, 0, 0, 1);
    teletype(&mut hw, b'H', 0, 0x0E);
    scroll(&mut hw, false, 1, 0, (0, 0), (79, 29));
    assert_eq!(lit(&mut hw, 2), first);
}
