# 0003 · Rust Clean Arch Deviation: Crate-Bordered Hexagonal

| Field | Value |
|---|---|
| Status | Accepted |
| Date | 2026-04-21 |
| Deciders | fannnzhang |

## Context

Project-wide preference: Clean Architecture. Swift and Dart sides naturally express this as four-folder layering (`Presentation / Application / Domain / Infrastructure`). Rust does not have OOP class hierarchies, and forcing the four-folder pattern inside a single Rust crate produces friction (oversized crates with heterogeneous responsibilities, slower clippy / test loops, awkward visibility rules).

## Decision

Rust uses **crate-bordered hexagonal architecture**: each crate is a layer or capability, with ports defined as traits in shared crates and adapters as separate impl crates. No four-folder structure inside any crate.

| Crate | Hexagonal role |
|---|---|
| `minos-domain` | Entities (pure types, no I/O, no async) |
| `minos-protocol` | Adapters / contract (jsonrpsee service trait) |
| `minos-pairing` | Use cases (state machine + `PairingStore` trait) |
| `minos-cli-detect` | Use cases (cli detection logic) |
| `minos-transport` | Adapters (WS server / client) |
| `minos-daemon` | Composition root (Mac side) |
| `minos-mobile` | Composition root (mobile side) |
| `minos-ffi-uniffi` / `minos-ffi-frb` | Adapters (FFI shims) |

Ports (traits) are defined in the crate where the use case lives (`PairingStore` in `minos-pairing`, `CommandRunner` in `minos-cli-detect`). Concrete adapter implementations live with their owners (file-backed `PairingStore` in `minos-daemon`; Keychain-backed `PairingStore` in `minos-mobile`).

## Consequences

**Positive**
- `cargo clippy` and `cargo test` scope per crate — fast iteration loops on each layer.
- Crate boundaries enforce visibility: nothing can reach into another crate's internals without explicit `pub` exposure.
- Matches Rust community conventions (Axum examples, Bitwarden, several Tokio-based projects).
- New agent backends in P1 (`minos-agent-codex`, `minos-agent-pty`) slot in as new crates without restructure.

**Neutral**
- Nine crates is more granular than a typical Clean Arch four-layer project. Total LOC is comparable; the partitioning is along capability axes rather than abstraction axes.
- Contributors fluent in OOP Clean Arch may need a brief orientation: "the crate **is** the layer."

## Alternatives Rejected

### Strict four-layer folders inside one large `minos-core` crate

```
minos-core/
└── src/
    ├── entities/
    ├── use_cases/
    ├── interface_adapters/
    └── frameworks_drivers/
```

Rejected:
- Single oversized crate makes incremental compilation slower; clippy and test cycles become measured in tens of seconds rather than ones.
- Visibility leaks: nothing prevents a `frameworks_drivers/` module from importing a private symbol from `entities/`. Layer separation becomes convention only, not enforced.
- Forces every consumer to depend on the entire monolith even if they only need entity types.

### Two-crate split (`minos-core` + `minos-platform`)

A common compromise — one crate for "pure" code, one for I/O code. Rejected as too coarse for the actual capability boundaries here: pairing, transport, cli-detect, daemon composition, and mobile composition each have distinct testing needs, distinct dependency footprints, and distinct evolution paths.
