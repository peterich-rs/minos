# 0015 · Rename `minos-relay` → `minos-backend`

| Field | Value |
|---|---|
| Status | Accepted |
| Date | 2026-04-24 |
| Deciders | fannnzhang |

## Context

ADR 0009-broker-architecture-pivot.md introduced `minos-relay` as a *broker* that "speaks envelopes and forwards opaque payloads between paired clients." That description was accurate when the crate's only state was the pairings table.

In plan 05 the same crate gains four new responsibilities:

1. **Persistence.** `threads` and `raw_events` tables hold the full history of every agent run for re-translation on read (see ADR 0013).
2. **Translation.** It runs `minos-ui-protocol::translate_*` on every ingested raw event and on every `read_thread` request.
3. **Credential distribution.** It holds Cloudflare Access tokens in env vars and assembles the full pairing QR payload (see ADR 0014).
4. **Per-thread fan-out routing.** It owns the registry of paired sessions and pushes `EventKind::UiEventMessage` to every paired mobile peer of a given agent-host.

A "relay" is none of those things. The name became actively misleading: a new contributor reading the crate name would expect a stateless message-passing service, not a SQLite-backed persistence + translation layer with credential rotation responsibilities.

## Decision

Rename the crate and all of its public surface in one atomic change. Specifically:

- `crates/minos-relay/` → `crates/minos-backend/`
- Crate package name `minos-relay` → `minos-backend`. Library identifier `minos_relay` → `minos_backend`. Binary name `minos-relay` → `minos-backend`.
- Env var prefix `MINOS_RELAY_*` → `MINOS_BACKEND_*` (all of `LISTEN`, `DB`, `TOKEN_TTL_SECS`, plus the new `PUBLIC_URL`, `CF_ACCESS_CLIENT_ID`, `CF_ACCESS_CLIENT_SECRET`, `ALLOW_DEV`).
- Default DB filename `./minos-relay.db` → `./minos-backend.db`. Default macOS support directory `~/Library/Application Support/minos-relay/` → `~/Library/Application Support/minos-backend/`.
- xlog log-prefix `relay` → `backend`.
- xtask subcommands `relay-run` / `relay-db-reset` → `backend-run` / `backend-db-reset`.
- Internal error type `RelayError` (in `minos-backend/src/error.rs`) is left as-is for now — it's an internal name; renaming it is mechanical churn that doesn't pay back. A follow-up may revisit.

**Not** renamed:
- The historical spec file `docs/superpowers/specs/minos-relay-backend-design.md` — it is the document of record for plan 04 and is referenced by ADRs 0009 / 0011 / 0012. Its filename is part of its identity; mutating it would obscure history.
- ADR text in 0009 / 0010 / 0011 / 0012. Those are point-in-time decision records and should describe the world as it was at the time of the decision. Future ADRs can supersede them.
- Any prose in the plan-04 file `docs/superpowers/plans/04-minos-relay-backend.md`.

No compatibility shim, no legacy crate, no env-var alias. The branch flips the name in one commit and every dependent reference is updated in lockstep. `cargo xtask check-all` is the gate.

Spec reference: §2.1.1 (rename), §15 (ADR list).

## Consequences

**Positive:**
- The crate name now describes what the crate does: persist, translate, distribute, and fan out.
- Anyone who reads `cargo xtask backend-run` immediately understands they are starting *the backend*, not some narrow router component.
- The env-var prefix `MINOS_BACKEND_*` makes the new CF-Access env vars visually consistent with the existing `LISTEN`/`DB` ones.

**Negative:**
- One forced disruption for any operator with a running `minos-relay.service` — env vars must change, the launchd plist must change, the DB file path moves. The runbook update in `docs/ops/cloudflare-tunnel-setup.md` is the migration guide.
- Everyone with a local `./minos-relay.db` from the dev branch needs to either rename it or rerun `cargo xtask backend-db-reset`. Not a real risk because the dev DB has no production data.
- ADR 0010 (CF tunnel) and the spec file in `docs/superpowers/specs/minos-relay-backend-design.md` retain the old name in prose. A reader has to know that "relay" in those documents is the same thing that's now called "backend" in code. Acceptable trade vs the alternative (rewriting historical decision records and breaking citation graphs).

**Operational migration:**
- Build with the new branch.
- Stop `minos-relay`. Start `cargo run -p minos-backend` (or `cargo xtask backend-run`).
- Move `~/Library/Application Support/minos-relay/db.sqlite` to `~/Library/Application Support/minos-backend/db.sqlite` if the old DB needs to be preserved (testbeds typically don't).
- Update the LaunchDaemon plist (or systemd unit, or shell rc) that sets env vars: `MINOS_RELAY_*` → `MINOS_BACKEND_*`.
- Re-issue any pairing QRs (they bake the backend URL + CF tokens; old ones are still valid only if the backend URL didn't change).
