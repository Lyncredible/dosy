//! GPU presenter for [`dosy_core::video::VideoFrame`]s. Will use wgpu and take
//! a surface from the app; it never depends on a windowing library.
//! See `docs/ARCHITECTURE.md` ("dosy-render") and task 006.

use dosy_core::video::VideoFrame;

/// Crate version.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Size in bytes of the indexed pixel upload for `frame`.
pub fn upload_size(frame: &VideoFrame) -> usize {
    frame.pixels.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn upload_size_matches_pixels() {
        assert_eq!(upload_size(&VideoFrame::blank(720, 400)), 288_000);
    }
}
