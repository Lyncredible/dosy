# 006 — wgpu presenter with integer scaling

- **Status:** todo
- **Milestone:** M1
- **Depends on:** 000
- **Owner:** —

## Goal
Draw a `VideoFrame` to a surface crisply: palette lookup, aspect-correct
integer scaling, padding and background color.

## Scope (`dosy-render`)
- Add `wgpu` (no winit). `Renderer::new(instance, surface, size)`,
  `resize(w, h)`, `upload(&VideoFrame)`, `render()`.
- Indexed `R8Uint` texture + 256x1 palette texture; WGSL shader does lookup.
- `layout.rs` (pure): given frame size, `display_aspect`, window size,
  padding, scale mode (`integer`/`fit`/`stretch`) → viewport rect. Integer
  mode picks the largest integer multiple of the aspect-corrected size that
  fits; falls back to fit when < 1x.
- Clear color from `RenderConfig { background, padding, scale }`.

## Acceptance criteria
- [ ] Unit tests for `layout.rs` (320x200@4:3, 720x400, odd window sizes,
      padding larger than window).
- [ ] Optional `#[ignore]` headless render test that reads back a texture.
- [ ] Re-uploading frames of the same size reuses textures.

## Notes
- Update the Linux apt step in CI if wgpu needs packages for tests.
- Keep shader source in `src/shaders/*.wgsl`.
