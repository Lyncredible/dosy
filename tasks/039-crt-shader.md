# 039 — Optional CRT shader

- **Status:** todo
- **Milestone:** M4
- **Depends on:** 006
- **Owner:** —

## Goal
A tasteful, optional CRT look.

## Scope (`dosy-render`)
- Post pass on the scaled image: scanlines, slight bloom, mask, curvature
  (each configurable, off by default: `crt = false`, `crt-scanlines`,
  `crt-curvature`, `crt-mask`).
- Works at any integer scale; disabled path costs nothing.
- Custom WGSL shader file via `custom-shader = path` (Ghostty-like).

## Acceptance criteria
- [ ] Layout/uniform tests; headless render test `#[ignore]`.
- [ ] Toggle at runtime without recreating the device.

## Review follow-ups
- [ ] `Renderer::upload` rewrites the palette texture every frame. Skip the
      write when the palette is unchanged (compare it with the previous one or
      keep a dirty flag).
