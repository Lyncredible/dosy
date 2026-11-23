//! winit physical keys → PC scancode set 1.

use winit::keyboard::KeyCode;

/// Set-1 make code and whether it carries the `E0` prefix.
pub fn set1(code: KeyCode) -> Option<(u8, bool)> {
    use KeyCode::*;
    let plain = |c| Some((c, false));
    let ext = |c| Some((c, true));
    match code {
        Escape => plain(0x01),
        Digit1 => plain(0x02),
        Digit2 => plain(0x03),
        Digit3 => plain(0x04),
        Digit4 => plain(0x05),
        Digit5 => plain(0x06),
        Digit6 => plain(0x07),
        Digit7 => plain(0x08),
        Digit8 => plain(0x09),
        Digit9 => plain(0x0A),
        Digit0 => plain(0x0B),
        Minus => plain(0x0C),
        Equal => plain(0x0D),
        Backspace => plain(0x0E),
        Tab => plain(0x0F),
        KeyQ => plain(0x10),
        KeyW => plain(0x11),
        KeyE => plain(0x12),
        KeyR => plain(0x13),
        KeyT => plain(0x14),
        KeyY => plain(0x15),
        KeyU => plain(0x16),
        KeyI => plain(0x17),
        KeyO => plain(0x18),
        KeyP => plain(0x19),
        BracketLeft => plain(0x1A),
        BracketRight => plain(0x1B),
        Enter => plain(0x1C),
        ControlLeft => plain(0x1D),
        KeyA => plain(0x1E),
        KeyS => plain(0x1F),
        KeyD => plain(0x20),
        KeyF => plain(0x21),
        KeyG => plain(0x22),
        KeyH => plain(0x23),
        KeyJ => plain(0x24),
        KeyK => plain(0x25),
        KeyL => plain(0x26),
        Semicolon => plain(0x27),
        Quote => plain(0x28),
        Backquote => plain(0x29),
        ShiftLeft => plain(0x2A),
        Backslash => plain(0x2B),
        KeyZ => plain(0x2C),
        KeyX => plain(0x2D),
        KeyC => plain(0x2E),
        KeyV => plain(0x2F),
        KeyB => plain(0x30),
        KeyN => plain(0x31),
        KeyM => plain(0x32),
        Comma => plain(0x33),
        Period => plain(0x34),
        Slash => plain(0x35),
        ShiftRight => plain(0x36),
        NumpadMultiply => plain(0x37),
        AltLeft => plain(0x38),
        Space => plain(0x39),
        CapsLock => plain(0x3A),
        F1 => plain(0x3B),
        F2 => plain(0x3C),
        F3 => plain(0x3D),
        F4 => plain(0x3E),
        F5 => plain(0x3F),
        F6 => plain(0x40),
        F7 => plain(0x41),
        F8 => plain(0x42),
        F9 => plain(0x43),
        F10 => plain(0x44),
        NumLock => plain(0x45),
        ScrollLock => plain(0x46),
        Numpad7 => plain(0x47),
        Numpad8 => plain(0x48),
        Numpad9 => plain(0x49),
        NumpadSubtract => plain(0x4A),
        Numpad4 => plain(0x4B),
        Numpad5 => plain(0x4C),
        Numpad6 => plain(0x4D),
        NumpadAdd => plain(0x4E),
        Numpad1 => plain(0x4F),
        Numpad2 => plain(0x50),
        Numpad3 => plain(0x51),
        Numpad0 => plain(0x52),
        NumpadDecimal => plain(0x53),
        IntlBackslash => plain(0x56),
        F11 => plain(0x57),
        F12 => plain(0x58),
        NumpadEnter => ext(0x1C),
        ControlRight => ext(0x1D),
        NumpadDivide => ext(0x35),
        AltRight => ext(0x38),
        Home => ext(0x47),
        ArrowUp => ext(0x48),
        PageUp => ext(0x49),
        ArrowLeft => ext(0x4B),
        ArrowRight => ext(0x4D),
        End => ext(0x4F),
        ArrowDown => ext(0x50),
        PageDown => ext(0x51),
        Insert => ext(0x52),
        Delete => ext(0x53),
        SuperLeft => ext(0x5B),
        SuperRight => ext(0x5C),
        ContextMenu => ext(0x5D),
        _ => None,
    }
}

/// Bytes the keyboard sends for a press or release of `code` (empty if the
/// key has no PC equivalent). Pause sends its whole sequence on press and
/// nothing on release; PrintScreen sends its E0-wrapped fake-shift form.
pub fn scancodes(code: KeyCode, pressed: bool) -> Vec<u8> {
    match (code, pressed) {
        (KeyCode::Pause, true) => vec![0xE1, 0x1D, 0x45, 0xE1, 0x9D, 0xC5],
        (KeyCode::Pause, false) => vec![],
        (KeyCode::PrintScreen, true) => vec![0xE0, 0x2A, 0xE0, 0x37],
        (KeyCode::PrintScreen, false) => vec![0xE0, 0xB7, 0xE0, 0xAA],
        _ => match set1(code) {
            Some((make, e0)) => {
                let b = if pressed { make } else { make | 0x80 };
                if e0 { vec![0xE0, b] } else { vec![b] }
            }
            None => vec![],
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use KeyCode::*;

    #[test]
    fn table() {
        let cases: &[(KeyCode, &[u8], &[u8])] = &[
            (KeyA, &[0x1E], &[0x9E]),
            (KeyQ, &[0x10], &[0x90]),
            (KeyZ, &[0x2C], &[0xAC]),
            (KeyM, &[0x32], &[0xB2]),
            (Digit1, &[0x02], &[0x82]),
            (Digit0, &[0x0B], &[0x8B]),
            (F1, &[0x3B], &[0xBB]),
            (F10, &[0x44], &[0xC4]),
            (F11, &[0x57], &[0xD7]),
            (F12, &[0x58], &[0xD8]),
            (Escape, &[0x01], &[0x81]),
            (Enter, &[0x1C], &[0x9C]),
            (Space, &[0x39], &[0xB9]),
            (Backquote, &[0x29], &[0xA9]),
            (ShiftLeft, &[0x2A], &[0xAA]),
            (ShiftRight, &[0x36], &[0xB6]),
            (ControlLeft, &[0x1D], &[0x9D]),
            (AltLeft, &[0x38], &[0xB8]),
            (CapsLock, &[0x3A], &[0xBA]),
            (ControlRight, &[0xE0, 0x1D], &[0xE0, 0x9D]),
            (AltRight, &[0xE0, 0x38], &[0xE0, 0xB8]),
            (ArrowUp, &[0xE0, 0x48], &[0xE0, 0xC8]),
            (ArrowDown, &[0xE0, 0x50], &[0xE0, 0xD0]),
            (ArrowLeft, &[0xE0, 0x4B], &[0xE0, 0xCB]),
            (ArrowRight, &[0xE0, 0x4D], &[0xE0, 0xCD]),
            (Home, &[0xE0, 0x47], &[0xE0, 0xC7]),
            (Delete, &[0xE0, 0x53], &[0xE0, 0xD3]),
            (Numpad7, &[0x47], &[0xC7]),
            (Numpad0, &[0x52], &[0xD2]),
            (NumpadDecimal, &[0x53], &[0xD3]),
            (NumpadAdd, &[0x4E], &[0xCE]),
            (NumpadMultiply, &[0x37], &[0xB7]),
            (NumpadEnter, &[0xE0, 0x1C], &[0xE0, 0x9C]),
            (NumpadDivide, &[0xE0, 0x35], &[0xE0, 0xB5]),
            (NumLock, &[0x45], &[0xC5]),
            (SuperLeft, &[0xE0, 0x5B], &[0xE0, 0xDB]),
            (
                PrintScreen,
                &[0xE0, 0x2A, 0xE0, 0x37],
                &[0xE0, 0xB7, 0xE0, 0xAA],
            ),
            (Pause, &[0xE1, 0x1D, 0x45, 0xE1, 0x9D, 0xC5], &[]),
            (MediaPlayPause, &[], &[]),
        ];
        for &(code, make, brk) in cases {
            assert_eq!(scancodes(code, true), make, "{code:?} make");
            assert_eq!(scancodes(code, false), brk, "{code:?} break");
        }
    }

    #[test]
    fn makes_are_unique() {
        let all = [
            KeyA,
            KeyB,
            KeyC,
            KeyD,
            KeyE,
            KeyF,
            KeyG,
            KeyH,
            KeyI,
            KeyJ,
            KeyK,
            KeyL,
            KeyM,
            KeyN,
            KeyO,
            KeyP,
            KeyQ,
            KeyR,
            KeyS,
            KeyT,
            KeyU,
            KeyV,
            KeyW,
            KeyX,
            KeyY,
            KeyZ,
            Digit0,
            Digit1,
            Digit2,
            Digit3,
            Digit4,
            Digit5,
            Digit6,
            Digit7,
            Digit8,
            Digit9,
            Numpad0,
            Numpad1,
            Numpad2,
            Numpad3,
            Numpad4,
            Numpad5,
            Numpad6,
            Numpad7,
            Numpad8,
            Numpad9,
            Home,
            End,
            PageUp,
            PageDown,
            Insert,
            Delete,
            ArrowUp,
            ArrowDown,
            ArrowLeft,
            ArrowRight,
            NumpadEnter,
            NumpadDivide,
            Enter,
            Slash,
            ControlLeft,
            ControlRight,
        ];
        let mut seen = std::collections::HashSet::new();
        for k in all {
            assert!(
                seen.insert(set1(k).unwrap()),
                "{k:?} duplicates another key"
            );
        }
    }
}
