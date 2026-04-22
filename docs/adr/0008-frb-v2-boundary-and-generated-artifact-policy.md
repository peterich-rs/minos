# 0008 · frb v2 boundary and generated-artifact policy

| Field | Value |
|---|---|
| Status | Accepted |
| Date | 2026-04-22 |
| Deciders | fannnzhang |

## Context

Plan 03 wires the Flutter app at `apps/mobile/` to the Rust core through
flutter_rust_bridge (frb) v2. The concrete question is how the Rust/Dart
boundary is drawn and how the generated artifacts are managed. Three
pressures shape the design:

1. **Hexagonal purity.** The composition-root crate `minos-mobile`
   constructs a `MobileClient` from transport + store + pairing ports
   (see spec §5.3). It must not depend on any specific FFI toolchain so
   the same crate can be hosted by frb today, UniFFI tomorrow, or a
   future wasm adapter — exactly mirroring how plan 02 kept
   `minos-daemon`'s app layer free of UniFFI imports.
2. **CI surface.** The repo already runs `dart analyze` and `flutter
   test` on `ubuntu-latest`. Forcing that lane to also host a full Rust
   toolchain and an frb codegen round-trip just to run `dart analyze`
   would double CI time and punish first-time contributors. Plan 02
   answered the analogous question on the UniFFI side by checking in
   the generated Swift; we mirror that choice here (spec §10.3).
3. **Two FFI adapters coexisting.** The repo ships both
   `minos-ffi-uniffi` (for the macOS app) and `minos-ffi-frb` (for the
   Flutter app). frb and UniFFI cannot share a single cdylib — each
   emits its own registration machinery — so they live as sibling
   adapter crates, each depending on `minos-mobile` as a normal
   library.

## Decision

### Boundary

- **`minos-mobile` does not depend on `flutter_rust_bridge`.** The sole
  frb-aware crate is `crates/minos-ffi-frb/`, which re-exports the
  `minos-mobile` surface behind frb macros.
- **`MobileClient` crosses as `#[frb(opaque)]`.** Dart sees it as an
  opaque handle; lifecycle stays Rust-owned and cancellation reaches
  through `drop`.
- **Value types cross as `#[frb(mirror)]`.** `ConnectionState`,
  `PairResponse`, `MinosError`, `ErrorKind`, `Lang`, and `PairingState`
  all get mirror shims in the adapter. The Dart side consumes them as
  sealed classes / enums with no runtime conversion on the Rust side.
- **QR payload crosses as `String`.** Dart passes the scanned JSON
  blob verbatim; Rust-side `serde_json::from_str::<QrPayload>()`
  (inside `minos-mobile::MobileClient::pair_with_json`) keeps
  `QrPayload` off the cross-language contract. This matches what the
  Swift app does on the UniFFI side and keeps `QrPayload` an internal
  detail of `minos-pairing`.
- **`events_stream` bridges via `StreamSink`.** The adapter spawns a
  `tokio` task that pumps `watch::Receiver<ConnectionState>` updates
  into the sink until the sink's channel closes; ownership lives in
  the adapter, not in `minos-mobile`.
- **Dart localization calls a free function.** `kind_message(kind,
  lang)` is exposed as a plain Rust function across the frb boundary,
  mirroring the UniFFI-side equivalent on Swift. Dart never hardcodes
  error copy.

### Generated artifact policy

- **Checked in.** Both generated trees are committed:
  - `apps/mobile/lib/src/rust/**` (Dart-side frb output, plus any
    `.freezed.dart` / `.g.dart` partials consumed by it);
  - `crates/minos-ffi-frb/src/frb_generated.rs` (Rust-side frb output
    — hand-written `src/api/` remains the source of truth).
- **Drift guard.** `cargo xtask check-all` appends a step that
  regenerates via `flutter_rust_bridge_codegen generate
  --config-file flutter_rust_bridge.yaml` and runs `git diff
  --exit-code` over the two paths above. Any silent mismatch between
  `crates/minos-ffi-frb/src/api/**` and the committed generated trees
  fails CI.
- **Regeneration path.** `cargo xtask gen-frb` is the one-liner used
  by both developers and the drift guard. It invokes the codegen from
  `apps/mobile/` (where `.fvmrc` lives) so `fvm flutter` resolves the
  pinned version; YAML paths inside `flutter_rust_bridge.yaml` are
  resolved relative to the config file, not CWD.
- **CI lane split.** The Ubuntu `dart` job runs
  format / analyze / build_runner / codegen drift without loading
  `libminos_ffi_frb.so` — the single FFI-coupled test
  (`minos_error_display_test.dart`) is tagged `ffi` via
  `test/dart_test.yaml` and skipped there. The macOS lane runs
  `cargo xtask check-all`, which `cargo build`s the host cdylib and
  then runs the full `flutter test` suite including the tagged test.

## Consequences

**Positive**

- `minos-mobile` stays an FFI-agnostic composition root. Swapping frb
  for a different adapter (e.g. a new wasm target for a web demo)
  means writing a new sibling crate, not refactoring `minos-mobile`.
- Ubuntu `dart` CI completes in minutes without a Rust toolchain or
  the iOS frb build chain — same cost profile plan 02's Swift lane
  enjoys.
- Dart error copy is driven by a single Rust-side
  `kind_message(kind, lang)` function, matching the Swift app. There
  is no third place i18n strings live.

**Neutral**

- Two FFI adapter crates is the physical consequence of frb + UniFFI
  not coexisting in one cdylib. Each exists so its respective platform
  can bind independently; the cost is acceptable because both are
  thin shims (no business logic).
- The `rust_builder/` cargokit-derived Flutter plugin (in
  `apps/mobile/rust_builder/`) is vendored and MINOS-patched to emit
  the underscored `libminos_ffi_frb.{a,dylib,so}` that cargo produces
  from the dashed crate name. Every patch site is listed in
  `apps/mobile/rust_builder/cargokit/MINOS-PATCHES.md` and re-marked
  inline with `// MINOS-PATCH:` comments so future cargokit refreshes
  can reapply them.
- Generated Dart (`apps/mobile/lib/src/rust/**`) shows up in code
  review diffs when the API surface changes. Reviewers treat it like
  the checked-in UniFFI Swift: the hand-written side
  (`crates/minos-ffi-frb/src/api/**`) is what gets read; the
  generated side is verified by `cargo xtask gen-frb` producing an
  empty diff.

## Alternatives Rejected

### Let `minos-mobile` depend on `flutter_rust_bridge` directly

Rejected. `minos-mobile` would couple its public API to whichever
FFI toolchain it imports, breaking hexagonal layering: swapping to a
different adapter would require source changes to the composition
root, not just a new adapter crate. The precedent from plan 02 is
explicit — `minos-daemon` does not depend on `uniffi`, and the
equivalent invariant here is that `minos-mobile` does not depend on
`flutter_rust_bridge`.

### Have Dart construct `QrPayload` and cross it over FFI

Rejected. `QrPayload` is an internal detail of `minos-pairing` — its
field set is subject to change as the pairing protocol evolves.
Promoting it to the frb boundary would expand the mirror surface,
force it to be kept in lockstep across three layers (Rust + frb
generated + Dart), and block non-breaking additions. Passing the QR
string verbatim and decoding Rust-side keeps `QrPayload` internal and
preserves the ability to evolve it without a Dart-side codegen roll.

### Gitignore the generated frb artifacts

Rejected. CI's Ubuntu `dart` job would need full Rust + frb setup to
even run `dart analyze`, doubling lane runtime and spreading the
toolchain surface to every first-time contributor. The
check-in-plus-drift-guard pattern is the same one plan 02 established
for UniFFI-generated Swift (commit `886e18a`); using it here keeps
the CI cost flat and turns accidental desync into a loud, mechanical
failure rather than a subtle bug.
