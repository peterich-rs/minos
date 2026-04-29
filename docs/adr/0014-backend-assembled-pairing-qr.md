# 0014 · Backend-Assembled Pairing QR (CF Access tokens leave the host)

| Field | Value |
|---|---|
| Status | Partially superseded by 0016 (CF Access) and 0018 (URL distribution) |
| Date | 2026-04-24 |
| Deciders | fannnzhang |

> Update 2026-04-25: ADR 0016 supersedes this ADR's Cloudflare Access
> credential-distribution decision. Backend-assembled QR payloads remain, but
> current clients get CF Access headers from build-time / host env config.
>
> Update 2026-04-29: ADR 0018 specifies the entry point and storage for the
> client-side build-time configuration (single .env.local + justfile).

## Context

Plan 04 (the relay-backend track) and ADR 0010-cloudflare-tunnel-and-access.md established that public ingress to the broker is fronted by Cloudflare Tunnel + Cloudflare Access, with the **mobile** client carrying a CF Access **service token** (`cf_access_client_id` + `cf_access_client_secret`) attached to every `/devices` WS request via `CF-Access-Client-Id` / `CF-Access-Client-Secret` headers. That part stayed.

The earlier relay spec (`docs/superpowers/specs/minos-relay-backend-design.md` §9.4) said the **mac-host** would hold the CF service token in its keychain and embed it into the QR payload at QR-generation time. Two reasons that broke once we looked at the broader picture:

1. **The host doesn't need the CF token.** The agent-host talks to the backend over a private LAN address (`ws://127.0.0.1:8787/devices` if same-machine, or a Tailscale-only address). Only the *mobile* WS goes through the CF edge. Putting the secret on the host enlarges its attack surface and introduces a per-host rotation problem with no operational benefit.
2. **The host binary is meant to be open-sourceable and self-bootstrapped.** A stock build cannot ship a CF token, and asking each operator to mint one and paste it into a Mac keychain is a real adoption cliff. The token is a backend-operator concern, not a per-host concern.

At the same time, the QR payload was already the right place to put *paired-side* secrets — `pairing_token` and `expires_at_ms` were already there, and the mobile app already treats whatever is in the QR as authoritative.

## Decision

The **backend** owns the CF Access service-token pair, reads it from environment variables on startup, and embeds it into the `PairingQrPayload` whenever an agent-host requests a QR.

Concretely:

- Backend reads `MINOS_BACKEND_CF_ACCESS_CLIENT_ID` and `MINOS_BACKEND_CF_ACCESS_CLIENT_SECRET` (plus `MINOS_BACKEND_PUBLIC_URL` and `MINOS_BACKEND_ALLOW_DEV`) at startup. Validation: when `public_url` starts with `wss://` and `allow_dev` is unset, missing CF env vars cause a startup-time error (`MinosError::CfAccessMisconfigured { reason }`).
- New `LocalRpc::RequestPairingQr` (replaces `RequestPairingToken`) returns:

  ```jsonc
  {
    "v": 2,
    "backend_url":          "wss://minos.example.com/devices",
    "host_display_name":    "Mac",
    "pairing_token":        "<random hex>",
    "expires_at_ms":        1714000000000,
    "cf_access_client_id":     "<from env>",  // optional; omitted in dev
    "cf_access_client_secret": "<from env>"   // optional; omitted in dev
  }
  ```

- The agent-host treats this payload as opaque: it renders the QR straight from the JSON it received. The host never sees the CF token in clear text *in code* (it's in the Rust `String` that gets serialised, but the host code doesn't read or persist it).
- Mobile parses the QR, persists `backend_url` + the CF token pair to the iOS Keychain (via `flutter_secure_storage`), and attaches the headers on every `/devices` WS connect.

Spec reference: §7.3 (pairing flow with QR v2), §13.3 (CF env-var setup), §9.1 (backend env validation).

## Consequences

**Positive:**
- One place to rotate the CF token: redeploy the backend with new env vars and re-issue QRs. Previously paired phones keep working until the operator forces a re-pair.
- Host binary stays self-contained and open-source-friendly. The CF dependency is a deployment concern of whoever runs the backend, not a host install-time concern.
- Mobile becomes the only edge-facing client (as it always was operationally), and that fact is now reflected in the credential distribution.

**Negative:**
- Re-pairing is now mandatory after a CF token rotation (mobile's stored copy is stale). A future feature could push a refreshed token over the existing paired channel; for MVP, a re-pair is acceptable and explicit.
- The QR payload is bigger (CF token pair adds ~150 chars). Still well within the QR spec's payload limit at error-correction level M.
- Self-hosting a backend on a remote machine still requires CF env vars to be set somewhere — typically a launchd plist for the macOS service. The plan adds a runbook entry; longer-term this should move to a documented systemd / launchd template.

**Out of scope:**
- Remote-host bootstrap (an agent-host on a different machine than the backend). The plan ships the local-loopback case; the cross-machine case (mTLS or per-host bearer to backend) is a separate spec.
- Token-refresh-without-re-pair. Possible follow-up: a `LocalRpc::RefreshCfAccess` that the mobile listens for on its existing WS and persists into Keychain.
- Migration. There is no v1 → v2 migration: `pairing_qr v1` did not exist on a shipped binary; this is the first QR format the public sees.
