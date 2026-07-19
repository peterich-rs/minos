# 0019 · Typed Codex App-Server Protocol via Vendored Typify Codegen

| Field | Value |
|---|---|
| Status | Accepted |
| Date | 2026-04-30 |
| Deciders | fannnzhang |

## Context

`minos-agent-runtime` originally spoke codex's JSON-RPC by hand
(`serde_json::json!` macros for outbound, raw `Value` payloads for
inbound). When OpenAI started publishing the JSON Schema set for the
app-server protocol (developers.openai.com/codex/app-server), two
latent bugs surfaced:

1. The runtime's hand-maintained `APPROVAL_METHODS` list used v1 method
   names; the v2 schema namespaces approvals
   (`item/commandExecution/requestApproval`, etc.). v2 approval prompts
   silently fell through to the "warn, don't reply" branch.
2. The auto-reject payload `{"decision":"rejected"}` was non-conformant
   for all five approval response schemas — each has its own enum, and
   one (`PermissionsRequestApprovalResponse`) has no `decision` field
   at all.

The schemas we need to cover are ~200 files; hand-writing the Rust
types is too much surface to maintain.

## Decision

Introduce `crates/minos-codex-protocol`, a standalone crate that
mirrors the schemas as Rust types. Generation pipeline:

1. `cargo xtask gen-codex-protocol` runs `typify` (Oxide Computer)
   over `schemas/codex_app_server_protocol{,.v2}.schemas.json`,
   writing the result to
   `crates/minos-codex-protocol/src/generated/types.rs`.
2. The same xtask post-processes the four union schemas
   (`{Client,Server}{Request,Notification}.json`) to emit
   `impl ClientRequest` / `impl ClientNotification` per method, plus
   `ServerRequest` / `ServerNotification` `serde(tag/content)` enums.
3. Both `schemas/` and `src/generated/` are committed to git; schema
   upgrades are a two-step PR (refresh `schemas/`, regenerate, commit
   both diffs).

`minos-agent-runtime` migrates to typed types end-to-end via
`call_typed` / `notify_typed` on `CodexClient`. The approval handler is
rewritten exhaustive over `ServerRequest`, so future schema additions
become a non-exhaustive-match compile error rather than a silent
runtime hole.

## Consequences

Positive:

- Two latent bugs (v2 approval method-name drift, non-conformant
  `decision: "rejected"` payload) are fixed by construction — typed
  variants cannot be misnamed, and the per-variant typed reply is
  schema-correct.
- New schema methods become a compile-time match-arm requirement,
  eliminating the "schema added a method, codegen wasn't re-run"
  silent gap.
- Wire-format documentation lives in the schemas + generated types,
  not in scattered `json!` literals across the runtime.

Negative:

- New developer step: `cargo xtask gen-codex-protocol` after touching
  `schemas/`. A future `cargo xtask refresh-codex-schemas` (which
  shells out to `codex app-server generate-json-schema --out
  ./schemas`) is left as future work; for now schemas are refreshed
  manually.
- xtask gains dev-deps on `typify`, `prettyplease`, `syn`, `quote`,
  `schemars`, `proc-macro2`. None reach production binaries.
- `minos-agent-runtime` gains a path-dep on `minos-codex-protocol`.
  The new crate has zero minos dependencies (orthogonal to
  `minos-protocol` and `minos-ui-protocol` — they are separate wire
  formats).

## Alternatives considered

- **Hand-written types.** Rejected: ~200 schemas, drift inevitable.
- **`build.rs` codegen at build time.** Rejected: generated code not
  visible in PR diffs; CI builds become non-reproducible if a schema
  dependency is fetched lazily.
- **Per-method `client.thread_start(...)` shims.** Rejected: 71
  hand-written wrappers, each new schema method requires hand-edits.
- **Single `ClientRequest` enum API.** Rejected: poor ergonomics for
  response extraction (caller match × every method).

## Related

- Spec: `docs/superpowers/specs/codex-typed-protocol-design.md`
- Plan: `docs/superpowers/plans/10-codex-typed-protocol.md`
- Refines wire-format details from
  `docs/superpowers/specs/codex-app-server-integration-design.md`
  §10.1 (wire format) and §6.4 (approval method names).
- Prior bridge ADR: `docs/adr/0009-codex-app-server-ws-transport.md`
- Prior approval ADR: `docs/adr/0010-agent-event-raw-variant.md`
