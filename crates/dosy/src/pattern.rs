//! Test-pattern frames shown until the machine loop lands (task 022).

use dosy_core::video::{Rgb8, VideoFrame};

/// The 16 EGA/VGA text colors.
pub const EGA16: [Rgb8; 16] = {
    const fn c(r: u8, g: u8, b: u8) -> Rgb8 {
        Rgb8 { r, g, b }
    }
    [
        c(0x00, 0x00, 0x00),
        c(0x00, 0x00, 0xaa),
        c(0x00, 0xaa, 0x00),
        c(0x00, 0xaa, 0xaa),
        c(0xaa, 0x00, 0x00),
        c(0xaa, 0x00, 0xaa),
        c(0xaa, 0x55, 0x00),
        c(0xaa, 0xaa, 0xaa),
        c(0x55, 0x55, 0x55),
        c(0x55, 0x55, 0xff),
        c(0x55, 0xff, 0x55),
        c(0x55, 0xff, 0xff),
        c(0xff, 0x55, 0x55),
        c(0xff, 0x55, 0xff),
        c(0xff, 0xff, 0x55),
        c(0xff, 0xff, 0xff),
    ]
};

/// 320x200 (mode 13h-like): 16 EGA bars on top, a 240-entry hue/gray ramp
/// below, and a 1-pixel white border to check edge crispness.
pub fn bars_320x200() -> VideoFrame {
    let (w, h) = (320u32, 200u32);
    let mut f = VideoFrame::blank(w, h);
    f.palette[..16].copy_from_slice(&EGA16);
    for (i, c) in f.palette[16..].iter_mut().enumerate() {
        *c = ramp(i);
    }
    for y in 0..h {
        for x in 0..w {
            let idx = if x == 0 || y == 0 || x == w - 1 || y == h - 1 {
                15
            } else if y < h / 2 {
                (x * 16 / w) as u8
            } else {
                (16 + x * 240 / w) as u8
            };
            f.pixels[(y * w + x) as usize] = idx;
        }
    }
    f
}

/// 720x400 (80x25 text-like, 9x16 cells): each cell has a background color
/// by row and a box "glyph" in a foreground color by column.
pub fn text_720x400() -> VideoFrame {
    let (w, h) = (720u32, 400u32);
    let mut f = VideoFrame::blank(w, h);
    f.palette[..16].copy_from_slice(&EGA16);
    for y in 0..h {
        for x in 0..w {
            let (col, row, cx, cy) = (x / 9, y / 16, x % 9, y % 16);
            let bg = (row % 8) as u8;
            let fg = 8 + (col % 8) as u8;
            let glyph = (1..8).contains(&cx)
                && (3..13).contains(&cy)
                && (cx == 1 || cx == 7 || cy == 3 || cy == 12 || (cx + cy) % 2 == 0);
            f.pixels[(y * w + x) as usize] = if glyph { fg } else { bg };
        }
    }
    f
}

/// 0..160: six hue segments; 160..240: gray ramp.
fn ramp(i: usize) -> Rgb8 {
    if i >= 160 {
        let v = ((i - 160) * 255 / 79) as u8;
        return Rgb8 { r: v, g: v, b: v };
    }
    let seg = i * 6 / 160;
    let t = ((i * 6 % 160) * 255 / 159) as u8;
    let (r, g, b) = match seg {
        0 => (255, t, 0),
        1 => (255 - t, 255, 0),
        2 => (0, 255, t),
        3 => (0, 255 - t, 255),
        4 => (t, 0, 255),
        _ => (255, 0, 255 - t),
    };
    Rgb8 { r, g, b }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn patterns_are_well_formed() {
        for f in [bars_320x200(), text_720x400()] {
            assert_eq!(f.pixels.len(), (f.width * f.height) as usize);
            assert_eq!(f.display_aspect, (4, 3));
        }
        let b = bars_320x200();
        assert_eq!(b.pixels[0], 15);
        assert_eq!(b.pixels[(10 * 320 + 25) as usize], 1);
        assert_eq!(
            b.palette[255],
            Rgb8 {
                r: 255,
                g: 255,
                b: 255
            }
        );
        let t = text_720x400();
        assert_eq!(t.pixels[0], 0);
        assert_eq!(t.pixels[(3 * 720 + 1) as usize], 8);
    }
}
