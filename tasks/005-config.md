# 005 — Ghostty-style config parser

- **Status:** todo
- **Milestone:** M1
- **Depends on:** 000
- **Owner:** —

## Goal
Parse dosy's plain-text config the way Ghostty does into a typed `Config`.

## Scope (`dosy-config`, no deps)
- Syntax per `docs/ARCHITECTURE.md`: `key = value`, `#` comments, quoted
  values, repeatable list keys, empty value resets, `config-file` includes
  (relative to including file, `?` optional, cycle detection).
- `Config` with defaults for the initial keys listed in the architecture doc;
  typed values (`Color` `#rrggbb`, `Scale` `integer|fit|stretch`, bools,
  numbers, `Mount { letter, path }`).
- `Config::load_default()` from the platform path (env-var based, no `dirs`
  crate needed), `apply_cli(args)` for `--key=value` overrides.
- Diagnostics: `Vec<Diagnostic { file, line, message }>`; unknown keys and
  bad values warn, never abort.

## Acceptance criteria
- [ ] Tests: each value type, list append/reset, include + optional include +
      cycle, CLI override order, diagnostics with line numbers.

## Notes
- Mirror Ghostty key naming (kebab-case). Keep the key table in one place so
  `dosy +show-config`-style dumping is easy later.
