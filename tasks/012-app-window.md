# 012 — App window: winit + renderer + config

- **Status:** todo
- **Milestone:** M1
- **Depends on:** 005, 006
- **Owner:** —

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
- [ ] Window opens on Windows; builds on Linux/macOS in CI.
- [ ] Resizing keeps crisp integer scaling and padding color.
- [ ] Unit test for CLI parsing.
