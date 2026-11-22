# 040 — Themes and window chrome

- **Status:** todo
- **Milestone:** M4
- **Depends on:** 012
- **Owner:** —

## Goal
Ghostty-style look: minimal chrome, themes, padding, smooth resize.

## Scope
- Theme files (`theme = name`, search built-in then config dir `themes/`):
  background, padding color, optional text-mode 16-color palette override.
  Ship a few built-in themes (default, light, solarized-like).
- `window-padding-x/y`, `window-padding-balance`, `window-decorations`,
  initial window size from frame size × scale, HiDPI scale factor.
- Smooth resize: present on every resize event, no stretching artefacts.
- Title shows running program name.

## Acceptance criteria
- [ ] Config tests for theme parsing/merge; layout tests with HiDPI factors.
- [ ] Visual check on Windows; CI builds on Linux/macOS.
