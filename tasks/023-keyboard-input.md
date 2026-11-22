# 023 — Host keyboard to scancodes

- **Status:** todo
- **Milestone:** M1
- **Depends on:** 009, 022
- **Owner:** —

## Goal
Typing in the window reaches the guest as PC set-1 scancodes.

## Scope (`dosy`)
- `input/keymap.rs`: winit `KeyCode` (physical keys) → set-1 make/break
  bytes incl. E0 extended keys, Pause (E1 sequence), PrintScreen.
- Key repeat: host repeat events forwarded as repeated makes.
- Release all held keys on focus loss.
- Reserve host shortcuts (none yet; 042 adds keybinds).

## Acceptance criteria
- [ ] Table-driven unit tests covering letters, digits, F1–F12, arrows,
      keypad, modifiers, extended keys.
- [ ] Typing at the prompt works in the app.
