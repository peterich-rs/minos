# Minos

Native macOS status-bar app + Flutter mobile client + shared Rust core for remote AI-coding control. Drive `codex` / `claude` / `gemini` on a Mac from a paired phone over Tailscale.

## Status

MVP under construction. See `docs/superpowers/specs/minos-architecture-and-mvp-design.md` for the design and `docs/superpowers/plans/` for implementation plans.

## Quick start (development)

```bash
# Bootstrap dev tools (uniffi-bindgen, frb codegen, cargo-deny, etc.)
cargo xtask bootstrap

# Run all checks (fmt + clippy + tests + lints)
cargo xtask check-all
```

## Repository layout

```
crates/    Rust workspace (9 crates: domain, protocol, pairing, cli-detect,
           transport, daemon, mobile, ffi-uniffi, ffi-frb)
apps/      macOS (Swift/UniFFI) and mobile (Flutter/frb) — populated in plans 02/03
xtask/     Build / codegen orchestration in Rust
docs/      Specs (`docs/superpowers/specs/`) and ADRs (`docs/adr/`)
```

## License

MIT — see `LICENSE`.
