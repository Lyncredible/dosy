# 023 — Host keyboard to scancodes

- **Status:** done
- **Milestone:** M1
- **Depends on:** 009, 022
- **Owner:** worker-3

## Goal
Typing in the window reaches the guest as PC set-1 scancodes.

## Scope (`dosy`)
- `input/keymap.rs`: winit `KeyCode` (physical keys) → set-1 make/break
  bytes incl. E0 extended keys, Pause (E1 sequence), PrintScreen.
- Key repeat: host repeat events forwarded as repeated makes.
- Release all held keys on focus loss.
- Reserve host shortcuts (none yet; 042 adds keybinds).

## Notes
- `input::Keyboard` tracks held keys (focus loss releases them; stray
  releases are dropped); repeats are extra makes. Keys go to the emu thread
  as `HostEvent::Key(bytes)` → `Machine::key_event`.
- The debug pattern switch moved from Space to Ctrl+Alt+F12 (the only host
  shortcut until 042).
- "Typing at the prompt" can only be checked in the app once the window
  shows VGA output and the shell (021) runs; the path is covered by core
  tests from 009 (`key_event` → INT 09h → INT 16h).

## Acceptance criteria
- [x] Table-driven unit tests covering letters, digits, F1–F12, arrows,
      keypad, modifiers, extended keys.
- [ ] Typing at the prompt works in the app.
