//! 256-colour graphics rasterizer (mode 13h and, unchained, mode X).
//!
//! Follows the CRTC memory address counter: each counter step fetches one
//! byte from each of the 4 planes = 4 pixels. Doubleword mode (CRTC 14h
//! bit 6, set in mode 13h) maps counter `n` to plane offset `4n`, which
//! together with chain-4 CPU writes gives the linear framebuffer.
//! Duplicate scanlines (max scan line / double scan) are collapsed, so mode
//! 13h produces a 320x200 frame.

use super::{PLANE_SIZE, Vga};
use crate::video::VideoFrame;

/// The line compare value (CRTC 18h + overflow bits), in scanlines.
fn line_compare(v: &Vga) -> usize {
    let b8 = (v.crtc[7] as usize >> 4) & 1;
    let b9 = (v.crtc[9] as usize >> 6) & 1;
    v.crtc[0x18] as usize | (b8 << 8) | (b9 << 9)
}

pub(super) fn render256(v: &Vga) -> VideoFrame {
    let scan_h = ((v.crtc[9] & 0x1F) as usize + 1) << (v.crtc[9] >> 7);
    let lines = v.display_lines() as usize;
    let height = lines / scan_h;
    // 8 dots per character clock, 2 dots per 256-colour pixel.
    let width = (v.crtc[1] as usize + 1) * 4;
    let start = (v.crtc[0xC] as usize) << 8 | v.crtc[0xD] as usize;
    let pitch = v.crtc[0x13] as usize * 2;
    let dword = v.crtc[0x14] & 0x40 != 0;
    let word = !dword && v.crtc[0x17] & 0x40 == 0;
    let phys = |counter: usize| -> usize {
        if dword {
            (counter << 2) & 0xFFFF
        } else if word {
            // Word mode: shift left, MA13 or MA15 into bit 0 (CRTC 17h bit 5).
            let hi = if v.crtc[0x17] & 0x20 != 0 { 15 } else { 13 };
            ((counter << 1) | ((counter >> hi) & 1)) & 0xFFFF
        } else {
            counter & 0xFFFF
        }
    };
    let lc = line_compare(v);
    let mask = v.pel_mask;

    let mut pixels = vec![0u8; width * height];
    let mut row_start = start;
    let mut split = false;
    for y in 0..height {
        if !split && y * scan_h > lc {
            // Split screen: the rest starts at address 0.
            split = true;
            row_start = 0;
        }
        let line = &mut pixels[y * width..(y + 1) * width];
        for (cx, quad) in line.chunks_mut(4).enumerate() {
            let off = phys(row_start + cx);
            for (p, px) in quad.iter_mut().enumerate() {
                *px = v.vram[p * PLANE_SIZE + off] & mask;
            }
        }
        row_start += pitch;
    }
    VideoFrame {
        width: width as u32,
        height: height as u32,
        pixels,
        ..VideoFrame::blank(0, 0)
    }
}
