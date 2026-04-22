# Cargokit · Minos local patches

This vendored tree is `flutter_rust_bridge_codegen integrate`'s cargokit scaffolding.
It is patched locally so that the dashed crate name `minos-ffi-frb` maps to the
underscored on-disk artifact `libminos_ffi_frb.a` that cargo emits by default.

**On any future refresh of cargokit (e.g. `frb integrate` re-run, manual rebase
against upstream `irondash/cargokit:main`), re-apply every patch listed below.**
Each patch site is additionally marked inline with a `MINOS-PATCH:` comment so
grep can find them.

## Patches

1. **`build_tool/lib/src/cargo.dart`** · `CrateInfo` gains a `libName` getter
   (`replaceAll('-', '_')`). This is the single authoritative mapping; every
   call site that previously used `crateInfo.packageName` for artifact
   filename construction now uses `crateInfo.libName`. `packageName` is still
   used for human-facing text (log messages, GitHub release names) and for
   `cargo build -p <packageName>` (which accepts only the raw dashed form).

2. **`build_tool/lib/src/build_pod.dart`** — line where `libName` is read and
   used for `lib$libName.a` / `$libName.framework/...` path construction.

3. **`build_tool/lib/src/artifacts_provider.dart`** — two `getArtifactNames`
   calls in the local-build branch (dylib + staticlib lookups) swapped from
   `packageName` to `libName`. A third call in the precompiled-binaries
   fetch branch (around line 125) is ALSO swapped for consistency — that
   branch is not exercised today (we don't enable remote precompiles) but
   would regress the same way if later enabled.

4. **`build_tool/lib/src/verify_binaries.dart`** — the `getArtifactNames` call
   inside the verify loop swapped from `packageName` to `libName`. Same latent
   rationale as artifacts_provider: precompiled-binaries code path, not
   active today, but kept consistent.

5. **`build_tool/lib/src/precompile_binaries.dart`** — the `getArtifactNames`
   call in the publish loop swapped. Note that the `_getOrCreateRelease`
   call on the same file still uses `packageName` as its `packageName:`
   argument — that's the GitHub release name, user-facing, intentionally
   dashed. Do NOT change that site.

## Finding every patched site

```
grep -rn "MINOS-PATCH" apps/mobile/rust_builder/cargokit/
```

Should return exactly one hit per patched site (6 as of Phase C of plan 03).
