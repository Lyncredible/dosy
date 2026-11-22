# 037 — OPL2 FM synthesis

- **Status:** todo
- **Milestone:** M3
- **Depends on:** 034, 036
- **Owner:** —

## Goal
AdLib music.

## Scope
- OPL2 (YM3812) emulator at 49716 Hz: 9 channels × 2 operators, envelope
  generator, waveforms 0–3, feedback, AM/vibrato, rhythm mode, key scale.
- Implementation: port of a permissively licensed core (e.g. ymfm, BSD-3)
  or original code; credit the source. No GPL/LGPL code.
- Mixer channel fed from 036's register event stream, sample-accurate.
- OPL3 can be a later task.

## Acceptance criteria
- [ ] Tests: single sine operator at a known F-number produces the expected
      frequency; envelope attack/decay timing within tolerance; determinism.
- [ ] Split into two PRs if > ~500 lines (envelope/operators, then channels
      and rhythm).
