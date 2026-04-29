# 0018 · Just-Driven Config & Build Pipeline

| Field | Value |
|---|---|
| Status | Accepted |
| Date | 2026-04-29 |
| Deciders | fannnzhang |

## Context

Configuration was scattered across five surfaces (backend clap-env,
daemon `option_env!`, mobile FFI `option_env!`, Flutter `--dart-define`,
per-developer shell + Xcode scheme) with no single entry point. The
mobile build silently baked `localhost` into release artifacts because
`Runner.xcscheme`'s `<EnvironmentVariables>` block applied at LaunchAction
time, after cargokit's BuildAction-time Rust compile had already resolved
`option_env!("MINOS_BACKEND_URL")` to its fallback. Cloudflare Access
service-token secrets ended up in plaintext in the xcscheme.

## Decision

A single workspace-root `.env.local` is the source of truth, loaded by
a `justfile` that is the **only** sanctioned entry point for build and
run commands. Recipes export the loaded vars to subprocess environments
(cargo, flutter, xcodebuild). The Runner Xcode target gets a Pre-Build
Run Script Phase that fails the build if `MINOS_BUILD_VIA_JUST=1` is
absent — guaranteeing IDE Build/Run can't bypass the env-injection.
Mobile FFI and daemon `build.rs` panic on `release` builds with
`MINOS_BACKEND_URL` unset.

Secrets live in `.env.local` (gitignored) for developers and in GitHub
Secrets for CI. The xcscheme's `<EnvironmentVariables>` block is empty;
no plaintext credentials in the source tree.

Default-string consolidation: three duplicate `ws://127.0.0.1:8787/devices`
fallbacks collapse into `minos_domain::defaults::DEV_BACKEND_URL`.

Dead vars (`MINOS_BACKEND_PUBLIC_URL`, `MINOS_BACKEND_CF_ACCESS_*`)
removed from active source and ops docs; historical ADRs (0014, 0016)
keep their references with banners pointing here.

## Consequences

Positive:

- One file to edit per environment switch.
- The localhost-baking bug cannot recur: build fails fast in release,
  warns loudly in debug, and the IDE-direct path is blocked entirely.
- CF Access secrets do not enter the source tree.
- A new contributor's onboarding becomes `cp .env.example .env.local
  && just backend` — no shell rcfile editing, no Xcode poking.

Negative:

- Adds `just` as a hard dependency (mitigated by ubiquitous availability:
  `brew install just`, `cargo install just`).
- Disallows the Xcode IDE Build/Run muscle memory; requires either a
  custom Xcode scheme runner or terminal-driven workflow.
- Pbxproj contains a hand-written PBXShellScriptBuildPhase block; if
  Flutter's build tooling regenerates pbxproj it could clobber the
  guard. Flutter currently does not regenerate Runner.pbxproj on
  ordinary builds (only `flutter create` would), so this is low-risk
  but worth flagging.

## Alternatives considered

- **`cargo xtask` extension to wrap flutter and xcodebuild.** Rejected:
  xtask is Rust-only by convention, and pulling in flutter/xcodebuild
  invocation logic muddles its scope. just is polyglot by design.
- **Multi-environment manifests (`envs/{local,staging,prod}.env`).**
  Deferred (design doc §2.2): single `.env.local` is sufficient for
  current workflows.
- **Login-page Server-URL field for runtime override.** Rejected:
  the URL is needed for the very first request (login), so a runtime
  override would have to live before authentication, adding an
  onboarding step.
- **Pre-Build hook as a build-script-only check (no Xcode integration).**
  Rejected: the build script doesn't run for IDE Build until cargokit
  runs, by which time the bake has already happened. The Xcode-level
  hook fires before any Rust source compiles.

Refines (does not replace) ADR 0013, 0014, 0016 — those decisions about
what gets baked into the binary remain in force; this ADR specifies
*how* the baking is invoked and where the values live.
