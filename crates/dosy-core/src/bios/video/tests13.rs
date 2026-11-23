//! Mode 13h: chain-4 framebuffer, 256-colour rendering, DAC, BIOS pixels.

use super::*;
use crate::bios;
use crate::hw::Hardware;
use crate::video::Rgb8;

fn hw13() -> Hardware {
    let mut hw = Hardware::new();
    bios::install(&mut hw);
    set_mode(&mut hw, 0x13);
    hw
}

fn fnv(data: &[u8]) -> u64 {
    data.iter().fold(0xcbf2_9ce4_8422_2325, |h, &b| {
        (h ^ b as u64).wrapping_mul(0x100_0000_01b3)
    })
}

fn crtc(hw: &mut Hardware, i: u8, v: u8) {
    hw.io_out8(0x3D4, i);
    hw.io_out8(0x3D5, v);
}

#[test]
fn linear_framebuffer_pixels_appear_in_frame() {
    let mut hw = hw13();
    let pattern = |x: u32, y: u32| (x ^ (y * 3)) as u8;
    for y in 0..200 {
        for x in 0..320 {
            hw.write8(0xA0000 + y * 320 + x, pattern(x, y));
        }
    }
    assert_eq!(hw.read8(0xA0000 + 7 * 320 + 9), pattern(9, 7));
    // Chain-4: pixel n lives in plane n & 3 at offset n & !3.
    assert_eq!(hw.vga.vram[3 * 0x10000 + 4], pattern(7, 0));

    let f = hw.vga.render();
    assert_eq!((f.width, f.height, f.display_aspect), (320, 200, (4, 3)));
    assert!(f.text.is_none());
    let want: Vec<u8> = (0..200)
        .flat_map(|y| (0..320).map(move |x| pattern(x, y)))
        .collect();
    assert!(f.pixels == want, "pixels differ from the pattern");
    let mut golden = f.pixels.clone();
    golden.extend(f.palette.iter().flat_map(|c| [c.r, c.g, c.b]));
    assert_eq!(fnv(&golden), 6_439_496_933_003_739_982, "golden hash");

    // Frames at vretrace carry the same picture.
    hw.start_video();
    let t = hw.sched.next_due().unwrap();
    hw.advance_to(t);
    hw.fire_due();
    assert!(hw.vga.take_frame().unwrap().pixels == want);
}

#[test]
fn dac_expands_6_to_8_bits_and_latches_at_retrace() {
    let mut hw = hw13();
    hw.io_out8(0x3C8, 1);
    for v in [0x3F, 0x20, 0x01] {
        hw.io_out8(0x3C9, v);
    }
    let f = hw.vga.render();
    assert_eq!(
        f.palette[1],
        Rgb8 {
            r: 0xFF,
            g: 0x82,
            b: 0x04
        }
    );
    assert_eq!(f.palette[0], Rgb8::default());

    // Fade idiom: rewrite the whole DAC during retrace; the next frame
    // uses the new values.
    hw.start_video();
    let t = hw.sched.next_due().unwrap();
    hw.advance_to(t);
    hw.fire_due();
    assert_eq!(hw.vga.take_frame().unwrap().palette[1].r, 0xFF);
    hw.io_out8(0x3C8, 0);
    for _ in 0..256 {
        for _ in 0..3 {
            hw.io_out8(0x3C9, 0x10);
        }
    }
    let t = hw.sched.next_due().unwrap();
    hw.advance_to(t);
    hw.fire_due();
    assert_eq!(
        hw.vga.take_frame().unwrap().palette[1],
        Rgb8 {
            r: 0x41,
            g: 0x41,
            b: 0x41
        }
    );
}

#[test]
fn unchained_mode_x_layout() {
    let mut hw = hw13();
    hw.io_out16(0x3C4, 0x0604); // chain-4 off
    crtc(&mut hw, 0x14, 0x00); // doubleword off
    crtc(&mut hw, 0x17, 0xE3); // byte mode
    for (plane, v) in [(0u8, 5u8), (1, 6), (2, 7), (3, 8)] {
        hw.io_out16(0x3C4, (1u16 << (8 + plane)) | 0x02);
        hw.write8(0xA0000, v);
    }
    let f = hw.vga.render();
    assert_eq!(f.pixels[..4], [5, 6, 7, 8]);
    assert_eq!(f.pixels[4], 0);
}

#[test]
fn start_address_and_line_compare() {
    let mut hw = hw13();
    for y in 0..200u32 {
        for x in 0..320u32 {
            hw.write8(0xA0000 + y * 320 + x, y as u8);
        }
    }
    // Start at row 10 (80 counter units per row).
    crtc(&mut hw, 0x0C, (800u16 >> 8) as u8);
    crtc(&mut hw, 0x0D, 800u16 as u8);
    assert_eq!(hw.vga.render().pixels[0], 10);
    // Split at scanline 99: rows from y=50 restart at address 0.
    crtc(&mut hw, 0x18, 99);
    let (c7, c9) = (hw.vga.crtc[7], hw.vga.crtc[9]);
    crtc(&mut hw, 0x07, c7 & !0x10);
    crtc(&mut hw, 0x09, c9 & !0x40);
    let f = hw.vga.render();
    assert_eq!(f.pixels[49 * 320], 59);
    assert_eq!(f.pixels[50 * 320], 0);
    assert_eq!(f.pixels[60 * 320], 10);
}

#[test]
fn mode_switch_3_13_3_restores_text() {
    let mut hw = Hardware::new();
    bios::install(&mut hw);
    hw.write16(0xB8000, 0x1F41);
    set_mode(&mut hw, 0x13);
    for i in 0..0x1_0000 {
        hw.write8(0xA0000 + i, 0xAA);
    }
    set_mode(&mut hw, 0x03);
    let f = hw.vga.render();
    assert_eq!((f.width, f.height), (720, 400));
    let t = f.text.as_ref().unwrap();
    assert!(t.cells.iter().all(|c| (c.ch, c.attr) == (b' ', 0x07)));
    let a = 2 * 0x10000 + 0x41 * 32;
    assert_eq!(
        hw.vga.vram[a..a + 16],
        font::FONT_8X16[0x41 * 16..0x42 * 16]
    );
    assert_eq!(
        f.palette[7],
        Rgb8 {
            r: 0xAA,
            g: 0xAA,
            b: 0xAA
        }
    );
}

#[test]
fn bios_pixels_teletype_and_scroll() {
    let mut hw = hw13();
    let mut cpu = Cpu::new();
    cpu.regs.set_ax(0x0C2A);
    cpu.regs.set_cx(10);
    cpu.regs.set_dx(20);
    int10(&mut cpu, &mut hw);
    assert_eq!(hw.read8(0xA0000 + 20 * 320 + 10), 0x2A);
    cpu.regs.set_ax(0x0D00);
    int10(&mut cpu, &mut hw);
    assert_eq!(cpu.regs.al(), 0x2A);

    // Teletype draws an 8x8 glyph in colour BL and advances.
    teletype(&mut hw, b'A', 0, 0x0E);
    assert_eq!(cursor(&mut hw, 0), (1, 0));
    for gy in 0..8u32 {
        let bits = font::FONT_8X8[0x41 * 8 + gy as usize];
        for gx in 0..8u32 {
            let want = if bits & (0x80 >> gx) != 0 { 0x0E } else { 0 };
            assert_eq!(hw.read8(0xA0000 + gy * 320 + gx), want, "({gx},{gy})");
        }
    }
    // LF on the last row scrolls the screen up 8 lines.
    set_cursor(&mut hw, 0, 0, 24);
    hw.write8(0xA0000 + 199 * 320, 0x33);
    teletype(&mut hw, b'\n', 0, 0x0E);
    assert_eq!(hw.read8(0xA0000 + 191 * 320), 0x33);
    assert_eq!(hw.read8(0xA0000 + 199 * 320), 0);

    assert_eq!(cursor(&mut hw, 0), (0, 24));
}
