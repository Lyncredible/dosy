//! Graphics rasterizers: 256-colour (mode 13h and, unchained, mode X) and
//! 16-colour planar (modes 0Dh–12h).
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

/// CRTC memory address counter to plane offset (dword / word / byte mode).
fn phys(v: &Vga, counter: usize) -> usize {
    let dword = v.crtc[0x14] & 0x40 != 0;
    let word = !dword && v.crtc[0x17] & 0x40 == 0;
    if dword {
        (counter << 2) & 0xFFFF
    } else if word {
        // Word mode: shift left, MA13 or MA15 into bit 0 (CRTC 17h bit 5).
        let hi = if v.crtc[0x17] & 0x20 != 0 { 15 } else { 13 };
        ((counter << 1) | ((counter >> hi) & 1)) & 0xFFFF
    } else {
        counter & 0xFFFF
    }
}

/// Lines per pixel row (max scan line + double scan).
fn scan_height(v: &Vga) -> usize {
    ((v.crtc[9] & 0x1F) as usize + 1) << (v.crtc[9] >> 7)
}

fn start_address(v: &Vga) -> usize {
    (v.crtc[0xC] as usize) << 8 | v.crtc[0xD] as usize
}

/// 16-colour planar: each counter step fetches one byte per plane = 8
/// pixels (bit 7 first); the 4-bit colour goes through colour plane enable
/// (AC 12h) and the attribute palette.
pub(super) fn render16(v: &Vga) -> VideoFrame {
    let scan_h = scan_height(v);
    let height = v.display_lines() as usize / scan_h;
    let width = (v.crtc[1] as usize + 1) * 8;
    let pitch = v.crtc[0x13] as usize * 2;
    let lc = line_compare(v);
    let mut attr = [0u8; 16];
    for (i, a) in attr.iter_mut().enumerate() {
        *a = v.attr_to_dac((i as u8) & v.ac[0x12]);
    }
    let mut pixels = vec![0u8; width * height];
    let mut row_start = start_address(v);
    let mut split = false;
    for y in 0..height {
        if !split && y * scan_h > lc {
            split = true;
            row_start = 0;
        }
        let line = &mut pixels[y * width..(y + 1) * width];
        for (cx, oct) in line.chunks_mut(8).enumerate() {
            let off = phys(v, row_start + cx);
            let planes: [u8; 4] = std::array::from_fn(|p| v.vram[p * PLANE_SIZE + off]);
            for (bit, px) in oct.iter_mut().enumerate() {
                let sh = 7 - bit;
                let idx = (0..4).fold(0u8, |c, p| c | ((planes[p] >> sh) & 1) << p);
                *px = attr[idx as usize];
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

pub(super) fn render256(v: &Vga) -> VideoFrame {
    let scan_h = scan_height(v);
    let lines = v.display_lines() as usize;
    let height = lines / scan_h;
    // 8 dots per character clock, 2 dots per 256-colour pixel.
    let width = (v.crtc[1] as usize + 1) * 4;
    let start = start_address(v);
    let pitch = v.crtc[0x13] as usize * 2;
    let phys = |counter: usize| phys(v, counter);
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
