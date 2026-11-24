# 029 — Batch files

- **Status:** done
- **Milestone:** M2
- **Depends on:** 021
- **Owner:** worker-3

## Goal
Run `.BAT` files the way COMMAND.COM does.

## Scope
- Batch frame stack (nested `CALL`, chaining without CALL replaces frame).
- `@` prefix, `ECHO ON/OFF/.`, `REM`, `::`, labels + `GOTO`,
  `IF [NOT] ERRORLEVEL n / EXIST f / s1==s2`, `%0–%9`, `SHIFT`, `%VAR%`,
  `%%` escaping, `FOR %%x IN (set) DO cmd`, `PAUSE`, `CHOICE` (basic).
- Batch reads the file line by line from the drive (file may change).
- Programs run from batch return to the next line with ERRORLEVEL set.
- `autoexec` config lines executed as an implicit batch at boot.

## Notes
- `shell/batch.rs`: frames (CALL pushes, chaining replaces), the file is
  reopened per line. Autoexec lines stay "typed" lines; a `.BAT` among them
  runs to completion, then the next line runs.
- ECHO's first separator (space or tab) is eaten, as in DOS; tabs expand
  to 8 columns on output.
- CHOICE is basic: `/C:keys`, `/N`; `/T` timeouts are ignored.

## Acceptance criteria
- [x] Headless tests: `@echo off` + `cls` + running two COM programs in
      sequence + `echo` output; GOTO loops; IF ERRORLEVEL after a child
      exits with code; parameters and SHIFT.
