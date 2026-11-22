//! Pure viewport math: where the emulated frame goes inside the window.

/// How the frame is scaled into the window.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ScaleMode {
    /// Largest integer multiple of the aspect-corrected size; `Fit` if < 1x.
    #[default]
    Integer,
    /// Largest aspect-correct size that fits.
    Fit,
    /// Fill the whole padded area, ignoring aspect.
    Stretch,
}

/// A pixel rectangle in window coordinates.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Rect {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
}

/// Frame size after aspect correction: width is kept, height becomes
/// `width * aspect_h / aspect_w` (rounded). 320x200 @ 4:3 -> 320x240.
/// A zero aspect component means "square pixels" (size unchanged).
pub fn corrected_size(frame: (u32, u32), aspect: (u32, u32)) -> (u32, u32) {
    let (w, _) = frame;
    let (aw, ah) = aspect;
    if aw == 0 || ah == 0 || w == 0 {
        return frame;
    }
    let ch = (w as u64 * ah as u64 + aw as u64 / 2) / aw as u64;
    (w, (ch as u32).max(1))
}

/// Viewport for a `frame` with display `aspect` inside a `window`, leaving
/// `padding` (x, y) pixels on each side. Padding that leaves no room is
/// ignored. Returns `None` when the window or frame is empty.
pub fn viewport(
    frame: (u32, u32),
    aspect: (u32, u32),
    window: (u32, u32),
    padding: (u32, u32),
    mode: ScaleMode,
) -> Option<Rect> {
    let (ww, wh) = window;
    if ww == 0 || wh == 0 || frame.0 == 0 || frame.1 == 0 {
        return None;
    }
    let (mut px, mut py) = padding;
    if px.saturating_mul(2) >= ww {
        px = 0;
    }
    if py.saturating_mul(2) >= wh {
        py = 0;
    }
    let (aw, ah) = (ww - 2 * px, wh - 2 * py);
    let (cw, ch) = corrected_size(frame, aspect);

    let (w, h) = match mode {
        ScaleMode::Stretch => (aw, ah),
        ScaleMode::Integer => {
            let n = (aw / cw).min(ah / ch);
            if n >= 1 {
                (cw * n, ch * n)
            } else {
                fit(cw, ch, aw, ah)
            }
        }
        ScaleMode::Fit => fit(cw, ch, aw, ah),
    };
    Some(Rect {
        x: px + (aw - w) / 2,
        y: py + (ah - h) / 2,
        w,
        h,
    })
}

/// Largest `cw:ch` rectangle inside `aw x ah` (at least 1x1).
fn fit(cw: u32, ch: u32, aw: u32, ah: u32) -> (u32, u32) {
    // Compare aw/cw with ah/ch without floats: limited by width if aw*ch <= ah*cw.
    let (cw64, ch64, aw64, ah64) = (cw as u64, ch as u64, aw as u64, ah as u64);
    if aw64 * ch64 <= ah64 * cw64 {
        let h = (aw64 * ch64 + cw64 / 2) / cw64;
        (aw, (h as u32).clamp(1, ah))
    } else {
        let w = (ah64 * cw64 + ch64 / 2) / ch64;
        ((w as u32).clamp(1, aw), ah)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const A43: (u32, u32) = (4, 3);

    fn vp(frame: (u32, u32), window: (u32, u32), pad: (u32, u32), mode: ScaleMode) -> Rect {
        viewport(frame, A43, window, pad, mode).unwrap()
    }

    #[test]
    fn corrected_sizes() {
        assert_eq!(corrected_size((320, 200), A43), (320, 240));
        assert_eq!(corrected_size((720, 400), A43), (720, 540));
        assert_eq!(corrected_size((640, 480), A43), (640, 480));
        assert_eq!(corrected_size((320, 200), (0, 0)), (320, 200));
    }

    #[test]
    fn mode13h_integer() {
        let r = vp((320, 200), (1280, 800), (0, 0), ScaleMode::Integer);
        assert_eq!(
            r,
            Rect {
                x: 160,
                y: 40,
                w: 960,
                h: 720
            }
        );
        let r = vp((320, 200), (640, 480), (0, 0), ScaleMode::Integer);
        assert_eq!(
            r,
            Rect {
                x: 0,
                y: 0,
                w: 640,
                h: 480
            }
        );
    }

    #[test]
    fn text_mode_integer_and_fallback() {
        let r = vp((720, 400), (1920, 1080), (0, 0), ScaleMode::Integer);
        assert_eq!(
            r,
            Rect {
                x: 240,
                y: 0,
                w: 1440,
                h: 1080
            }
        );
        // Smaller than 1x falls back to fit.
        let r = vp((720, 400), (640, 480), (0, 0), ScaleMode::Integer);
        assert_eq!(
            r,
            Rect {
                x: 0,
                y: 0,
                w: 640,
                h: 480
            }
        );
    }

    #[test]
    fn odd_window_sizes() {
        let r = vp((320, 200), (1001, 777), (0, 0), ScaleMode::Integer);
        assert_eq!(
            r,
            Rect {
                x: 20,
                y: 28,
                w: 960,
                h: 720
            }
        );
        let r = vp((320, 200), (1001, 777), (0, 0), ScaleMode::Fit);
        assert_eq!((r.w, r.h), (1001, 751));
        assert_eq!(r.y, 13);
        let r = vp((320, 200), (333, 1001), (0, 0), ScaleMode::Fit);
        assert_eq!((r.x, r.w, r.h), (0, 333, 250));
        let r = vp((320, 200), (1, 1), (0, 0), ScaleMode::Integer);
        assert_eq!(
            r,
            Rect {
                x: 0,
                y: 0,
                w: 1,
                h: 1
            }
        );
    }

    #[test]
    fn padding() {
        let r = vp((320, 200), (1000, 760), (20, 20), ScaleMode::Integer);
        assert_eq!(
            r,
            Rect {
                x: 20,
                y: 20,
                w: 960,
                h: 720
            }
        );
        let r = vp((320, 200), (1000, 760), (10, 10), ScaleMode::Stretch);
        assert_eq!(
            r,
            Rect {
                x: 10,
                y: 10,
                w: 980,
                h: 740
            }
        );
        // Padding larger than the window is ignored on that axis.
        let r = vp((320, 200), (640, 480), (400, 10), ScaleMode::Stretch);
        assert_eq!(
            r,
            Rect {
                x: 0,
                y: 10,
                w: 640,
                h: 460
            }
        );
        let r = vp((320, 200), (640, 480), (320, 240), ScaleMode::Integer);
        assert_eq!(
            r,
            Rect {
                x: 0,
                y: 0,
                w: 640,
                h: 480
            }
        );
    }

    #[test]
    fn empty_inputs() {
        assert_eq!(
            viewport((320, 200), A43, (0, 480), (0, 0), ScaleMode::Fit),
            None
        );
        assert_eq!(
            viewport((0, 0), A43, (640, 480), (0, 0), ScaleMode::Fit),
            None
        );
    }
}
