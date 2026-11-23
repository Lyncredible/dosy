# 049 — VGA accuracy polish

- **Status:** todo
- **Milestone:** M4
- **Depends on:** 010
- **Owner:** —

## Goal
Close the VGA gaps left after 010.

## Scope (`hw/vga/`)
- Blank the screen (border color) while attribute-controller index bit 5
  (PAS) is cleared.
- Monochrome underline (CRTC 14h underline location, attribute 01h in
  mono modes).
- Cursor skew (CRTC 0Bh bits 5–6) and the cursor-start disable bit.
- Horizontal pixel panning (attribute register 13h).
- 8x8 font from a proper permissively licensed source (record source +
  license), and an 8x14 font for EGA-style modes and INT 10h/1130h.

## Acceptance criteria
- [ ] One test per item (frame golden hashes or register-driven checks).
