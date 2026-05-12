# Slock.ai Feature Completion — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan phase-by-phase. Steps use checkbox (`- [ ]`) syntax for tracking. Every commit MUST be preceded by `cargo xtask check-all` (backend/rust) and `pnpm --filter web run build` + `flutter analyze` for the app touched.

**Goal:** Close the gaps identified in [`docs/superpowers/specs/slock-ai-feature-completion.md`](../specs/slock-ai-feature-completion.md) so that Minos reaches end-to-end parity with slock.ai on the in-scope matrix (R1 – R10). The spec is the contract; this plan is the sequenced, file-level execution of that contract.

**Architecture:** No protocol / wire-version change. All work stays on `envelope.v = 1` and the existing `UiEventMessage` set (spec R9 explicitly forbids forks). The plan is weighted toward the Web admin (the spec's largest single gap) and toward raising parity where mobile already leads (interrupt, request-trace, log panel, host skills). Backend, host, and the new claude / gemini PTY adapter land in later phases because they depend on choices that can — and should — be reviewed after the client UX clears.

**Tech stack:** Rust 2021 (axum, sqlx, jsonrpsee, tokio, tracing), flutter_rust_bridge v2 + Flutter 3, React 19 + Vite + TS for web, Swift 5 / SwiftUI + XcodeGen for macOS.

**Spec:** `docs/superpowers/specs/slock-ai-feature-completion.md` (authoritative). Every task references its `R*` acceptance criterion so verification is mechanical.

**Pre-existing state (source of truth: code, not prior docs):**

- Backend already exposes `/v1/auth/*`, `/v1/pairing/*`, `/v1/me/{hosts,peers,peer,profile}`, `/v1/threads/*`, `/v1/users/search`, `/v1/friends`, `/v1/friend-requests`, `/v1/conversations/*`, and `/devices` WS with `Forward` / `Forwarded` / `Event` / `Ingest` envelopes. 17 migrations are applied.
- macOS host (`apps/macos/Minos`, `crates/minos-daemon`, `crates/minos-agent-runtime`) is the most complete: multi-workspace codex `AgentManager`, reconciliator, host skills RPCs, cli-detect, `/v1/me/peers` rendering.
- Mobile (`apps/mobile`, `crates/minos-mobile`, `crates/minos-ffi-frb`) covers register/login, QR pair, thread list/detail, streaming UI events, `start_agent` / `send_user_message` / `interrupt_thread` / `forget_host`, social IM, `LogViewerPage`, `RequestTracePanel`.
- Web (`apps/web`) is a single-file `App.tsx` console + `components/{agents,social}-workspace.tsx`; missing interrupt button, host-skills UI, request trace, ring-buffer logs, WS lifecycle / reconnect, offline banner, and end-reason display.

**Explicitly out of scope (mirrors spec §8):** E2EE, Android release, OAuth/SSO/2FA/email verification/password reset, APNs/FCM, Sparkle/TestFlight, HA backend, workspace-level Git ops, multi-tenant / team, cloud-sync agent profiles, full tool-approval UI, mobile local SQLite persistence.

---

## Phase Map (at a glance)

| Phase | Scope | Primary spec coverage | Depends on |
|---|---|---|---|
| P1 | Web admin UX parity | R2.8, R3.6, R4.6, R4.8, R10.2, R10.3, R10.5, R8.4 | — |
| P2 | Web: host skills + observability | R5 (web), R7.4, R10.1 | P1 |
| P3 | Workspace MRU + agent-profile boundary | R4.9, §5.4 / §5.5 | — |
| P4 | Host MenuBar: account + minos_id surface | R1.8, §5.1, §5.6 | — |
| P5 | Backend logging + secret redaction audit | R6.*, R7.1, R7.5, R7.6 | — |
| P6 | Claude / Gemini minimal PTY adapter | R3.4, §9 risk #1 | — |
| P7 | Protocol round-trip & contract tests | R9.* | touches all wire crates |

Phases are intentionally independent where possible so each lands behind its own review.

---

## File Structure (consolidated)

### New files

| Path | Responsibility | Phase |
|---|---|---|
| `apps/web/src/lib/relay-socket.ts` | extracted `RelaySocket` with reconnect, visibility hooks, pending-timer cleanup (currently embedded in `lib/minos.ts`) | P1 |
| `apps/web/src/lib/observability.ts` | ring-buffer for HTTP + RPC traces and log capture (≥ 500 entries) | P2 |
| `apps/web/src/components/host-skills-panel.tsx` | host-skills list / toggle UI for the active host | P2 |
| `apps/web/src/components/observability-panel.tsx` | request-trace + logs panel (tabs) | P2 |
| `apps/web/src/components/workspace-mru.tsx` | reusable MRU chips bound to `lib/workspace-mru.ts` | P3 |
| `apps/web/src/lib/workspace-mru.ts` | per-host workspace MRU (localStorage, cap 8) | P3 |
| `apps/mobile/lib/presentation/widgets/workspace_mru_chips.dart` | matching MRU chips for mobile | P3 |
| `apps/mobile/lib/infrastructure/workspace_mru_store.dart` | per-host workspace MRU (shared_preferences, cap 8) | P3 |
| `apps/macos/Minos/Presentation/AccountPeerRowView.swift` | row view showing `account_email` + `minos_id` + `mobile_device_name` | P4 |
| `crates/minos-backend/src/logging.rs` | tracing file-sink (xlog-style rotating file under `$MINOS_HOME/logs`) | P5 |
| `crates/minos-backend/tests/secret_redaction.rs` | blanket assertion that `password` / `refresh_token` / `device_secret` never appear in `tracing` output | P5 |
| `crates/minos-agent-runtime/src/pty_agent.rs` | `PtyAgent` wrapping spawn + stdin/stdout + `UiEventMessage::Raw` fan-out | P6 |
| `crates/minos-agent-runtime/tests/pty_agent_smoke.rs` | spawn a fake CLI (`bash -c 'printf ...'`), verify raw events reach ingest | P6 |
| `crates/minos-protocol/tests/envelope_roundtrip.rs` | extended round-trip coverage (ADR 0011 golden + new cases) | P7 |
| `crates/minos-ui-protocol/tests/roundtrip.rs` | per-variant round-trip, including `Raw { raw_kind, payload_json }` | P7 |

### Modified files (by phase)

**P1 — Web UX parity**
- `apps/web/src/App.tsx` — add interrupt button, offline banner, visibility-driven lifecycle, auto-drop missing `activeHost`, end-reason badge on thread list.
- `apps/web/src/lib/minos.ts` — split out `RelaySocket` (move to `relay-socket.ts`); add linear-backoff reconnect (1s → 2s → 5s, max 3); keep HTTP helpers.
- `apps/web/src/components/agents-workspace.tsx` — consume updated `RelaySocket` API (no logic churn expected).
- `apps/web/README.md` — document new UX states (interrupt, offline, visibility).

**P2 — Web observability + host skills**
- `apps/web/src/App.tsx` — mount `HostSkillsPanel` under sidebar and `ObservabilityPanel` behind a new "Diagnostics" affordance.
- `apps/web/src/lib/minos.ts` — every `fetch` and `sendRpc` routed through `observability.ts` trace hooks.
- `apps/web/src/lib/relay-socket.ts` — publish per-RPC `method` / `latency_ms` / `status` to the trace ring buffer.

**P3 — Workspace MRU + profile boundary**
- `apps/web/src/App.tsx` — workspace field uses MRU chips; persist on thread start; prefill from `activeHost`.
- `apps/web/src/components/agents-workspace.tsx` — profiles stay purely local (enforce no backend write).
- `apps/web/src/lib/agent-profiles.ts` — add unit-test-style guard assertion in dev build (`throwIfSynced`) to document "client-only".
- `apps/mobile/lib/presentation/pages/agents_hub_page.dart` — attach `WorkspaceMruChips` in the new-thread launcher.
- `apps/mobile/lib/application/minos_providers.dart` — expose `workspaceMruProvider`.

**P4 — macOS MenuBar account surface**
- `apps/macos/Minos/Presentation/MenuBarView.swift` — render bound account list (`account_email`, `mobile_device_name`) using `AccountPeerRowView`.
- `apps/macos/Minos/Presentation/AgentSegmentView.swift` — footer strip with `minos_id` read-only if ≥ 1 account bound.
- `apps/macos/Minos/Application/MenuBarViewModel.swift` (if present — otherwise `MenuBarView.swift` inline) — fetch `/v1/me/peers` and `/v1/me/profile` through existing daemon; cache.
- `crates/minos-daemon/src/rpc/me.rs` (new helper) — daemon-side proxy for `/v1/me/peers` & `/v1/me/profile` if not already exposed; otherwise leave to Swift `URLSession` client with device-secret header.

**P5 — Backend logging + redaction**
- `crates/minos-backend/src/main.rs` — initialize new `logging::init()` at startup; honor `$MINOS_HOME`.
- `crates/minos-backend/src/http/v1/auth.rs` — confirm `password`, `refresh_token` never logged in error paths; add `#[tracing::instrument(skip(body))]` where missing.
- `crates/minos-backend/src/http/v1/pairing.rs` — ensure `Event::Paired.your_device_secret` never surfaces in logs.
- `crates/minos-backend/src/http/mod.rs` — extend middleware to log `method`, `path`, `status`, `latency_ms`, `account_id` (when authed).
- `crates/minos-backend/src/http/health.rs` — return HTTP 503 if DB ping fails (R7.6).

**P6 — Claude / Gemini PTY**
- `crates/minos-agent-runtime/src/manager.rs` — dispatch by `AgentName`: existing `codex_client` path for `codex`, new `pty_agent` path for `claude` | `gemini`.
- `crates/minos-agent-runtime/src/lib.rs` — re-export `PtyAgent`.
- `crates/minos-cli-detect/src/lib.rs` — make sure `claude` and `gemini` detection returns `CliStatus::Ok { path, version }` or `CliStatus::Missing`.
- `crates/minos-backend/src/ingest/mod.rs` — fallthrough: unknown `raw_kind` → `UiEventMessage::Raw` (already the documented behavior; tighten tests).

**P7 — Round-trip tests**
- `crates/minos-protocol/src/envelope.rs` — no API change, add missing `Deserialize`/`Serialize` derives if any; verify `EventKind` round-trip.
- `crates/minos-ui-protocol/src/lib.rs` — same spirit.
- `crates/minos-protocol/tests/envelope_golden.rs` — extend with fixtures exercising `Forward` with nested params, `IngestCheckpoint`, `ServerShutdown`.

---

## Phase 1 — Web admin UX parity

**Goal:** The web console reaches functional parity with mobile on `console` tab: interrupt current turn, survive network hiccups, survive `activeHost` disappearing, survive tab backgrounding, show thread end reason. Covers spec R2.8, R3.6, R4.6, R4.8, R8.4, R10.2, R10.3, R10.5.

**Non-goals:** Host skills UI (P2), request-trace UI (P2), MRU (P3), any backend change.

### Task P1.1 — Extract `RelaySocket` into its own module

**Files:**
- Create: `apps/web/src/lib/relay-socket.ts`
- Modify: `apps/web/src/lib/minos.ts` (re-export, keep backwards-compatible import path)
- Modify: `apps/web/src/App.tsx` (import site may shift but public surface stays)

- [ ] **Step 1:** Move `RelaySocket`, `RelayConnectionState`, `PendingRequest`, `EnvelopeEvent`, `UiEventFrame`, `SocialMessageFrame` to `relay-socket.ts`.
- [ ] **Step 2:** Re-export them from `minos.ts` to avoid churning every import.
- [ ] **Step 3:** Ensure `pending` timers are cleared on `close()` and on socket `onclose` to prevent stale timeouts (pre-existing correctness fix).
- [ ] **Verify:** `pnpm --filter web run build` still green; smoke-run `pnpm --filter web run dev` and confirm login + thread load.

### Task P1.2 — Interrupt button on selected thread (R4.6)

**Files:** `apps/web/src/App.tsx`.

- [ ] **Step 1:** Add handler `handleInterruptThread(threadId)` that calls `socket.sendRpc<void>(activeHost, 'minos_interrupt_thread', { thread_id: threadId })`.
- [ ] **Step 2:** Wire new `Button` in the workspace surface header, next to `Close thread`. Button disabled when:
  - `!selectedThread`
  - selected thread has `end_reason != null` (already closed)
  - there is no assistant message in `MessageStarted` without `MessageCompleted` in the current transcript.
- [ ] **Step 3:** After a successful interrupt, refresh `threadRecords[threadId]` from `/v1/threads/read` (warm-path already exists).
- [ ] **Verify:** Manual test — start a thread, fire a long prompt, click Interrupt; `thread_closed` event arrives with the expected reason. Map to R3.6 / R4.6.

### Task P1.3 — End-reason badge on thread list (R4.8)

**Files:** `apps/web/src/App.tsx`.

- [ ] **Step 1:** Replace the `threadStateCopy` badge row with a second `Badge` that renders `thread.end_reason.kind` as a human string when non-null.
- [ ] **Step 2:** Add visual variant: `crashed → danger`, `user_stopped|agent_done → outline`, `timeout|host_disconnected → accent`.
- [ ] **Verify:** Seed a closed thread via backend fixture and confirm badge renders.

### Task P1.4 — Auto-drop missing `activeHost` (R2.8)

**Files:** `apps/web/src/App.tsx`, `apps/web/src/lib/minos.ts`.

- [ ] **Step 1:** In `commitSnapshot`, when `activeHost` no longer appears in `snapshot.hosts`, either pick `snapshot.hosts[0]?.host_device_id ?? null` or clear.
- [ ] **Step 2:** When clearing to `null`, surface an info banner "No hosts available. Pair a host to continue." using the existing `status-banner` CSS.
- [ ] **Step 3:** Ensure `saveActiveHost(null)` is called so `localStorage` stays consistent.
- [ ] **Verify:** Unit-style reasoning — trigger `forget_host` from mobile; on next `fetchConsoleSnapshot`, web drops the host.

### Task P1.5 — Pairing UX hardening (R2.3)

**Files:** `apps/web/src/App.tsx`.

- [ ] **Step 1:** Accept both a full `PairingQrPayload` JSON **and** a bare `pairing_token` string in the pair dialog. Detect: if the trimmed input does not start with `{`, treat the whole value as the token, use `'Browser admin'` as device name, omit display-name match.
- [ ] **Step 2:** On success set `activeHost` to the just-returned host (fall back to first host if display name match fails).
- [ ] **Verify:** paste a QR JSON from host → success; paste a raw token copied from the daemon log → success.

### Task P1.6 — Linear-backoff WS reconnect (R10.3)

**Files:** `apps/web/src/lib/relay-socket.ts`, `apps/web/src/App.tsx`.

- [ ] **Step 1:** Add an optional `autoReconnect: { ticketProvider: () => Promise<string>, attempts?: [1000,2000,5000] }` to `RelaySocket`.
- [ ] **Step 2:** On `onclose` (unless `close()` was called explicitly) schedule `setTimeout` with the next backoff; call `ticketProvider()` then `connect(ticket)`; advance index on each failure.
- [ ] **Step 3:** After exhausting attempts, emit `onConnectionState('error', 'relay offline')` and stop; surface a manual "Retry" button from `App.tsx`.
- [ ] **Step 4:** Reset the backoff index whenever `onopen` fires.
- [ ] **Verify:** `cargo run -p minos-backend -- --port 8787`, drop & restore packets with `npx ws-cat` or kill/restart backend; web reconnects within the backoff window.

### Task P1.7 — Visibility-driven lifecycle (R10.2)

**Files:** `apps/web/src/App.tsx`.

- [ ] **Step 1:** Add `useEffect` listening on `document.visibilitychange`.
- [ ] **Step 2:** On transition to `hidden`, start a 30 s timer; on timer fire, call `socketRef.current?.close()`.
- [ ] **Step 3:** On transition to `visible`, if `socket == null || state === 'closed'`, call the same ticket-bootstrap path used on login and re-`connect`.
- [ ] **Step 4:** Cancel the hidden-timer on any `visible` transition.
- [ ] **Verify:** Manually background the tab > 30 s, return, confirm the relay badge is `connected` within ~1 s.

### Task P1.8 — Offline transcript indicator (R10.5)

**Files:** `apps/web/src/App.tsx`.

- [ ] **Step 1:** When `connectionState === 'closed' | 'error'` and a thread is selected with cached `threadRecords[id]`, show a muted banner "Relay offline — showing cached history".
- [ ] **Step 2:** Disable the composer send button under the same condition (already partly true via `!activeHost` check — unify).
- [ ] **Verify:** Kill backend; selected thread still visible with banner; composer disabled.

### Task P1.9 — Logout tightness (R1.5)

**Files:** `apps/web/src/App.tsx`.

- [ ] **Step 1:** Confirm `handleLogout` closes WS within 500 ms (`socketRef.current?.close()` is already synchronous; add an explicit `setRelaySocket(null)` before awaiting the HTTP logout).
- [ ] **Step 2:** Verify `localStorage` keys listed in spec R8.4 are cleared: `minos.web.session`, `minos.web.active-host`, `minos.web.workspace`. Keep `minos.web.device-id`.
- [ ] **Verify:** DevTools → Application → LocalStorage after logout matches spec R8.4.

**Phase 1 exit:** `pnpm --filter web run lint && pnpm --filter web run build` clean; manual smoke of §4.4 user stories runs end-to-end.

---

## Phase 2 — Web observability + host skills

**Goal:** Ship the two largest remaining web-only gaps — a host-skills panel (R5) and an observability surface matching mobile's `LogViewerPage` + `RequestTracePanel` (R7.4, R10.1).

### Task P2.1 — Ring-buffer observability primitive

**Files:** create `apps/web/src/lib/observability.ts`; modify `apps/web/src/lib/minos.ts`, `apps/web/src/lib/relay-socket.ts`.

- [ ] **Step 1:** Export `createTraceBuffer({ cap = 500 })` with `push`, `subscribe`, `snapshot`. Entries carry `{ ts_ms, kind: 'http' | 'rpc', method, target, status, latency_ms, summary }`.
- [ ] **Step 2:** Add `createLogBuffer({ cap = 500 })` with the same pattern; `push(level, message)` signature.
- [ ] **Step 3:** In `requestJson`, wrap around the `fetch` to push `{ kind: 'http', method: init.method, target: path, status, latency_ms, summary: firstLineOfError?: }`. Do **not** include request / response bodies — spec R7.5 bans it.
- [ ] **Step 4:** In `RelaySocket.sendRpc`, wrap the promise to push `{ kind: 'rpc', method, target: targetDeviceId, status: 'ok'|'error'|'timeout', latency_ms, summary: errorMessage?: }`.
- [ ] **Step 5:** Pipe `console.warn` / `console.error` from `App.tsx`'s `setSystemMessage` callsites into `logBuffer.push`.
- [ ] **Verify:** unit reasoning — after 600 HTTP hits, `snapshot().length === 500` and oldest entries are gone.

### Task P2.2 — Observability panel component

**Files:** create `apps/web/src/components/observability-panel.tsx`; modify `apps/web/src/App.tsx`.

- [ ] **Step 1:** Tabs: `Requests`, `Logs`. Virtualize only if list grows past ~200 visible rows (skip for v1; `overflow-y: auto` is fine up to the 500 cap).
- [ ] **Step 2:** Requests tab: table with columns method | target | status | latency (ms) | time. Color-code `status`: ok = default, error/timeout = danger.
- [ ] **Step 3:** Logs tab: `<pre>` stream with timestamp + level + message, newest first, auto-scroll with user-override (scroll position detection).
- [ ] **Step 4:** Add "Copy JSON" button that exports the current snapshot to clipboard.
- [ ] **Step 5:** Mount as a collapsible drawer from the workspace header in `App.tsx`.

### Task P2.3 — Host-skills panel (R5.1–R5.4)

**Files:** create `apps/web/src/components/host-skills-panel.tsx`; modify `apps/web/src/App.tsx`.

- [ ] **Step 1:** New `HostSkillsPanel` takes `relaySocket`, `activeHost`, `workspace`.
- [ ] **Step 2:** On mount + when `activeHost`/`workspace` changes, call `sendRpc<ListHostSkillsResponse>(activeHost, 'minos_list_host_skills', { workspace: workspace || null })`.
- [ ] **Step 3:** Render by entry (`cwd`), group skills by `scope` (`global` / `workspace`), show `display_name` fallback `name`, `short_description` fallback `description`, a `Switch` for `enabled`.
- [ ] **Step 4:** On toggle, call `sendRpc(activeHost, 'minos_write_host_skill_config', { workspace: entry.cwd, path: skill.path, enabled: next })`, then refetch within 300 ms (R5.2).
- [ ] **Step 5:** Render `HostSkillError[]` with copy-to-clipboard per entry (R5.4).
- [ ] **Step 6:** Mount the panel below the "Runtime" surface in the sidebar.

### Task P2.4 — Mobile parity check (skills entry)

**Files:** `apps/mobile/lib/presentation/pages/agents_hub_page.dart`.

- [ ] **Step 1:** Confirm the existing skills UI is reachable in ≤ 2 taps from the thread detail; if it sits behind a hidden menu, lift it into a visible action. Spec wording: "mobile 补易找的入口".
- [ ] **Verify:** `flutter analyze` clean; manual tap-through.

**Phase 2 exit:** `pnpm --filter web run build` clean; `flutter analyze` clean; visually confirm skills toggle survives a refresh.

---

## Phase 3 — Workspace MRU + agent-profile boundary

**Goal:** Spec §5.4 / §5.5 — remember last-used workspaces per host, and enforce in code that `AgentProfile` is strictly client-local.

### Task P3.1 — Web workspace MRU store

**Files:** create `apps/web/src/lib/workspace-mru.ts`, `apps/web/src/components/workspace-mru.tsx`; modify `apps/web/src/App.tsx`.

- [ ] **Step 1:** `workspaceMru.load(hostDeviceId): string[]` returns up to 8 entries. `push(hostDeviceId, workspace)` dedups + prepends.
- [ ] **Step 2:** Storage key: `minos.web.workspace-mru.<host_device_id>` (JSON string array).
- [ ] **Step 3:** On successful `minos_start_agent`, push the workspace into the MRU for the active host.
- [ ] **Step 4:** `WorkspaceMruChips` component renders the MRU as clickable chips above the workspace input.
- [ ] **Verify:** After sending two threads on host A with different workspaces, chips reflect both; switching to host B shows B's MRU.

### Task P3.2 — Mobile workspace MRU store

**Files:** create `apps/mobile/lib/infrastructure/workspace_mru_store.dart`, `apps/mobile/lib/presentation/widgets/workspace_mru_chips.dart`; modify `apps/mobile/lib/application/minos_providers.dart`, `apps/mobile/lib/presentation/pages/agents_hub_page.dart`.

- [ ] **Step 1:** Store persists via `SharedPreferences` with key `minos.workspace-mru.<host_device_id>`.
- [ ] **Step 2:** Provider `workspaceMruProvider(hostDeviceId)` exposes `AsyncNotifier<List<String>>`.
- [ ] **Step 3:** Mount `WorkspaceMruChips` inside the new-thread composer.
- [ ] **Verify:** `flutter analyze`, widget test with 9 pushes leaves 8 entries.

### Task P3.3 — Profile-is-local guard

**Files:** `apps/web/src/lib/agent-profiles.ts`, `apps/mobile/lib/application/agent_profile_store.dart` (or equivalent).

- [ ] **Step 1:** Add a top-of-file comment + a `__SAFETY_ASSERT__` export explaining "profiles are client-only; never POST them to the backend" for future maintainers.
- [ ] **Step 2:** Add a grep-based xtask rule — extend `xtask/src/lint_naming.rs` (already present from plan 12) or add a parallel `lint_profiles_client_only.rs` that fails if `agent_profile` appears in any `crates/minos-backend/src/**` file.
- [ ] **Verify:** `cargo xtask check-all` green.

**Phase 3 exit:** Build + analyze clean on all three clients; MRU survives a reload.

---

## Phase 4 — Host MenuBar: account + `minos_id` surface

**Goal:** Spec R1.8 — MenuBar shows bound `account_email` / `mobile_device_name` per peer; `minos_id` read-only when at least one account is bound.

### Task P4.1 — Peer row view

**Files:** create `apps/macos/Minos/Presentation/AccountPeerRowView.swift`; modify `apps/macos/Minos/Presentation/MenuBarView.swift`.

- [ ] **Step 1:** `AccountPeerRowView` takes `PeerSummary` (pre-existing DTO in Generated UniFFI surface) and renders a two-line row: bold = `account_email ?? mobile_device_name`, secondary = `mobile_device_name` when the bold line used the email.
- [ ] **Step 2:** Replace the current list row in `MenuBarView` peers section with this component.
- [ ] **Verify:** Swift build via `xcodebuild -project apps/macos/Minos.xcodeproj -scheme Minos` (or XcodeGen) succeeds.

### Task P4.2 — Minos ID footer (R1.8)

**Files:** modify `apps/macos/Minos/Presentation/MenuBarView.swift` or `AgentSegmentView.swift`.

- [ ] **Step 1:** If the daemon surfaces ≥ 1 peer with a populated `account_id`, fetch `/v1/me/profile` via the daemon's existing HTTP path and cache the `minos_id`. If the daemon does not yet proxy this, add a thin `me_profile_query` uniffi method.
- [ ] **Step 2:** Render the `minos_id` as a read-only label at the bottom of the menu, or hide the label entirely if no account is bound.
- [ ] **Verify:** Manual smoke with a paired mobile account; unpair the last account → label disappears.

**Phase 4 exit:** XcodeGen project still builds; MenuBar screenshots reviewed.

---

## Phase 5 — Backend logging + secret redaction audit

**Goal:** Spec R7.1, R7.5, R7.6. Add file-backed logging under `$MINOS_HOME/logs`, confirm secret fields never leak to tracing output, and make `/health` HTTP-503 when DB is unreachable.

### Task P5.1 — File-backed tracing sink

**Files:** create `crates/minos-backend/src/logging.rs`; modify `crates/minos-backend/src/main.rs`, `crates/minos-backend/src/lib.rs`, `crates/minos-backend/Cargo.toml` if a new crate (e.g., `tracing-appender`) is needed.

- [ ] **Step 1:** `logging::init(log_dir: &Path)` layers the existing stdout subscriber with a `tracing-appender::rolling::daily(log_dir, "backend")` non-blocking writer. Keep the stdout layer — container deployments depend on it.
- [ ] **Step 2:** Directory resolution: `env::var("MINOS_HOME").map(|h| PathBuf::from(h).join("logs"))` else `PathBuf::from("./.minos-data/logs")`. Fail startup if `create_dir_all` fails.
- [ ] **Step 3:** Every log line carries at minimum `device_id` **or** `account_id` (spec NFR). Use `tracing::instrument(fields(account_id = ...))` on request handlers.
- [ ] **Verify:** Start the backend, send requests, `ls $MINOS_HOME/logs/` shows rotated files.

### Task P5.2 — Secret redaction audit (R7.5)

**Files:** sweep `crates/minos-backend/src/http/v1/auth.rs`, `crates/minos-backend/src/http/v1/pairing.rs`, `crates/minos-backend/src/pairing/**`, `crates/minos-backend/src/session/**`.

- [ ] **Step 1:** Every handler that takes a body containing `password` / `refresh_token` / `device_secret` must use `#[tracing::instrument(skip_all)]` or equivalent `skip(body)`.
- [ ] **Step 2:** Grep for `tracing::debug!` / `info!` / `warn!` lines that format request bodies; replace with explicit fields.
- [ ] **Step 3:** Create `crates/minos-backend/tests/secret_redaction.rs`: use `tracing-subscriber::fmt::TestWriter`; POST to `/v1/auth/register` with a unique password; assert the password string does not appear in the captured buffer.
- [ ] **Verify:** `cargo test -p minos-backend secret_redaction` passes.

### Task P5.3 — Request trace middleware

**Files:** modify `crates/minos-backend/src/http/mod.rs`.

- [ ] **Step 1:** Add a `tower::Layer` that measures request duration and logs `method`, `path`, `status`, `latency_ms`, `account_id` (pulled from `Extensions` after auth).
- [ ] **Step 2:** Ensure 4xx/5xx responses still log without full body.
- [ ] **Verify:** `curl` a couple of endpoints and eyeball the log line shape.

### Task P5.4 — Health check with DB ping (R7.6)

**Files:** `crates/minos-backend/src/http/health.rs`.

- [ ] **Step 1:** On GET /health, run `sqlx::query("SELECT 1").execute(pool).await`. On error return `StatusCode::SERVICE_UNAVAILABLE` with body `{ "status": "degraded", "reason": "db" }`.
- [ ] **Step 2:** On success: `{ "status": "ok", "version": env!("CARGO_PKG_VERSION") }`.
- [ ] **Verify:** Kill the DB file (or `chmod 000` in tmp dir) and confirm 503.

**Phase 5 exit:** `cargo xtask check-all` green; smoke `curl /health` returns 200; logs appear in `$MINOS_HOME/logs`.

---

## Phase 6 — Claude / Gemini minimal PTY adapter

**Goal:** Spec R3.4 — `start_agent { agent: Claude | Gemini, workspace }` spawns the CLI, pipes stdout/stderr lines as `UiEventMessage::Raw`, pipes composer text into stdin. No structured translation; the fallback `Raw` variant is explicitly permitted.

**Risk (spec §9 #1):** PTY semantics differ per CLI. Scope here is "don't crash, don't drop bytes, don't block the runtime".

### Task P6.1 — `PtyAgent` module

**Files:** create `crates/minos-agent-runtime/src/pty_agent.rs`, `crates/minos-agent-runtime/tests/pty_agent_smoke.rs`; modify `crates/minos-agent-runtime/src/lib.rs`, `crates/minos-agent-runtime/src/manager.rs`, `crates/minos-agent-runtime/Cargo.toml` (add `portable-pty` or `tokio::process::Command` depending on capabilities).

- [ ] **Step 1:** Prototype with `tokio::process::Command` first (simpler; PTY only if a target CLI proves to need a TTY).
- [ ] **Step 2:** `PtyAgent::spawn(cli_path, workspace) -> PtyAgent` starts the child in `workspace`, wires stdin/stdout/stderr.
- [ ] **Step 3:** `PtyAgent::send_user_message(text)` writes `text + "\n"` to stdin, flushes.
- [ ] **Step 4:** A reader task line-buffers stdout+stderr, emits `UiEventMessage::Raw { raw_kind: "stdout" | "stderr", payload_json: serde_json::to_string(&line)? }` through the existing `EventWriter` (single-writer invariant holds).
- [ ] **Step 5:** `PtyAgent::close()` sends SIGTERM then SIGKILL after 3 s; emits `UiEventMessage::ThreadClosed { reason }`.

### Task P6.2 — Wire through `AgentManager`

**Files:** `crates/minos-agent-runtime/src/manager.rs`, `crates/minos-agent-runtime/src/thread_handle.rs`.

- [ ] **Step 1:** Match on `AgentName`. `Codex` stays on `codex_client`. `Claude | Gemini` → `PtyAgent`.
- [ ] **Step 2:** Use `cli_detect::status_of(agent)` before spawn. If missing, return JSON-RPC error `{ code: "cli_missing", data: { agent } }` and do **not** create a thread (spec R3.4 explicit).

### Task P6.3 — Smoke test

**Files:** `crates/minos-agent-runtime/tests/pty_agent_smoke.rs`.

- [ ] **Step 1:** Spawn `/bin/sh -c 'printf "hello\n"; read line; printf "got %s\n" "$line"'`.
- [ ] **Step 2:** Assert two `Raw { raw_kind: "stdout", payload_json: "\"hello\"" }` — one before send, one after.
- [ ] **Step 3:** Assert `send_user_message("world")` arrives on stdout as "got world".
- [ ] **Verify:** `cargo test -p minos-agent-runtime pty_agent_smoke` passes locally on macOS.

**Phase 6 exit:** Host can start claude / gemini threads; remote clients see `Raw` events in the transcript (rendered as monospaced blocks via existing `raw` variant). Structured translation remains an explicit follow-up spec.

---

## Phase 7 — Protocol round-trip & contract tests

**Goal:** Spec R9 — parse(serialize(x)) == x for every wire type, including the `Raw` fallback. Lock the invariants as CI gates.

### Task P7.1 — Envelope round-trip

**Files:** modify `crates/minos-protocol/tests/envelope_golden.rs`, extend goldens under `crates/minos-protocol/tests/golden/envelope/`.

- [ ] **Step 1:** Add property-style test (no need for `proptest` crate; a hand-rolled loop over representative variants is enough) that encodes each variant and decodes back.
- [ ] **Step 2:** Add golden fixtures for `EventKind::IngestCheckpoint`, `ServerShutdown`, `PeerOnline`, `PeerOffline`, `Forward` with arbitrary nested JSON params.
- [ ] **Verify:** `cargo test -p minos-protocol envelope`.

### Task P7.2 — UI protocol round-trip

**Files:** create `crates/minos-ui-protocol/tests/roundtrip.rs`.

- [ ] **Step 1:** Enumerate every `UiEventMessage` variant, including `Raw { raw_kind: "anything", payload_json: "\"data\"" }`.
- [ ] **Step 2:** Assert `serde_json::from_str::<UiEventMessage>(&serde_json::to_string(&x)?)? == x`.
- [ ] **Verify:** `cargo test -p minos-ui-protocol roundtrip`.

### Task P7.3 — Pairing QR round-trip (R9.3)

**Files:** wherever `PairingQrPayload` is defined (`crates/minos-protocol/src/messages.rs`) + its test.

- [ ] **Step 1:** Add round-trip test for `{ v:2, host_display_name, pairing_token, expires_at_ms }`.
- [ ] **Step 2:** Add explicit test that web and mobile decoders use the same Rust-emitted JSON (fixture-driven — decode a Rust-produced string on the TS side via `scripts/` helper if feasible; if not, at least grep mobile/web parse call sites to confirm they use the central DTO).

**Phase 7 exit:** New tests are CI-blocking; spec R9 automatically enforced on future PRs.

---

## Verification gate per phase

| Phase | Commands |
|---|---|
| P1 | `pnpm --filter web run lint && pnpm --filter web run build` |
| P2 | Phase-1 commands + `flutter analyze` (mobile touch) |
| P3 | Phase-1 commands + `flutter analyze` + `cargo xtask check-all` |
| P4 | `xcodegen --spec apps/macos/project.yml` → `xcodebuild -project apps/macos/Minos.xcodeproj -scheme Minos build` |
| P5 | `cargo xtask check-all` + `cargo test -p minos-backend` |
| P6 | `cargo test -p minos-agent-runtime` + smoke start_agent from mobile |
| P7 | `cargo test -p minos-protocol -p minos-ui-protocol` |

---

## Traceability — spec R* → phase / task

| Spec ID | Landing phase | Task(s) |
|---|---|---|
| R1.1 – R1.7 | — (already covered by backend) | regression-only |
| R1.8 | P4 | P4.1, P4.2 |
| R2.1 – R2.7 | — (already covered) | regression-only |
| R2.8 | P1 | P1.4 |
| R3.1 – R3.3 | — | regression-only |
| R3.4 | P6 | P6.1 – P6.3 |
| R3.5 | — (already covered) | regression-only |
| R3.6 | P1 | P1.2 |
| R3.7, R3.8 | — | regression-only |
| R4.1 – R4.5 | — | regression-only |
| R4.6 | P1 | P1.2 |
| R4.7 | P1 | P1.2 (disable logic) |
| R4.8 | P1 | P1.3 |
| R4.9 | P3 | P3.1 – P3.3 |
| R5.1 – R5.4 | P2 | P2.3, P2.4 |
| R6.* | — (already covered) | regression-only |
| R7.1 | P5 | P5.1 |
| R7.2, R7.3 | — | regression-only |
| R7.4 | P2 | P2.1, P2.2 |
| R7.5 | P5 | P5.2, P5.3 |
| R7.6 | P5 | P5.4 |
| R8.1 – R8.3 | — | regression-only |
| R8.4 | P1 | P1.9 |
| R8.5 | — | regression-only |
| R9.1 – R9.4 | P7 | P7.1 – P7.3 |
| R10.1 | P2 | P2.2, P2.3 |
| R10.2 | P1 | P1.7 |
| R10.3 | P1 | P1.6 |
| R10.4 | — (already shape-correct) | regression-only |
| R10.5 | P1 | P1.8 |

---

## Open decisions to confirm before execution

1. **PTY library (P6):** start with `tokio::process::Command` or commit to `portable-pty` up front? Default: plain `Command`; revisit only if claude or gemini refuse to produce output without a TTY.
2. **Backend log rotation cadence (P5.1):** default `daily`; switch to `hourly` if ops requests it.
3. **Observability drawer UX (P2.2):** floating drawer vs. a new top-level tab alongside `console` / `social` / `agents`. Default: floating drawer to keep the tab count stable.
4. **Workspace MRU cap:** 8 entries per host. Bump if users request.
5. **Phase interleaving:** phases are independent enough to parallelize P4, P5, P6 once P1 is merged. Default: still land P1 first to minimize review context-switches.

---

## Rollout order (recommended)

1. **P1** (review-heavy UX) — merge first; unblocks web users.
2. **P7** (contract tests) — cheap, landable alongside P1 to prevent regressions in later phases.
3. **P2** (observability) — small blast radius on web, pure additive on mobile.
4. **P3** (MRU) — touches three clients but is additive.
5. **P5** (backend) — merge during a quiet window; operational change.
6. **P4** (macOS) — review cadence dominated by XcodeGen churn.
7. **P6** (PTY) — last, because its follow-up (typed claude/gemini protocol) will rewrite parts of it.
