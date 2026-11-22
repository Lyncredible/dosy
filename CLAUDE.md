# dosy — agent notes

- Read `docs/WORKFLOW.md` (team protocol) and `docs/ARCHITECTURE.md` (design)
  before working. Pick tasks from `TASKS.md`; each has a `tasks/NNN-*.md`.
- One task per PR on `task/NNN-slug`; never push to `main`.
- Before a PR: `cargo fmt --all`, `cargo clippy --workspace --all-targets -- -D warnings`,
  `cargo test --workspace`.
- `dosy-core`/`dosy-vfs`/`dosy-config` stay headless and deterministic.
- Never commit DOS software or name private test assets; smoke tests use
  `DOSY_SMOKE_ZIP`.
- Keep docs succinct.
