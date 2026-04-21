# 0005 · No End-to-End Encryption in MVP

| Field | Value |
|---|---|
| Status | Accepted |
| Date | 2026-04-21 |
| Deciders | fannnzhang |

## Context

Remodex demonstrates a full E2EE design: X25519 key exchange, Ed25519 identity signing, AES-256-GCM directional keys with HKDF-SHA256, monotonic counters for replay protection. Implementing this is non-trivial — key management, rotation, recovery, pairing-time trust establishment, and UI for key state.

MVP scope (one-shot pairing + one read-only RPC) does not surface secrets that materially raise the value of E2EE within the MVP threat model.

## Decision

MVP relies on Tailscale's WireGuard layer for transport confidentiality and integrity. No application-level encryption. Trusted-device records on disk are plain JSON (not Keychain-encrypted on Mac for MVP). The pairing token is plaintext in the WebSocket upgrade header.

E2EE is scheduled for the **P2 spec** (`end-to-end-encryption.md`), which must land **before any feature handles secrets**: code containing credentials, API tokens, or content that warrants confidentiality beyond LAN trust.

## Consequences

**Positive**
- MVP delivery focuses on "the connection works" without simultaneously battling crypto UX (key recovery flows, identity rotation, untrusted-device detection).
- The pairing API (`PairingStore` trait, `seal/open` extension points reserved with passthrough defaults) is shaped to absorb E2EE in P2 without refactoring callers.

**Negative (accepted)**
- Anyone with access to either endpoint's process memory or filesystem can read pairing material and WebSocket payloads.
- A compromised tailnet member could see all Minos traffic. Mitigation relies entirely on Tailscale ACLs and tailnet membership integrity.
- Plain-JSON `devices.json` on Mac is readable by any process running as the user.

### Threat-model bracket

MVP is acceptable when:
- The user is the sole operator of both endpoints.
- The tailnet is not shared with untrusted parties.
- No agent execution is yet exposed (so no commands and no command outputs flow over the channel — only pairing acknowledgments and CLI-detection metadata).

The third condition is what actually keeps MVP defensible: when agent execution lands in P1, P2 (E2EE) must land within the same release window or be fast-followed.

## Alternatives Rejected

### Implement basic E2EE in MVP

Shipping a partial scheme (e.g., AES-GCM with shared secret derived from QR token) was considered. Rejected:
- "Half-E2EE" creates false confidence without addressing replay, identity rotation, or device revocation.
- Doubles MVP UI surface (key state, "trust this Mac" confirmation, "key changed" warnings) at a stage where the channel is not yet carrying anything more sensitive than `which codex` output.
- Better to ship no encryption with documented threat model than partial encryption with implicit promises.
