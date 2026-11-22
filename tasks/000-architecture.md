# 000 — Architecture docs and workspace skeleton

- **Status:** done
- **Milestone:** M0
- **Depends on:** —
- **Owner:** architect

## Goal
Give workers a shared plan and a compiling workspace to start from.

## Scope
- `docs/ARCHITECTURE.md`, `docs/WORKFLOW.md`, `TASKS.md`, `tasks/*.md`,
  `README.md`, `CLAUDE.md`.
- Workspace with five crate stubs; `dosy-core` defines `Bus` + `FlatBus`,
  `hle` callback contract, `VideoFrame`, `AudioSink`.
- CI: fmt, clippy `-D warnings`, test on Windows / Linux / macOS.

## Acceptance criteria
- [x] `cargo build/clippy/test --workspace` pass locally and in CI.
- [x] `cargo run -p dosy` prints the version.
