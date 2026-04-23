# 0009 · Broker Architecture Pivot (Relay over Tailscale P2P)

| Field | Value |
|---|---|
| Status | Accepted |
| Date | 2026-04-23 |
| Deciders | fannnzhang |

## Context

The original MVP (ADRs 0001–0006, spec `minos-architecture-and-mvp-design.md`) assumed a peer-to-peer topology: the macOS daemon binds a JSON-RPC WebSocket server on its Tailscale `100.x.y.z` address; the iPhone dials it directly over the tailnet. The shape was cheap to scaffold but collided with three realities during implementation review:

1. **User-facing prerequisite wall.** Tailscale requires install + sign-in + tailnet selection + MagicDNS comprehension before the Minos flow begins. The intended product (slock.ai-style remote AI-coding control) cannot assume that level of pre-work from the end user.
2. **Future direction.** The roadmap points at a backend-mediated hub: multiple Mac hosts under one account, a browser admin console, eventual cloud coordination. A P2P topology forecloses those without a second, harder pivot later.
3. **Infrastructure already in hand.** The owner runs a Cloudflare-managed domain (`fan-nn.top`), has `cloudflared` authenticated locally, and is comfortable hosting a small Rust service on the same box. A single named tunnel (`minos.fan-nn.top`) in front of a relay is simpler to explain, deploy, and extend than the tailnet prerequisite.

The choice is between fixing the Tailscale experience or committing to a broker model now, while the code surface is small enough that the rewrite cost is hours, not weeks.

## Decision

Minos pivots from peer-to-peer-over-Tailscale to **broker-over-public-WSS**. A new standalone Rust service, `minos-relay`, is the only endpoint clients connect to. The Mac daemon and iOS client both become outbound WebSocket clients of the relay.

Topology:

```
Mac app  ──┐
           ├──► wss://minos.fan-nn.top (cloudflared) ──► minos-relay
iOS app  ──┘                                                │
                                                        SQLite persistence
```

Exposure is via Cloudflare Tunnel (see ADR 0010). Authentication is two-layered: Cloudflare Access Service Token at the edge and application-level `DeviceId` + `DeviceSecret` at the relay. Peer-to-peer RPC schemas (`minos-protocol`'s `#[rpc]` trait) are retained verbatim; the relay forwards those payloads opaquely (envelope protocol, ADR 0011).

`minos-daemon` keeps its name but changes role from WS server to WS client. `minos-transport`'s server surface is retired; the client surface grows auth-header arguments. A new crate `minos-relay` becomes the backend.

## Consequences

**Positive**
- Zero end-user prerequisite beyond "install the app" — no tailnet sign-in, no device authorization, no MagicDNS.
- Identical URL (`wss://minos.fan-nn.top`) works from LAN, cellular, and foreign Wi-Fi — NAT traversal and firewall concerns all move to the cloudflared + CF edge path.
- Opens the door to browser admin console, multi-Mac registration, and eventual cloud-side agent coordination without another topology pivot.
- The relay is a single-file Rust process with SQLite-backed state; simple enough to reason about, small enough to rewrite if wrong.
- JSON-RPC peer-to-peer schemas are untouched. Client-to-client contracts evolve at the same pace as before; only the physical path changes.

**Neutral / accepted cost**
- One extra process to operate (the relay binary + cloudflared). On the MVP Mac-only deployment this is two launchd services.
- Every message adds one Cloudflare-edge hop (~10–30ms) vs direct Tailscale LAN. Interactive CLI flows remain responsive; streaming throughput is unaffected.
- Relay downtime = whole product down. Mitigated by running the relay on the same always-on box (Mac with `caffeinate` or eventual Linux) and monitoring via `/health`.
- `minos-daemon`'s former server-bound logic is deleted, not archived. If P2P ever comes back as a peer option, it is a fresh implementation against the current client surfaces.

**Negative / explicit trade-off**
- A single relay is a single trust point. The owner controls it, which is fine for MVP, but a multi-user product would need either per-user relay instances or a more elaborate tenancy story. Out of MVP scope.
- End-to-end encryption is not free anymore — the transport protects bytes up to the relay, but the relay sees envelope metadata. E2EE of `forward` payloads is deferred to a follow-up ADR.

## Alternatives Rejected

### Keep P2P, smooth the Tailscale UX

Build an in-app onboarding that installs / signs into Tailscale for the user, handles tailnet selection, and verifies connectivity before showing the QR. Rejected:
- The hardest parts (NetworkExtension entitlement on iOS, MDM profiles for Tailscale on macOS) are not things an app can just wallpaper over. The user still ends up in Apple's permission dialogs.
- Even if the UX were perfect, the topology still does not support browser clients or cross-Mac aggregation without a separate parallel stack.
- The fix is UX paint on an architecture that no longer matches the product direction.

### Relay on Cloudflare Workers + Durable Objects

Run the broker on Cloudflare's edge runtime using a Durable Object per user session. Rejected for this MVP:
- Owner already runs a Rust stack comfortably; adopting Workers means a second language (TypeScript), a second test loop, and a vendor lock-in decision for what is still an architectural experiment.
- SQLite + `sqlx` + a single Rust bin is a more direct experiment platform. Workers is a migration target for P3+ once the protocol and product shape are stable.
- Bandwidth and per-request costs on Workers are non-zero; a self-hosted relay through cloudflared transits Cloudflare's network for free.

### Per-user SaaS backend (Supabase / Firebase / PlanetScale)

Adopt a managed backend that gives auth + persistence + realtime channels. Rejected:
- Realtime channels on those products are fine for pub/sub but awkward for bidirectional RPC routing between two specific sessions.
- The envelope routing logic is load-bearing; outsourcing it to a generic realtime layer means working around its constraints rather than with them.
- Vendor lock-in cost for what we can host in ~1000 lines of Rust is not justified.
