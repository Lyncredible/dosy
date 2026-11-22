# 042 — Keybinds and actions

- **Status:** todo
- **Milestone:** M4
- **Depends on:** 005, 022
- **Owner:** —

## Goal
Ghostty-style `keybind = trigger=action` for host actions.

## Scope
- Actions: `toggle_fullscreen`, `pause`, `reset`, `screenshot` (PNG of the
  current frame), `toggle_crt`, `reload_config`, `release_mouse`,
  `cycles_up/down`, `quit`.
- Defaults per platform (e.g. Alt+Enter fullscreen on Windows/Linux,
  Cmd+Enter on macOS); bound keys are not sent to the guest.
- Config reload applies render settings live and machine settings where safe.

## Acceptance criteria
- [ ] Parser tests for trigger syntax (`ctrl+alt+f`, `super+enter`) and
      `keybind = clear`.
- [ ] Each action works in the app.
