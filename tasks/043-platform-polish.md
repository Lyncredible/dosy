# 043 — Linux/macOS polish and release artifacts

- **Status:** todo
- **Milestone:** M5
- **Depends on:** 023, 035
- **Owner:** —

## Goal
First-class Linux and macOS builds and downloadable releases.

## Scope
- Linux: X11 + Wayland verified, keyboard via physical keycodes, ALSA/Pulse
  via cpal, config path per XDG.
- macOS: `.app` bundle (icon, Info.plist), Cmd shortcuts, Retina scaling,
  Option key handling.
- Release workflow on tag `v*`: build Windows (zip), Linux (tar.gz),
  macOS (universal or per-arch zip); attach to GitHub release.

## Acceptance criteria
- [ ] Release workflow produces artifacts for all three OSes on a test tag.
- [ ] Smoke run on each OS documented in the PR (manual).
- [ ] Release artifacts include a `THIRD_PARTY`/`NOTICE` file covering
      bundled and linked third-party code. That includes the Spleen font
      (BSD-2-Clause) and the zip, zlib-rs, wgpu and winit licenses; generate
      the list (e.g. `cargo about`) rather than hand-maintain it.
