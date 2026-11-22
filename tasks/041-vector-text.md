# 041 — Vector-font text mode

- **Status:** todo
- **Milestone:** M4
- **Depends on:** 006, 010
- **Owner:** —

## Goal
Optionally render DOS text mode with a crisp TrueType font at any size.

## Scope (`dosy-render`)
- `text-render = vector`: render `TextScreen` cells with a glyph atlas
  (`fontdue` or `swash`), CP437 → Unicode mapping incl. box drawing (draw
  box/block glyphs procedurally for seamless lines).
- `font-family` / `font-size`; fallback to a bundled permissively licensed
  font; cell grid sized to keep 4:3 or fill (config).
- Cursor and blink from `TextScreen`; falls back to bitmap path in
  graphics modes.

## Acceptance criteria
- [ ] Unit tests: CP437 mapping table, atlas packing, cell layout math.
- [ ] Box-drawing characters connect without gaps at fractional scales.
