//! Renderer-facing video output types. The core produces these; `dosy-render`
//! consumes them. Field additions are fine; breaking changes need an
//! architecture note.

/// 8-bit-per-channel color (VGA DAC 6-bit values are expanded to 8 bits).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Rgb8 {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

/// One text-mode character cell.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TextCell {
    /// Code page 437 character code.
    pub ch: u8,
    /// VGA attribute byte (low nibble fg, high nibble bg / blink).
    pub attr: u8,
}

/// Text-mode screen contents, for optional vector-font rendering.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TextScreen {
    pub cols: u16,
    pub rows: u16,
    /// Row-major, `cols * rows` cells.
    pub cells: Vec<TextCell>,
    /// `(col, row)` when the cursor is visible.
    pub cursor: Option<(u16, u16)>,
    /// Attribute index (0..16) to color, after the attribute controller.
    pub palette16: [Rgb8; 16],
    /// Current blink phase (blinking chars / cursor visible when true).
    pub blink_on: bool,
}

/// A complete emulated frame. Always contains rasterized indexed pixels
/// (text modes are rasterized by the core), so a renderer only needs the
/// indexed path; `text` is extra information for vector text rendering.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VideoFrame {
    /// Emulated pixels, e.g. 320x200 (mode 13h) or 720x400 (text).
    pub width: u32,
    pub height: u32,
    /// Palette indices, row-major, `width * height` bytes.
    pub pixels: Vec<u8>,
    pub palette: [Rgb8; 256],
    /// Intended display aspect ratio of the whole frame, usually `(4, 3)`.
    pub display_aspect: (u32, u32),
    /// Present in text modes.
    pub text: Option<TextScreen>,
    /// Monotonic frame counter.
    pub seq: u64,
}

impl VideoFrame {
    /// A black frame of the given size.
    pub fn blank(width: u32, height: u32) -> Self {
        Self {
            width,
            height,
            pixels: vec![0; (width * height) as usize],
            palette: [Rgb8::default(); 256],
            display_aspect: (4, 3),
            text: None,
            seq: 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blank_frame_has_one_byte_per_pixel() {
        let f = VideoFrame::blank(320, 200);
        assert_eq!(f.pixels.len(), 64_000);
        assert_eq!(f.display_aspect, (4, 3));
    }
}
