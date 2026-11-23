//! Text-mode rasterizer: VRAM planes 0/1 (chars/attributes) + font in
//! plane 2 → indexed `VideoFrame` with a `TextScreen`.

use super::{FONT_STRIDE, PLANE_SIZE, Vga, bank_offset, dac8};
use crate::video::{Rgb8, TextCell, TextScreen, VideoFrame};

/// Frames per cursor blink phase; character blink is twice as slow.
const CURSOR_BLINK_FRAMES: u64 = 8;

pub(super) fn render(v: &Vga, frame_no: u64) -> VideoFrame {
    let cols = v.crtc[1] as usize + 1;
    let char_w = v.char_width() as usize;
    let dclk2 = v.seq[1] & 8 != 0;
    let char_h = (v.crtc[9] & 0x1F) as usize + 1;
    let double_scan = v.crtc[9] & 0x80 != 0;
    let scan_h = char_h << double_scan as usize;
    let height = v.display_lines() as usize;
    let rows = height / scan_h;
    let px_w = 1 + dclk2 as usize;
    let width = cols * char_w * px_w;

    let start = (v.crtc[0xC] as usize) << 8 | v.crtc[0xD] as usize;
    let pitch = v.crtc[0x13] as usize * 2;
    let cursor_addr = (v.crtc[0xE] as usize) << 8 | v.crtc[0xF] as usize;
    let (cur_start, cur_end) = ((v.crtc[0xA] & 0x1F) as usize, (v.crtc[0xB] & 0x1F) as usize);
    let cursor_enabled = v.crtc[0xA] & 0x20 == 0 && cur_start <= cur_end;
    let cursor_on = (frame_no / CURSOR_BLINK_FRAMES).is_multiple_of(2);
    let blink_on = (frame_no / (2 * CURSOR_BLINK_FRAMES)).is_multiple_of(2);
    let blink_enabled = v.ac[0x10] & 8 != 0;
    let line_graphics = v.ac[0x10] & 4 != 0;

    let s = v.seq[3] as usize;
    let map_a = bank_offset(((s >> 3) & 4) | ((s >> 2) & 3));
    let map_b = bank_offset(((s >> 2) & 4) | (s & 3));
    let font = &v.vram[2 * PLANE_SIZE..3 * PLANE_SIZE];

    let dac16: [u8; 16] = std::array::from_fn(|i| v.attr_to_dac(i as u8));

    let mut pixels = vec![0u8; width * height];
    let mut cells = Vec::with_capacity(cols * rows);
    let mut cursor = None;
    for row in 0..rows {
        for col in 0..cols {
            let addr = (start + row * pitch + col) & 0x7FFF;
            let off = (addr * 2) % PLANE_SIZE;
            let ch = v.vram[off];
            let attr = v.vram[PLANE_SIZE + off];
            cells.push(TextCell { ch, attr });

            let mut fg = attr & 0xF;
            let mut bg = attr >> 4;
            if blink_enabled {
                bg &= 7;
                if attr & 0x80 != 0 && !blink_on {
                    fg = bg;
                }
            }
            let is_cursor = cursor_enabled && addr == (cursor_addr & 0x7FFF);
            if is_cursor {
                cursor = Some((col as u16, row as u16));
            }
            let (fg, bg) = (dac16[fg as usize], dac16[bg as usize]);
            let glyph_base = if attr & 8 != 0 { map_a } else { map_b } + ch as usize * FONT_STRIDE;
            let ninth = char_w == 9 && line_graphics && (0xC0..=0xDF).contains(&ch);

            for y in 0..scan_h {
                let gy = y >> double_scan as usize;
                let mut bits = font[glyph_base + gy] as u16;
                if is_cursor && cursor_on && (cur_start..=cur_end).contains(&gy) {
                    bits = 0x1FF;
                } else {
                    bits <<= 1;
                    if ninth {
                        bits |= (bits >> 1) & 1;
                    }
                }
                let line = &mut pixels[(row * scan_h + y) * width + col * char_w * px_w..];
                for x in 0..char_w {
                    let on = bits & (0x100 >> x) != 0;
                    let c = if on { fg } else { bg };
                    line[x * px_w] = c;
                    if dclk2 {
                        line[x * px_w + 1] = c;
                    }
                }
            }
        }
    }

    let palette16: [Rgb8; 16] = std::array::from_fn(|i| {
        let c = v.dac[dac16[i] as usize];
        Rgb8 {
            r: dac8(c[0]),
            g: dac8(c[1]),
            b: dac8(c[2]),
        }
    });
    VideoFrame {
        width: width as u32,
        height: height as u32,
        pixels,
        palette: [Rgb8::default(); 256],
        display_aspect: (4, 3),
        text: Some(TextScreen {
            cols: cols as u16,
            rows: rows as u16,
            cells,
            cursor,
            palette16,
            blink_on,
        }),
        seq: frame_no,
    }
}
