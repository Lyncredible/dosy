# Team workflow

Roles: **lead** assigns tasks, **workers** implement them, **reviewer**
reviews and merges. Tasks live in `tasks/NNN-slug.md`, indexed by `TASKS.md`.

## Rules

- One task per PR. Keep PRs small (a few hundred lines). Split a task that
  grows; note the remainder in the task file.
- Never push to `main` (a ruleset enforces it). Branch: `task/NNN-slug`,
  matching the task file name.
- Only start tasks whose dependencies are `done`.
- Stay inside the task's scope and the module boundaries in
  `docs/ARCHITECTURE.md`. Interface changes that affect other tasks need a
  short note in `docs/ARCHITECTURE.md` in the same PR.
- Never commit DOS software, disk images or zips, and never name or path
  private test assets in the repo, commits, PRs or issues. Smoke tests read
  `DOSY_SMOKE_ZIP` and skip when it is unset.
- New dependencies: permissive licenses only; justify in the PR body.

## Doing a task

1. `git fetch origin && git switch -c task/NNN-slug origin/main`
2. Implement with tests that cover the acceptance criteria.
3. Update the task file `Status:` and its `TASKS.md` row to `done` (set
   `in-progress` instead if the PR only partly completes it), and fill in
   `Owner`.
4. `git fetch origin && git rebase origin/main`, then run:
   ```
   cargo fmt --all
   cargo clippy --workspace --all-targets -- -D warnings
   cargo test --workspace
   ```
5. Push and open the PR:
   `gh pr create --base main --title "[NNN] Title" --body "..."`.
   Body: `Task: tasks/NNN-slug.md`, a short summary, how it was tested, any
   follow-ups.
6. Watch CI (`gh pr checks <n> --watch`); fix failures on all three OSes.

Status values: `todo`, `in-progress`, `in-review`, `done`. `main` only shows
merged state, so PRs normally land as `done`; the lead may mark
`in-progress`/`in-review` and owners in a tracking-only PR.

## Review

The reviewer checks scope, acceptance criteria, tests, architecture fit and
green CI, then **squash-merges** (title `[NNN] Title`) and deletes the branch,
or requests changes. Conflicts (often in `TASKS.md`) are resolved by the
author rebasing onto `origin/main`.

## Toolchain

- `rust-toolchain.toml` pins `stable` with rustfmt + clippy; CI uses it on
  windows-latest, ubuntu-latest and macos-latest.
- Windows dev machine: default `stable-x86_64-pc-windows-msvc` toolchain.
  Cargo lives in `%USERPROFILE%\.cargo\bin` and may be missing from `PATH`
  in new shells; prepend it:
  - PowerShell: `$env:Path = "$env:USERPROFILE\.cargo\bin;$env:Path"`
  - bash: `export PATH="$USERPROFILE/.cargo/bin:$PATH"`
- Linux needs system packages once windowing/audio land (see the comment in
  `.github/workflows/ci.yml`).
