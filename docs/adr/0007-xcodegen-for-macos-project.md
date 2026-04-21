# 0007 · XcodeGen for the macOS project

| Field | Value |
|---|---|
| Status | Accepted |
| Date | 2026-04-21 |
| Deciders | fannnzhang |

## Context

Plan 02 adds a macOS MenuBarExtra app with one application target, one unit-test target, generated UniFFI Swift sources, and a universal Rust static library linked from Xcode. The repo needs the project definition to be:
- reviewable in code review;
- regenerable from a single command in local development and CI;
- compatible with generated paths such as `apps/macos/Minos/Generated/` and `target/xcframework/`;
- light enough to fit a repo that already centralizes build orchestration in `xtask`.

Hand-authoring `project.pbxproj` would push opaque Xcode diffs into routine review and make generated-path changes harder to reason about. The app surface is still small, so a heavier project-management layer is unnecessary.

## Decision

Use XcodeGen as the source of truth for the macOS project.

- Commit `apps/macos/project.yml`.
- Generate `apps/macos/Minos.xcodeproj` via `cargo xtask gen-xcode` in both local development and CI.
- Keep the generated `.xcodeproj` out of git.
- Store the required app and test target settings in `project.yml`, including `LSUIElement`, bundle identifiers, module-map/header search paths, and the Rust static-library search path.

## Consequences

**Positive**
- The macOS project shape is reviewed as YAML rather than pbxproj churn.
- Local development and CI use the same regeneration command before `xcodebuild`, which keeps project state reproducible.
- UniFFI-generated headers/module maps and the Rust universal archive stay wired through one checked-in file.
- Future target or build-setting changes stay diff-friendly.

**Neutral**
- macOS contributors need XcodeGen installed. This is absorbed into `apps/macos/Brewfile` and `cargo xtask bootstrap`.
- The generated `.xcodeproj` remains ephemeral; after project-shape changes, developers regenerate it rather than editing it in place.

## Alternatives Rejected

### Hand-authored `.xcodeproj`

Rejected:
- `project.pbxproj` diffs are noisy and easy to accidentally damage.
- Generated-source and linker-path updates are harder to audit when buried inside Xcode-managed project state.
- CI would need to trust a checked-in generated project or depend on manual synchronization.

### Tuist

Rejected:
- Adds another project graph tool and bootstrap surface to a repo that already uses `xtask` for build orchestration.
- The current app graph is simple enough that Tuist's extra abstraction is unnecessary.
- XcodeGen already solves the concrete need: deterministic project generation from a checked-in text spec.