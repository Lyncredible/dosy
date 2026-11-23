use super::*;
use crate::bus::Bus;
use crate::hw::Hardware;

fn fnv(data: &[u8]) -> u64 {
    data.iter().fold(0xcbf2_9ce4_8422_2325, |h, &b| {
        (h ^ b as u64).wrapping_mul(0x100_0000_01b3)
    })
}

/// Write `s` at text cell (`col`, `row`) with `attr` through the CPU window.
fn put(hw: &mut Hardware, col: u32, row: u32, s: &[u8], attr: u8) {
    for (i, &c) in s.iter().enumerate() {
        let a = 0xB8000 + (row * 80 + col + i as u32) * 2;
        hw.write8(a, c);
        hw.write8(a + 1, attr);
    }
}

fn crtc(hw: &mut Hardware, i: u8, v: u8) {
    hw.io_out8(0x3D4, i);
    hw.io_out8(0x3D5, v);
}

#[test]
fn register_round_trips() {
    let mut hw = Hardware::new();
    crtc(&mut hw, 0x0E, 0x12);
    hw.io_out16(0x3D4, 0x340F); // index 0F, data 34
    hw.io_out8(0x3D4, 0x0E);
    assert_eq!(hw.io_in8(0x3D5), 0x12);
    hw.io_out8(0x3B4, 0x0F); // mono alias
    assert_eq!(hw.io_in8(0x3B5), 0x34);

    hw.io_out16(0x3C4, 0x0F02);
    assert_eq!(hw.vga.seq[2], 0x0F);
    hw.io_out16(0x3CE, 0x0204);
    hw.io_out8(0x3CE, 0x04);
    assert_eq!(hw.io_in8(0x3CF), 0x02);
    hw.io_out8(0x3C2, 0x63);
    assert_eq!(hw.io_in8(0x3CC), 0x63);

    // Attribute controller: 3DA read resets the flip-flop to "index".
    hw.io_out8(0x3C0, 0x55); // junk data/index
    hw.io_in8(0x3DA);
    hw.io_out8(0x3C0, 0x21); // index 1, PAS set
    hw.io_out8(0x3C0, 0x3F); // data
    assert_eq!(hw.io_in8(0x3C1), 0x3F);
    assert_eq!(hw.io_in8(0x3C0), 0x21);
    hw.io_out8(0x3C0, 0x22); // index again
    assert_eq!(hw.io_in8(0x3C1), hw.vga.ac[2]);

    // CRTC 11h bit 7 write-protects 0..7.
    crtc(&mut hw, 0x11, 0x8E);
    crtc(&mut hw, 0x00, 0x00);
    assert_eq!(hw.vga.crtc[0], 0x5F);
    crtc(&mut hw, 0x11, 0x0E);
    crtc(&mut hw, 0x00, 0x00);
    assert_eq!(hw.vga.crtc[0], 0x00);
}

#[test]
fn dac_auto_increments() {
    let mut hw = Hardware::new();
    hw.io_out8(0x3C8, 5);
    for v in [1, 2, 3, 0xFF, 5, 6] {
        hw.io_out8(0x3C9, v);
    }
    assert_eq!(hw.vga.dac[5], [1, 2, 3]);
    assert_eq!(hw.vga.dac[6], [0x3F, 5, 6], "6-bit values");
    assert_eq!(hw.io_in8(0x3C8), 7);
    assert_eq!(hw.io_in8(0x3C7), 0x00);
    hw.io_out8(0x3C7, 5);
    assert_eq!(hw.io_in8(0x3C7), 0x03);
    let got: Vec<u8> = (0..6).map(|_| hw.io_in8(0x3C9)).collect();
    assert_eq!(got, [1, 2, 3, 0x3F, 5, 6]);
    // Default EGA palette: entry 0x14 is brown.
    assert_eq!(hw.vga.dac[0x14], [0x2A, 0x15, 0x00]);
}

#[test]
fn input_status_follows_display_timing() {
    let mut hw = Hardware::new();
    let v = &hw.vga;
    // 720x400 @ 70 Hz: 900 dots x 449 lines at 28.322 MHz.
    assert_eq!((v.dots_per_line(), v.total_lines()), (900, 449));
    let at = |line: u64, dot: u64| hw.vga.ns_at(line * 900 + dot);
    let (active, hblank) = (at(10, 100), at(10, 800));
    let (vblank, retrace, after) = (at(405, 100), at(412, 100), at(414, 100));
    let frame = hw.vga.ns_at(900 * 449);
    assert!((14_260_000..14_270_000).contains(&frame), "{frame}");
    for (t, want) in [
        (active, 0x00),
        (hblank, 0x01),
        (vblank, 0x01),
        (retrace, 0x09),
        (after, 0x01),
        (frame + active, 0x00),
    ] {
        hw.advance_to(t);
        assert_eq!(hw.io_in8(0x3DA), want, "t={t}");
    }
}

#[test]
fn text_window_odd_even_and_font_plane() {
    let mut hw = Hardware::new();
    put(&mut hw, 0, 0, b"A", 0x1F);
    assert_eq!(hw.vga.vram[0], b'A');
    assert_eq!(hw.vga.vram[PLANE_SIZE], 0x1F);
    assert_eq!(hw.read8(0xB8001), 0x1F);
    assert_eq!(hw.read8(0xA0000), 0xFF, "outside the B8000 window");
    // Built-in font sits in plane 2 at 32 bytes per char.
    let a = 2 * PLANE_SIZE + 0x41 * FONT_STRIDE;
    assert_eq!(
        hw.vga.vram[a..a + 16],
        font::FONT_8X16[0x41 * 16..0x42 * 16]
    );

    // BIOS-style font access: plane 2, sequential, A0000 64K.
    hw.io_out16(0x3C4, 0x0402);
    hw.io_out16(0x3C4, 0x0704);
    hw.io_out16(0x3CE, 0x0204);
    hw.io_out16(0x3CE, 0x0005);
    hw.io_out16(0x3CE, 0x0406);
    hw.write8(0xA0000 + 0x41 * 32 + 3, 0xAA);
    assert_eq!(hw.vga.vram[a + 3], 0xAA);
    assert_eq!(hw.read8(0xA0000 + 0x41 * 32 + 3), 0xAA);
    assert_eq!(hw.vga.vram[0], b'A', "plane 0 untouched");
}

#[test]
fn text_rasterization() {
    let mut hw = Hardware::new();
    put(&mut hw, 0, 0, b"Hello, dosy!", 0x1E);
    put(&mut hw, 0, 1, &[0xC4, 0xB3, 0xDB], 0x07);
    put(&mut hw, 79, 24, b"Z", 0x8C); // blinking
    crtc(&mut hw, 0x0A, 0x20); // cursor off
    let f = hw.vga.render();
    assert_eq!((f.width, f.height), (720, 400));
    let t = f.text.as_ref().unwrap();
    assert_eq!((t.cols, t.rows), (80, 25));
    assert_eq!(
        t.cells[0],
        crate::video::TextCell {
            ch: b'H',
            attr: 0x1E
        }
    );
    assert_eq!(t.cursor, None);
    assert_eq!(
        t.palette16[1],
        Rgb8 {
            r: 0,
            g: 0,
            b: 0xAA
        }
    );

    let px = |x: usize, y: usize| f.pixels[y * 720 + x];
    // 'H' row 5: glyph bits, fg = DAC 0x3E (yellow), bg = DAC 0x01 (blue).
    let g = font::FONT_8X16[b'H' as usize * 16 + 5];
    for x in 0..8 {
        let want = if g & (0x80 >> x) != 0 { 0x3E } else { 0x01 };
        assert_eq!(px(x, 5), want, "x={x}");
    }
    assert_eq!(px(8, 5), 0x01, "9th column is background");
    // Line-graphics char C4h extends into the 9th column.
    let y = 16 + 7;
    assert_eq!(px(8, y), px(7, y));
    assert_eq!(fnv(&f.pixels), 16_275_925_479_570_880_639, "golden hash");
}

#[test]
fn cursor_and_blink() {
    let mut hw = Hardware::new();
    put(&mut hw, 0, 1, b"x", 0x8F);
    put(&mut hw, 1, 1, b" ", 0x07);
    crtc(&mut hw, 0x0E, 0);
    crtc(&mut hw, 0x0F, 81); // (1, 1)
    let f = text::render(&hw.vga, 0);
    let t = f.text.as_ref().unwrap();
    assert_eq!(t.cursor, Some((1, 1)));
    assert!(t.blink_on);
    // Cursor scanlines 0D..0E are solid fg (attr 07 -> DAC 07).
    let row_px = |f: &VideoFrame, y: usize| f.pixels[(16 + y) * 720 + 9..][..9].to_vec();
    assert_eq!(row_px(&f, 0x0D), vec![7; 9]);
    assert_eq!(row_px(&f, 0x0C), vec![0; 9]);
    // Off phase (frame 8): cursor hidden; blinking 'x' still shown.
    let off = text::render(&hw.vga, 8);
    assert_eq!(row_px(&off, 0x0D), vec![0; 9]);
    // Blink phase off (frame 16): blinking char drawn in its background.
    let blink = text::render(&hw.vga, 16);
    assert!(!blink.text.as_ref().unwrap().blink_on);
    fn cell(f: &VideoFrame) -> Vec<u8> {
        (0..16)
            .flat_map(|y| f.pixels[(16 + y) * 720..][..9].to_vec())
            .collect()
    }
    assert!(cell(&blink).iter().all(|&p| p == 0));
    assert!(cell(&f).contains(&0x3F));
}

#[test]
fn forty_columns_double_pixels() {
    let mut hw = Hardware::new();
    hw.io_out16(0x3C4, 0x0901); // 8-dot chars, dot clock / 2
    crtc(&mut hw, 0x11, 0x0E); // unprotect 0..7
    crtc(&mut hw, 0x01, 39);
    crtc(&mut hw, 0x13, 20);
    put(&mut hw, 0, 0, b"W", 0x0F);
    let f = hw.vga.render();
    assert_eq!((f.width, f.text.as_ref().unwrap().cols), (640, 40));
    for x in (0..16).step_by(2) {
        assert_eq!(f.pixels[3 * 640 + x], f.pixels[3 * 640 + x + 1]);
    }
}

#[test]
fn frames_at_vretrace() {
    let mut hw = Hardware::new();
    hw.start_video();
    assert_eq!(hw.vga.take_frame(), None);
    let t = hw.sched.next_due().unwrap();
    assert_eq!(t, hw.vga.ns_at(412 * 900));
    hw.advance_to(t);
    hw.fire_due();
    let f = hw.vga.take_frame().expect("frame at vretrace");
    assert_eq!((f.seq, f.width, f.height), (1, 720, 400));
    assert_eq!(
        f.palette[7],
        Rgb8 {
            r: 0xAA,
            g: 0xAA,
            b: 0xAA
        }
    );
    assert_eq!(hw.vga.take_frame(), None);
    let next = hw.sched.next_due().unwrap();
    let period = hw.vga.ns_at(900 * 449);
    assert!((next - t).abs_diff(period) <= 1, "one frame later");
}
