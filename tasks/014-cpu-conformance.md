# 014 — CPU conformance tests (SingleStepTests)

- **Status:** todo
- **Milestone:** M1
- **Depends on:** 007
- **Owner:** —

## Goal
Validate the CPU against the SingleStepTests 8088 (and later 80286 real
mode) JSON suites.

## Scope
- Integration test in `dosy-core/tests/` that reads gzipped JSON test files
  from `DOSY_CPU_TESTS` (skip when unset), sets up `FlatBus` + registers,
  steps once, compares registers/memory (masking undefined flags per the
  suite's metadata).
- `scripts/fetch-cpu-tests.{sh,ps1}` downloading the suite into
  `target/cpu-tests/` (not committed).
- Dev-dependencies only (`serde_json`, `flate2`).
- Report: per-opcode pass/fail summary; an allowlist of known failures.

## Acceptance criteria
- [ ] All 8086 opcodes implemented in 007 pass or are listed with a reason.
- [ ] Optional CI job (non-blocking) that fetches and runs the suite.
