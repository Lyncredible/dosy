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

Done (PR 2): `Machine::frame()` published when its `seq` changes (vsync),
at most every 14 ms in `cycles = max`; test patterns removed. Mounts open
in the main thread via `open_mount` (a bad PATH exits 2, bad config mounts
warn); `.zip` works once 028 extends `open_mount`. `cpu` → `cpu_model`.
Interim until 021: the first non-`X:` autoexec/`-c` line runs via
`Machine::exec`.

Remaining:
- After 021: replace `run_first_program` with
  `machine.start_shell(&setup.autoexec)` (agreed with 021's author), then
  check off "boots to the shell prompt".
