# 033 — Private game smoke test

- **Status:** done
- **Milestone:** M2
- **Depends on:** 021, 026, 028, 029
- **Owner:** worker-3

## Goal
A repeatable headless check that a real zipped DOS game boots and draws.

## Scope
- `dosy-core/tests/smoke.rs` (or app-level test): read `DOSY_SMOKE_ZIP`
  (skip with a message when unset), mount it read-only + memory overlay as
  C:, run `DOSY_SMOKE_CMD` (a command line for the shell) for
  `DOSY_SMOKE_SECS` emulated seconds (default 30), inject optional keys from
  `DOSY_SMOKE_KEYS`.
- Assert: no `RunExit::Fault`, no unimplemented-opcode warnings, at least one
  graphics-mode frame; write the final frame as PNG under `target/smoke/`.
- Document usage in the test file only; never name or path a specific game.

## Notes
- `crates/dosy-core/tests/smoke.rs`; usage is in its header. Keys are
  `SECS=KEY` pairs; `DOSY_SMOKE_PNG` overrides the PNG path;
  `DOSY_SMOKE_ALLOW_TEXT=1` accepts text-only output.
- Warnings are captured through `log`; any mentioning an opcode fails the
  test, the rest are listed. Failures print a CPU dump (CS:IP, registers,
  code bytes, trace history) via 024's `Machine::fault_report`.

## Acceptance criteria
- [x] Skips cleanly in CI; runs locally against a private zip.
- [x] Failures print the trace dump from 024.
