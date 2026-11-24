# 027 — EGA/VGA planar modes, mode X, CGA graphics

- **Status:** in-progress
- **Milestone:** M2
- **Depends on:** 026
- **Owner:** worker-2

## Goal
The remaining graphics modes games use.

## Scope
- Planar read/write: write modes 0–3, read modes 0/1, map mask, bit mask,
  set/reset, rotate/function, latches.
- Modes 0Dh, 0Eh, 10h, 12h; unchained 256-color (mode X/Y, 320x240) incl.
  page flipping via start address and split screen (line compare).
- CGA modes 04h/05h/06h at `B8000` (interleaved), CGA palette registers
  (3D9h).
- INT 10h mode tables for all of the above.

## Acceptance criteria
- [ ] Tests: each write mode against a reference, mode X pixel addressing,
      CGA interleave, golden hashes per mode.

## Progress
Part 1: planar memory path (latches, write modes 0–3 with set/reset,
rotate, logical function, bit mask; read modes 0/1), 16-colour planar
renderer, INT 10h mode 12h (table, AH=0Ch/0Dh, glyphs, scroll).

Remaining: modes 0Dh/0Eh/10h tables (and their EGA palettes/char heights),
CGA modes 04h–06h (interleave, 3D9h), mode X helpers/tests (320x240),
golden hashes per mode.
