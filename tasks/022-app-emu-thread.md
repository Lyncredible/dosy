# 022 — Emulation thread, pacing and CLI

- **Status:** in-progress
- **Milestone:** M1
- **Depends on:** 012, 013
- **Owner:** worker-2

## Goal
Run `Machine` on its own thread at real-time speed and show its frames.

## Scope (`dosy`)
- Threading per `docs/ARCHITECTURE.md`: `HostEvent` mpsc to the emu thread,
  latest-wins frame mailbox, `EventLoopProxy` redraw wake.
- Pacing: ~1 ms ticks, `cycles_per_ms × elapsed`, cap catch-up at e.g.
  50 ms; `cycles = max` runs flat out with frame-rate-limited presents.
- Host RTC injected at startup (`set_rtc`).
- CLI: `dosy [PATH]` mounts PATH as C: (dir/zip/file) and runs an optional
  `-c "command"`; config `mount`/`autoexec` keys applied.
- Clean shutdown on window close; fault → message in title + pause.

## Acceptance criteria
- [ ] Boots to the shell prompt in a window (once 021 has landed; before
      that, shows the BIOS text screen).
- [x] Unit tests for the pacing calculator and mailbox.

## Progress
Done (PR 1): emu thread (`emu.rs`), `HostEvent` channel, latest-wins
`Mailbox`, `EventLoopProxy` wake, `Pacer` (1 ms ticks, 50 ms catch-up cap),
`cycles = max` (new `Cycles` config value), host RTC (UTC; no local offset
in std), `dosy [PATH] -c cmd` → `EmuSetup { mounts, autoexec }` merged with
config, clean shutdown, stop + title message on fault/halt.

Remaining (blocked):
- Mount `EmuSetup.mounts` once host/zip drives exist (011, 028).
- Run `EmuSetup.autoexec` in the shell (021).
- Publish `Machine` frames at vsync instead of test patterns (010); then
  rate-limit presents in `cycles = max`.
