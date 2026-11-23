# 017 — DOS kernel core

- **Status:** done
- **Milestone:** M1
- **Depends on:** 002
- **Owner:** worker-4

## Goal
The HLE DOS kernel's memory model and INT 21h dispatcher.

## Scope (`dos/`, ids `0x01xx`)
- `Dos` struct; `install<B: Bus>(bus)`: INT 20h/21h/2Fh stubs, DOS data in
  low memory (List of Lists, InDOS, SDA minimal), first MCB, environment.
- MCB chain in guest memory; INT 21h 48/49/4A (allocation strategies basic,
  largest-block reporting), 58h get/set strategy.
- PSP creation (`create_psp`), 50/51/62 set/get PSP, 25/35 vectors, 30h
  version (5.0), 33h break flag, 2A–2D date/time (from BDA ticks + kept
  date), 4Ch/00h terminate + 4Dh return code (process stack here, EXEC in
  019), 1Ah/2Fh DTA.
- Unknown functions: log once, CF=1 AX=1.

## Acceptance criteria
- [x] `FlatBus` tests: allocate/free/resize incl. coalescing and error
      codes (7, 8, 9), ≥ 600 KiB free after install, PSP fields
      (INT 20 at offset 0, top of memory, env segment), vectors, version.
