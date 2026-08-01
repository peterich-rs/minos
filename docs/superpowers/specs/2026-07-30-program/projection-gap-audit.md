# D05 · Projection Gap Audit (T-proj-01)

| Field | Value |
|-------|--------|
| Date | 2026-08-01 |
| Epic | `feature/host-link-mobile` / Phase G |
| Spec | [05-projection-sync.md](05-projection-sync.md) |
| Tasks | `T-proj-01` (audit), feeds `T-proj-02` / `T-proj-06` |

## 1. Architecture under audit

```text
Daemon (agents) ──HostIngestLiveBatch / HostStreamEvent──► minos-backend
                                                              │
                                                              ├─ persist raw_events
                                                              ├─ approval_requests side effects
                                                              ├─ formal agent_sessions status
                                                              └─ fanout StreamEvent ──► /ws/client viewers
```

Routing identity for fan-out is **account ↔ host** via `host_links`, then client installations under those accounts (`device_installations` with mobile/browser/desktop kinds). Peer-target cache invalidates on Host Link / unlink.

## 2. Gap matrix (Desktop / daemon action → cloud)

| Local / product action | Cloud path | Visible on Mobile/Web? | Gap? | Notes |
|------------------------|------------|------------------------|------|-------|
| Host Link this Mac | `POST /v1/hosts/link` → `host_links` + HIT; peer-target invalidate | N/A (identity) | **No** | Prerequisite for all projection |
| Host WS online | `/ws/host` ticket + Hello | Viewers see host online via hosts list | **No** | |
| Daemon live ingest (`IngestSyncHandle` → `HostIngestLiveBatch`) | gateway `persist_host_ingest_chunk` → topic fanout | **Yes if** formal `agent_sessions` row exists **and** client `Subscribe`s to `agent_session:{id}` | **Fixed** (status + approval) / residual below | Pre-fix: projection fanout only; status stayed `pending`; approvals not recorded |
| Gap recover on reconnect | `HostGapManifest` → `PullIngestRange` → `HostIngestPullResponse` | Yes for known sessions | **No** (path exists) | |
| Cloud start session | `POST` agent session API → durable `agent_session_started` + `host_command` `agent_session.start` | List + subscribe | **No** | Golden start |
| Cloud send input | host_command `agent_session.send_input` + durable turn | Yes | **No** | Golden send |
| Cloud stop | host_command `agent_session.stop` | Yes (status via command path) | **No** | Golden stop |
| Desktop **local-only** start agent (daemon RPC, no cloud start) | HostIngestLiveBatch for unknown `session_id` | **Yes if host Linked** — hub auto-registers formal `agent_sessions` (ensure conversation + claim host) then server-translates raw | **Mitigated (2026-08)** | Unlinked host still drops (rate-limited warn). Optional honesty badge remains `T-proj-04` |
| Stream UI deltas (projected) | `StreamEvent{kind: ui_event}` to topic subscribers | Yes when subscribed | **No** after fix | Regression: `host_ingest_live_batch_fans_out_projection_to_subscribed_client` |
| Approval request on host | payload `method=approval/request` inside live batch | UI may show Raw; respond needs row | **Fixed** | Was half-broken: no `approval_requests` insert on live batch; Postgres legacy path also skipped. Regression: `host_ingest_live_batch_records_approval_request` |
| Approval respond from Mobile | `POST /v1/approvals/respond` → host_command `minos_approval_decision` | Yes if row + host_link exists | **No** after record fix | Authorization uses `host_links` (via `account_host_pairings` facade) |
| Multi-host command routing | optional `host_installation_id` on start; else first `list_hosts_for_account` | Commands hit selected host | **Residual** | Default host is first listed; `T-proj-08` smoke |
| Host offline | no `/ws/host`; host_commands fail peer_offline | Empty / offline messaging | **Residual UX** | `T-proj-05` |
| Subagent trees | same ingest stream under child session ids | Partial | **Open** | `T-proj-09` |

## 3. Focus checks from spec §4.1

| Check | Result |
|-------|--------|
| Does daemon push ingest when Linked / WS connected? | **Yes** — `IngestSyncHandle` spawns with daemon handle; `submit_live` when `RelayLinkState::Connected`; dirty ranges when offline/queue full |
| Does `HostIngestLiveBatch` start automatically on `/ws/host`? | **Yes** — live upload loop is process-lifetime; frames enqueue to host WS when connected (not “on connect only”) |
| Peer target lookup uses `host_links`? | **Yes** — `peer_targets_for_host` → `host_links::list_account_client_targets_for_host`; cache invalidate on link/unlink |

## 4. Bugs fixed in Phase G (T-proj-02)

1. **`HostIngestLiveBatch` did not record approvals** — Mobile could show Raw approval UI but `/v1/approvals/respond` returned not found. Now calls `apply_approval_side_effects_from_payload` on insert.
2. **Formal session status stuck at `pending`** under live batch path — now promotes `pending → running` on first accepted chunk and applies `sync_formal_agent_session_from_ui_events` on projection.
3. **Postgres legacy ingest skipped all approval events** (`should_skip_external_sql_approval_event`) — removed; store layer already supports Postgres `approval_requests`.
4. **Missing regression for live-batch fanout** — added gateway integration tests (T-proj-06).

## 5. Explicit non-goals this phase

- Mobile UI redesign (Phase F)
- Full ACP feature matrix parity
- Auto-projecting Desktop-local sessions without hub `agent_sessions` row (history backfill / local-to-cloud registration)
- Re-adding QR pairing

## 6. E2E checklist (manual — T-proj-03 precursor)

See [05-projection-sync.md § Exit / E2E](05-projection-sync.md) and the checklist at the end of that doc after Phase G updates.

## 7. Residual risks

| Risk | Severity | Mitigation / follow-up |
|------|----------|------------------------|
| Desktop-local sessions never create hub rows → silent drop of ingest | High for “see my Desktop-only chat on phone” | Product honesty badge (`T-proj-04`); optional later: host-side session registration API |
| Viewers must Subscribe to `agent_session:{id}` or miss stream | Medium | Mobile/Web data plane already subscribe; document |
| Default host selection for multi-host accounts is non-explicit | Medium | UI must pass `host_installation_id` (`T-proj-08`) |
| Live batch fanout is in-process only (not Redis cluster bus) | Medium multi-instance | Formal durable events use bus; stream is sticky to gateway instance |
| Approval timeout poller races synthetic timestamps in tests | Low | Tests use `timeout_ms: 0` or wall-clock `ts_ms` |
