# 027 — EGA/VGA planar modes, mode X, CGA graphics

- **Status:** todo
- **Milestone:** M2
- **Depends on:** 026
- **Owner:** —

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
