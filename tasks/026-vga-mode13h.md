# 026 — VGA mode 13h and palette

- **Status:** done
- **Milestone:** M2
- **Depends on:** 010, 016
- **Owner:** worker-1

## Goal
320x200x256 graphics, the most common DOS game mode.

## Scope
- Chain-4 addressing at `A0000`, line compare/start address, 200-line
  double scan → `VideoFrame` 320x200 with `display_aspect` 4:3.
- INT 10h AX=0013h register tables; mode 13h-specific BIOS behavior
  (AH=0C/0D pixel write/read, teletype in graphics).
- DAC palette changes mid-frame: latch at retrace (good enough).
- Palette fade idioms (rapid 3C8/3C9 writes during vretrace) work.

## Acceptance criteria
- [x] Tests: pixel writes via bus appear in frame; DAC 6→8 bit expansion;
      golden hash of a test pattern; mode switch 3 → 13h → 3 restores text.

## Notes
- `hw/vga/gfx.rs` renders any 256-colour mode from the CRTC address
  counter (doubleword/word/byte mode, start address, offset, line compare),
  so unchained mode X already displays; 027 adds its write paths/latches.
- Duplicate scanlines are collapsed (13h → 320x200); the DAC is sampled at
  vretrace. Horizontal panning is ignored.
