---
name: minos-quality-gate
description: Run and remediate Minos CI-equivalent quality gates for a completed change. Use after any code, test, schema, codegen, dependency, configuration, documentation, comment, rename, or deletion change and before every commit, push, PR update, or handoff; use again when addressing a CI failure.
---

# Minos local quality gate

Use this as the required final phase of a change. CI confirms a locally green
change; it is not a discovery loop. Do not hand off, commit, or push while a
selected gate has failed, been skipped by a missing prerequisite, or has not
been run.

`docs/ci-gates.md` is the local-gate contract. Read it before selecting
commands. Inspect `.github/workflows/ci.yml` for CI-only setup and exact steps
such as SQLx metadata, the mobile code-generator, or the macOS desktop check.
If the two disagree, resolve the current contract at its owner and update the
documentation in the same change; do not invent a third command list here.

## 1. Determine the final change surface

Inspect the complete working tree, including staged and untracked files:

```bash
git status --short
git diff --name-only
git diff --cached --name-only
git ls-files --others --exclude-standard
```

Classify every in-scope changed path, including deletions, generated files,
comments, and renames. Do not validate only the last file edited. Preserve
unrelated user changes; a whole-tree gate can expose their failure, but do not
silently fix or discard them. Report such a failure as not green rather than
claiming the task passed.

Changes to a comment, a name, or a deleted file retain the gate of their source
language/surface. For example, a Rust doc-comment change can require rustfmt,
clippy, or FRB regeneration; a Desktop rename still requires the file-size
gate.

## 2. Run every applicable gate

Use the composite commands below rather than substituting a convenient subset.
They preserve the CI phase order and, for Rust, `clippy --keep-going`.

| Changed surface | Required local evidence |
| --- | --- |
| Rust workspace: root Cargo files, `crates/**`, `xtask/**`, `schemas/**`, migrations, `.sqlx/**`, `justfile` | `cargo xtask check-rust`. This includes fmt, naming/docs lints, backend platform-contract drift, schema parity, clippy with warnings denied, workspace tests, and daemon `test-support` tests. For `minos-backend`, migrations, SQL queries, or `.sqlx`, also reproduce the workflow's `cargo sqlx prepare --check --workspace -- -p minos-backend --all-targets` step with its SQLite `DATABASE_URL`; install the workflow-pinned `sqlx-cli` first if necessary. Run `just check-backend-pg` for migration/storage-parity changes when its local Postgres prerequisite is available. |
| Desktop TypeScript, Desktop config/dependencies, or `apps/desktop/src-tauri/**` | `cd apps/desktop && pnpm check:all`; on macOS also run `cargo check -p minos-desktop`. `check:all` is mandatory because it includes tests, Biome, file-size, and px-text gates. If the dependency manifest or lockfile changed, first use `pnpm install --frozen-lockfile`. |
| Web TypeScript/config/dependencies under `apps/web/**` | `cd apps/web && pnpm check`. If the dependency manifest or lockfile changed, first use `pnpm install --frozen-lockfile`. |
| Mobile Dart, mobile dependencies, FRB config, `minos-mobile`, or `minos-ffi-frb` | Run `cargo xtask check-all` after confirming that `fvm` and `flutter_rust_bridge_codegen` are available; do not count its toolchain-dependent skip as a pass. Also reproduce the mobile CI sequence from `.github/workflows/ci.yml`: `flutter pub get`; `dart run build_runner build --delete-conflicting-outputs`; `dart format --set-exit-if-changed lib test`; `dart analyze --fatal-warnings`; and `flutter test --exclude-tags ffi`. Use the workflow's temporary plugin-disable step for analysis, not a permanent `analysis_options.yaml` edit. Regenerate FRB with `cargo xtask gen-frb` (or the workflow command) and verify a second generation produces no change. |
| Backend platform contract/OpenAPI/WebSocket schema | `cargo xtask check-rust`; its static phase runs `gen-backend-platform-contract --check`. Do not manually edit generated artifacts to mask drift. |
| Project documentation or agent instructions only | `cargo xtask lint-docs` when documentation paths are affected. For a skill change, run that skill's declared validator as well. |

For a cross-surface change, run the union of rows. `cargo xtask check-all` is
the preferred aggregate on a prepared macOS/mobile machine, but it does not
replace `pnpm check:all` for Desktop or `pnpm check` for Web.

Before running FRB drift against Git, distinguish intended generated edits from
new generator output: generate once, include the intended artifacts in the
change, then generate again. The second run must leave the generated roots
unchanged. On a clean final commit, also run the workflow's `git diff
--exit-code` check for `apps/mobile/lib/src/rust` and
`crates/minos-ffi-frb/src/frb_generated.rs`, plus check for untracked output.

## 3. Fix and re-run to green

Treat every failure as a defect in the final change until evidence proves it is
an unrelated pre-existing worktree issue. Trace the owning code or contract,
make the smallest correct repair, regenerate committed artifacts when required,
and re-run the failed gate. Then re-run all selected composite gates so an
earlier failure does not hide a later one. Do not waive a lint, loosen an
allowlist, suppress a test, or leave generated drift merely to make a gate
green.

If a required platform/toolchain is unavailable, install/use the repository's
pinned prerequisite when that is in scope. Otherwise report the gate as
unavailable and request the needed environment; do not call the work complete
or imply that CI will cover it.

## 4. Hand off with evidence

State the complete change surfaces and each selected command with its exit
result. Mark non-applicable gates as `not applicable` with the path-based
reason. List any unavailable gate or unrelated worktree failure explicitly.
Only describe the change as complete when every applicable gate exited zero.

Example:

```text
Quality gates
- Rust workspace + backend contract: cargo xtask check-rust — passed
- Desktop: apps/desktop pnpm check:all — passed
- Mobile/Web: not applicable (no paths changed)
```
