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

## Review follow-ups
- The ALU (001) follows 186+/386 semantics: IDIV accepts the quotient
  -2^(n-1), shift and rotate counts are masked to 5 bits, and undefined flags
  follow DOSBox. 8088 vectors will mismatch on these, so either keep a
  per-opcode exception list or flag mask for 8088, or prefer the 80286 / 386
  real-mode vectors as the primary suite.
