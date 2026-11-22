# 033 — Private game smoke test

- **Status:** todo
- **Milestone:** M2
- **Depends on:** 021, 026, 028, 029
- **Owner:** —

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

## Acceptance criteria
- [ ] Skips cleanly in CI; runs locally against a private zip.
- [ ] Failures print the trace dump from 024.
