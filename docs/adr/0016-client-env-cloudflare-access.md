# 0016 · Client-Configured Cloudflare Access Headers

| Field | Value |
|---|---|
| Status | Refined by 0018 (entry-point and storage) |
| Date | 2026-04-25 |
| Deciders | fannnzhang |

## Context

ADR 0014 moved Cloudflare Access Service Token storage out of the agent-host
Keychain and into backend env vars so the backend could embed
`CF-Access-Client-Id` / `CF-Access-Client-Secret` in pairing QR payloads.

That still leaves two product problems:

- Agent hosts are no longer macOS-only; Linux/macOS hosts may both connect to
  the public Cloudflare-protected backend and therefore need Access headers.
- Cloudflare Access service tokens are edge-gate bearer credentials, not Minos
  business authorization. Minos already needs `pairing_token` and per-device
  `device_secret` to decide what a device may do.

## Decision

Cloudflare Access service tokens are configured on clients, not stored in
Minos Keychain and not required by the backend.

- Mobile receives the pair at build time via Flutter `--dart-define`
  (`CF_ACCESS_CLIENT_ID`, `CF_ACCESS_CLIENT_SECRET`). CI should source those
  values from GitHub Secrets; local runs can forward shell env vars.
- Agent hosts read the same variable names from their process environment,
  whether launched by CLI, launchd, systemd, or the macOS app.
- The macOS app no longer offers UI for manually entering Access tokens and
  never persists them to Keychain.
- Backend `MINOS_BACKEND_CF_ACCESS_CLIENT_ID` / `...SECRET` remain optional
  compatibility fields. If set, the backend may still include them in QR
  payloads for older clients, but current clients override/ignore QR-carried
  Access fields.
- The same Access header pair must be applied to every Cloudflare-protected
  request path, including REST endpoints such as `/health` and WebSocket
  endpoints such as `/devices`.
- Minos business authorization remains `pairing_token` for first pairing and
  per-device `device_secret` for ongoing access.

## Consequences

Positive:

- No user-facing Access-token setup screen on agent hosts.
- One consistent operational model for macOS, Linux, and mobile clients.
- Cloudflare Access stays a defense-in-depth public ingress filter while Minos
  retains real device authorization and revocation.

Negative:

- A client-embedded mobile token is a shared bearer credential. If extracted,
  it can pass the Cloudflare edge until rotated.
- Rotation requires updating client build/env configuration and allowing an
  overlap window before revoking the old Cloudflare token.

Supersedes the credential-distribution part of ADR 0014. ADR 0014's QR pairing
and backend-assembled payload shape otherwise remain in force.
