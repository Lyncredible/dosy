# 044 — Config parser polish

- **Status:** todo
- **Milestone:** M4
- **Depends on:** 005
- **Owner:** —

## Goal
Close the small Ghostty-compatibility gaps found when 005 was reviewed.

## Scope (`dosy-config`)
- A bare `--` ends option parsing; later args are positional.
- An unreadable top-level config file reports line 0, not line 1. Match the
  missing-include diagnostics.
- `config-file =` with an empty value clears the include list, as in
  Ghostty.

## Acceptance criteria
- [ ] One unit test per item; existing tests still pass.
