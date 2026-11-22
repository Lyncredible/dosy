# 012 — App window: winit + renderer + config

- **Status:** done
- **Milestone:** M1
- **Depends on:** 005, 006
- **Owner:** worker-2

## Goal
`cargo run -p dosy` opens a window showing a test-pattern `VideoFrame`.

## Scope (`dosy`)
- winit event loop (`ApplicationHandler`), create surface, `dosy-render`
  renderer, redraw on resize/expose.
- Load config via `dosy-config` + CLI overrides; map to `RenderConfig`.
- Test pattern: 320x200 palette bars, and a 720x400 text-like pattern.
- Minimal CLI: `--version`, `--config <file>`, `--key=value`.
- Add Linux apt packages to CI if needed.

## Acceptance criteria
- [x] Window opens on Windows; builds on Linux/macOS in CI.
- [x] Resizing keeps crisp integer scaling and padding color.
- [x] Unit test for CLI parsing.

## Notes
- Space cycles the two patterns (placeholder until 022/042).
- `aspect = false` shows square pixels (frame `display_aspect = (0, 0)`).
- Padding is in points, multiplied by the window scale factor.
