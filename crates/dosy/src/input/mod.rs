//! Host input → guest: keyboard state and scancode translation.

pub mod keymap;

use std::collections::BTreeSet;

use winit::keyboard::KeyCode;

/// Host shortcut handled by the app instead of the guest.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shortcut {
    /// Ctrl+Alt+F12: cycle the debug test pattern.
    NextPattern,
}

/// Tracks held keys so focus loss can release them, and turns key events
/// into scancode bytes for `Machine::key_event`.
#[derive(Default)]
pub struct Keyboard {
    held: BTreeSet<KeyCode>,
}

impl Keyboard {
    /// A key press (`repeat` = host auto-repeat, sent as another make) or
    /// release. Returns the bytes to send, or a host shortcut.
    pub fn key(&mut self, code: KeyCode, pressed: bool) -> Result<Vec<u8>, Shortcut> {
        if pressed && code == KeyCode::F12 && self.ctrl_alt() {
            return Err(Shortcut::NextPattern);
        }
        if code == KeyCode::Pause {
            // Pause has no typematic or break code; Ctrl+Pause is Break.
            if !pressed {
                self.held.remove(&code);
                return Ok(Vec::new());
            }
            if !self.held.insert(code) {
                return Ok(Vec::new());
            }
            if self.held.contains(&KeyCode::ControlLeft)
                || self.held.contains(&KeyCode::ControlRight)
            {
                return Ok(vec![0xE0, 0x46, 0xE0, 0xC6]);
            }
        } else if pressed {
            self.held.insert(code);
        } else if !self.held.remove(&code) {
            // Release of a key we never sent (e.g. pressed before focus).
            return Ok(Vec::new());
        }
        Ok(keymap::scancodes(code, pressed))
    }

    /// Focus lost: release everything still held, so no key sticks.
    pub fn release_all(&mut self) -> Vec<u8> {
        let held = std::mem::take(&mut self.held);
        held.into_iter()
            .flat_map(|k| keymap::scancodes(k, false))
            .collect()
    }

    fn ctrl_alt(&self) -> bool {
        let any = |a, b| self.held.contains(&a) || self.held.contains(&b);
        any(KeyCode::ControlLeft, KeyCode::ControlRight) && any(KeyCode::AltLeft, KeyCode::AltRight)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repeats_are_makes_and_focus_loss_releases() {
        let mut k = Keyboard::default();
        assert_eq!(k.key(KeyCode::ShiftLeft, true), Ok(vec![0x2A]));
        assert_eq!(k.key(KeyCode::KeyA, true), Ok(vec![0x1E]));
        assert_eq!(k.key(KeyCode::KeyA, true), Ok(vec![0x1E]), "auto-repeat");
        assert_eq!(k.key(KeyCode::ArrowUp, true), Ok(vec![0xE0, 0x48]));
        assert_eq!(k.key(KeyCode::KeyA, false), Ok(vec![0x9E]));
        assert_eq!(k.release_all(), [0xAA, 0xE0, 0xC8]);
        assert_eq!(k.release_all(), Vec::<u8>::new());
        assert_eq!(k.key(KeyCode::KeyB, false), Ok(vec![]), "stray release");
    }

    #[test]
    fn pause_does_not_repeat_and_ctrl_pause_is_break() {
        let mut k = Keyboard::default();
        let pause = vec![0xE1, 0x1D, 0x45, 0xE1, 0x9D, 0xC5];
        assert_eq!(k.key(KeyCode::Pause, true), Ok(pause.clone()));
        assert_eq!(k.key(KeyCode::Pause, true), Ok(vec![]), "no typematic");
        assert_eq!(k.key(KeyCode::Pause, false), Ok(vec![]));
        assert_eq!(k.key(KeyCode::Pause, true), Ok(pause));
        assert_eq!(k.key(KeyCode::Pause, false), Ok(vec![]));
        k.key(KeyCode::ControlRight, true).unwrap();
        assert_eq!(
            k.key(KeyCode::Pause, true),
            Ok(vec![0xE0, 0x46, 0xE0, 0xC6])
        );
        assert_eq!(k.release_all(), [0xE0, 0x9D]);
    }

    #[test]
    fn ctrl_alt_f12_is_a_host_shortcut() {
        let mut k = Keyboard::default();
        k.key(KeyCode::ControlLeft, true).unwrap();
        assert_eq!(k.key(KeyCode::F12, true), Ok(vec![0x58]));
        k.key(KeyCode::AltRight, true).unwrap();
        assert_eq!(k.key(KeyCode::F12, true), Err(Shortcut::NextPattern));
        assert_eq!(
            k.key(KeyCode::F12, false),
            Ok(vec![0xD8]),
            "earlier F12 make"
        );
    }
}
