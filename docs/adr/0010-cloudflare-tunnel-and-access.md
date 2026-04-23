# 0010 · Cloudflare Tunnel + Access for Public Exposure

| Field | Value |
|---|---|
| Status | Accepted |
| Date | 2026-04-23 |
| Deciders | fannnzhang |

## Context

ADR 0009 commits to a broker architecture: clients connect outbound to a single relay over public WSS. The relay itself runs on a box the owner controls (MVP: the owner's Mac; future: a home Linux). Two problems arise:

1. **Exposure.** The origin box has no public IP, sits behind NAT, and should not have ports forwarded on the home router. Something must terminate TLS on a public hostname and tunnel traffic back to the origin.
2. **Authentication at the edge.** The public hostname will be reachable by anyone on the internet. Business-layer auth (device-id + device-secret, ADR 0009) will catch illegitimate clients, but the relay should not be answering arbitrary WS handshakes from the open internet in the first place.

The owner already runs a Cloudflare-managed domain (`fan-nn.top`) and has `cloudflared` installed and authenticated locally. Cloudflare Zero Trust (with Access + Service Tokens) is free for up to 50 seats.

## Decision

Expose `minos-relay` via **Cloudflare Tunnel** (`cloudflared`), gated by a **Cloudflare Access** Zero-Trust policy on the hostname.

- `cloudflared` runs on the origin box, maintains an outbound QUIC tunnel to the Cloudflare edge, and routes requests from `https://minos.fan-nn.top` into `http://127.0.0.1:8787`.
- Cloudflare Access binds a policy to `minos.fan-nn.top`: interactive browser access is gated by email OTP / SSO; non-browser clients (macOS app, iOS app) authenticate via Service Tokens (`CF-Access-Client-Id` + `CF-Access-Client-Secret` headers).
- The relay itself is unaware of Cloudflare. It listens on plain HTTP on loopback and trusts that any request that reaches it has already passed Access. A second, independent auth layer at the application level (device-id + device-secret) remains the real source of authorization for business operations.

Operational details (tunnel creation, config, LaunchDaemon install, Access application setup, Service Token minting) live in `docs/ops/cloudflare-tunnel-setup.md`.

## Consequences

**Positive**
- Zero public IP, zero port forwarding, zero self-managed TLS. The origin box is always a pure loopback listener.
- TLS termination and certificate rotation happen at Cloudflare; no Let's Encrypt renewal loop to maintain.
- Access provides a production-quality, audit-logged auth layer without writing any code. An email-allowlist policy is the work of three clicks in the Zero Trust dashboard.
- Service Tokens are a clean seam for non-browser clients: single long-lived credential per app install, revocable from the dashboard without redeploying.
- Bandwidth via Cloudflare Tunnel is not metered on Cloudflare's side. For a personal / single-user product the cost stays zero across plausible usage patterns.
- Same exposure mechanism works whether the relay moves from the Mac to a home Linux, or to a datacenter VM — only the box running `cloudflared` changes.

**Neutral / accepted cost**
- A ~10–30 ms latency increment compared to a direct tailnet hop, from the public-internet → CF edge → tunnel → origin path. Interactive CLI flows stay responsive; streaming throughput is unaffected.
- WebSocket idle timeout at the CF edge is ~100s. Clients must send application-layer ping frames every ~30s. This is standard long-connection hygiene and is handled in `minos-transport`'s heartbeat loop.
- Service Tokens must be provisioned into clients out-of-band for MVP (user pastes them during first-run setup). The onboarding UX is not automated yet. Acceptable for single-user.
- Cloudflare outage = whole product down. Historical uptime of Cloudflare's tunnel fleet is competitive with any self-hosted alternative we would realistically operate.

**Negative / explicit trade-off**
- All envelope metadata (device IDs, message sizes, timing) is visible to Cloudflare. This is accepted for the single-user MVP; a future E2EE-payload ADR will address the privacy implications for the payloads themselves, but the relay-visible metadata is a structural property of the topology.
- We accept a soft dependency on Cloudflare's product lineup. A migration off Cloudflare would require replacing both Tunnel (swap to Tailscale Funnel / ngrok / self-hosted) and Access (swap to Authelia / Keycloak / self-hosted SSO) in one pass. Given that the relay itself is unaware of either, the blast radius is confined to `cloudflared` config and client-side auth header plumbing.

## Alternatives Rejected

### Self-hosted TLS + public IP + port forwarding

Open a port on the home router, terminate TLS on the origin box with rustls / Caddy. Rejected:
- Requires a public IP (many ISPs rotate / CG-NAT) and router permissions that not every deployment will have.
- Introduces our own certificate rotation, DDOS surface, and abuse mailbox. Time sink.
- Provides no equivalent to Access's allowlist-based auth without adding Authelia / oauth2-proxy / similar.

### ngrok (or similar reverse tunnels)

Simpler than Cloudflare Tunnel for one-off demos, but:
- Hostnames are rotating on free tier; pinned hostnames require a paid plan and are still per-tunnel.
- No Access equivalent; we would re-invent the allowlist on the origin.
- Operational parity with `cloudflared` at cost; no reason to split across providers when the domain already lives in Cloudflare.

### Cloudflare Workers as the relay itself (not just the edge)

Run the broker inside Workers + Durable Objects, removing the origin box entirely. Rejected by ADR 0009 for product-shape reasons. If that decision reverses, this ADR is reconsidered as a byproduct.

### Tailscale Funnel

Tailscale's own public-ingress feature. Rejected:
- Requires the origin to be on Tailscale, which is the prerequisite we are trying to remove from end users. Even if the origin is already on a tailnet, we gain nothing by mixing Funnel with our broker model.
- No built-in equivalent to Access allowlisting; we would write the gate ourselves anyway.
