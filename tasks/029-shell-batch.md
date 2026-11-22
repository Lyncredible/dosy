# 029 — Batch files

- **Status:** todo
- **Milestone:** M2
- **Depends on:** 021
- **Owner:** —

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

## Acceptance criteria
- [ ] Headless tests: `@echo off` + `cls` + running two COM programs in
      sequence + `echo` output; GOTO loops; IF ERRORLEVEL after a child
      exits with code; parameters and SHIFT.
