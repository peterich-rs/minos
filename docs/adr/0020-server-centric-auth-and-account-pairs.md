# 0020 · Server-Centric Auth Simplification and Account-Keyed Pairs

| Field | Value |
|---|---|
| Status | Accepted |
| Date | 2026-05-01 |
| Deciders | fannnzhang |
| Supersedes | §12.2 ("Single device vs multi-device → single") of `docs/superpowers/specs/mobile-auth-and-agent-session-design.md`; partially supersedes §5.4 (dual-rail iOS auth) of same |

## Context

Minos is account-centric (slack.ai-shaped), not P2P. The current dual-rail
auth (`X-Device-Secret` + bearer JWT for iOS, secret-only for Mac) was
inherited from the Remodex P2P design without re-justification for the
server-centric model. Three observable consequences:

1. iOS keychain holds two long-lived credentials (`device_secret` + auth
   tuple). Equivalent security guarantees can be obtained with bearer alone
   (JWT.did binds to X-Device-Id; refresh_token is per-device-revocable).
2. The pair model is keyed on device IDs: `pairings(device_a, device_b)`.
   Re-installing the iOS app changes `device_id` and orphans existing
   pairs. The product expectation is that an iOS user signed into the same
   account on a new phone immediately inherits all paired Macs.
3. `Envelope::Forward` carries no target — the backend infers it from a
   single-valued `SessionHandle.paired_with` slot. With multiple Macs paired
   to one account, the backend silently routes to "the most recent pair".

## Decision

1. **iOS becomes bearer-only.** `classify()` becomes role-aware: when a
   device row has `role = ios-client` and `secret_hash IS NULL`, the
   request authenticates via bearer alone. iOS rows are created with
   `secret_hash = NULL` and never populated. Mac rows remain as today.
2. **Pair model becomes (mac_device_id, mobile_account_id).** A new
   `account_mac_pairings` table replaces `pairings`. The mobile
   `device_id` that performed the scan is recorded in
   `paired_via_device_id` for audit only — it does not participate in
   routing.
3. **`Envelope::Forward` gets `target_device_id`.** iOS clients must name
   the Mac they are addressing. The backend validates
   `target_device_id ∈ {macs paired to caller's account_id}` before
   routing; mismatch → `PeerOffline`.
4. **`EventKind::Paired.your_device_secret` becomes `Option<DeviceSecret>`.**
   Set to `Some(secret)` for the Mac recipient (unchanged behaviour);
   `None` for iOS recipients (no secret minted).
5. **`/v1/me/peer` is replaced by `/v1/me/macs`** for iOS callers.
   Returns `Vec<MacSummary>`. Mac-side equivalent is deferred (no UI need
   today; the Mac learns peers via `EventKind::Paired` + future
   broadcast).

## Consequences

- iOS keychain stores only `(device_id, access_token, access_expires_at,
  refresh_token, account_id, account_email, peer_display_name)`.
  `device_secret` field is wiped on cold start.
- `pairings` table and `crates/minos-backend/src/store/pairings.rs` are
  deleted outright (pre-deployment).
- The `your_device_secret` field on `PairResponse` is removed; iOS clients
  receive only `(peer_device_id, peer_name)`.
- Mac WS upgrade and REST paths are unchanged. Mac still holds one
  `device_secret` per host machine.
- Anti-replay across devices: bearer's `did` claim still binds JWT to a
  specific `X-Device-Id`. Stealing only the JWT without also knowing the
  device_id (which is in TLS-protected keychain) yields nothing.
- Multi-mobile-per-account: refresh_tokens(account_id, device_id) already
  supports it; the new pair table preserves the semantic.
- Mac-side daemon's single-peer slot (`crates/minos-daemon/src/handle.rs`
  `peer: Option<PeerRecord>`) is **out of scope** for this ADR. P2 in the
  macos-relay-client-migration spec.

## Alternatives considered

- **Keep `X-Device-Secret` as a derivation of `device_id`** (e.g.
  HMAC). Rejected: the security value of secret comes from "attacker has
  device_id but not secret"; deriving secret from device_id collapses
  dual-factor to single-factor while keeping the protocol surface.
- **Make `X-Device-Secret` short-lived per session.** Rejected: doesn't
  remove the backend `secret_hash` column, doesn't simplify keychain,
  introduces a rotate endpoint with its own auth question.
- **Migrate `pairings` schema in-place by adding `account_id` column.**
  Rejected: pre-deployment context permits clean replacement; in-place
  migration would carry the device-keyed semantics indefinitely.

## Related

- Implementation plan: `docs/superpowers/plans/11-server-centric-auth-and-pair.md`
- Spec being partially superseded: `docs/superpowers/specs/mobile-auth-and-agent-session-design.md` (§12.2, §5.4)
- Prior pairing-related ADRs: `docs/adr/0014-backend-assembled-pairing-qr.md`, `docs/adr/0016-client-env-cloudflare-access.md`
