# D05 · Projection & Sync (Host → Cloud → Viewers)

| Field | Value |
|-------|--------|
| Domain ID | D05 |
| Status | Refined (2026-07-31) |
| L0 | Visibility matrix in [long-term spec §4.6](../2026-07-30-cloud-identity-clients-long-term.md) |
| Tasks | `T-proj-*` in [tasks/TASKS.md](tasks/TASKS.md) |
| Depends on | P0 for baseline; **D02** for Linked host; D03/D04 for viewers |
| Blocks | "Phone can see Desktop work" product claim |

---

## 1. Goal

确保 host **Linked** 且 online 时：

1. 相关的 **conversation / agent session** 活动在 hub 上可用。
2. Mobile/Web **list + subscribe + stream** 看到相同的产品真实状态。
3. Desktop UI 对 **local-only** 数据诚实标注。

```text
Daemon (agents) ──ingest/events──► minos-backend ──fanout──► /ws/client viewers
```

---

## 2. Decisions (locked)

| # | Decision |
|---|----------|
| 1 | Cloud 不运行 agents；它**投影** host 活动 |
| 2 | Local-only history 允许存在；如果未投影必须标注 |
| 3 | Golden path projection 优先（start/stream/send/stop）；full parity 后续 |
| 4 | Multi-host：viewers 指定具体 host（ADR 0020） |

---

## 3. Visibility matrix (normative)

| Artifact | Local daemon | Cloud (Linked) |
|----------|--------------|----------------|
| CLI processes / raw FS | Yes | No (default) |
| Agent session lifecycle | Yes | Yes if ingested |
| Transcript / turns | Yes | Yes if ingested |
| Approvals | Yes | Remotable via `host_commands` path |
| Agent profiles | Yes | Optional later |

---

## 4. Workstreams

### 4.1 Audit（可立即开始，`T-proj-01`）

- 映射 Desktop/daemon 操作 → backend endpoints / events / tables
- 产出 gap list："UI 可以本地做 X 但 cloud 从不知道"
- **重点检查**：Daemon 当前是否在 Linked 状态下正确推送 ingest？`HostIngestLiveBatch` 是否在 `/ws/host` 连接时自动开始？

### 4.2 Completeness（D02 之后）

- 修复 golden path 的 ingest/fanout gaps
- 在最窄可靠边界添加 regression tests
- 验证 Mobile + Web 对同一 session id 看到相同数据

### 4.3 Honesty UX

- Desktop：conversation/session 未 cloud-visible 时显示 badge
- Mobile/Web：无 Linked host / host offline 时显示空状态

---

## 5. Known related debt

- Background jobs 引用 legacy relation/enum names（prod logs 中可见）
- `pairing_codes` 表移除后，确保没有 job 还在引用它（`T-cleanup-*` 任务覆盖）
- Store 层从 `devices`/`account_host_pairings` 迁移到 `device_installations`/`host_links` 后，所有 SQL 查询更新

---

## 6. Implementation note: ingest 依赖 host_links

Daemon 通过 `/ws/host` 连接后，backend 需要知道这个 host 属于哪个 account 才能把 ingest 数据 fan-out 到该 account 的 `/ws/client` viewers。

当前实现中，这个映射通过 `pairing_codes` → `account_host_pairings` 建立。移除后，改用 `host_links` 表：

```sql
-- 给定 host_installation_id，找到 account_id：
SELECT account_id FROM host_links WHERE host_installation_id = $1;
```

`ingest` 模块的 peer target lookup 需要更新（`invalidate_peer_targets_for_account` 等）。

---

## 7. Open questions

以下问题**不阻塞第一里程碑**（Desktop ↔ Mobile 联通的 golden path projection）。如果后续发现它们是前置条件，则添加对应的 `T-proj-*` 任务。

1. Local-only Desktop conversations 是否在 Link 时自动创建 cloud projection？还是只投影 Link 之后的新 session？（倾向：只投影新 session；历史 backfill 是后续 polish）
2. Projected events 在 hub 的保留策略 vs host SQLite SSOT（深度历史）？（倾向：hub 短期保留 + host SQLite 长期 SSOT；具体策略后续定）
3. Subagent trees：Mobile v1 全量投影还是折叠？（`T-proj-09` 覆盖此决策）

---

## 8. Exit criteria

- [x] Gap audit 文档完成并链接到此 → [projection-gap-audit.md](projection-gap-audit.md)
- [x] Golden path（hub 已注册 session）：start/stream/send/stop + live-batch fanout 有自动化覆盖
- [ ] Offline host：viewers 显示 offline；不 silent hang（`T-proj-05` UX）
- [x] 至少一个自动化测试覆盖 projection 或 fanout invariant
  - `host_ingest_live_batch_fans_out_projection_to_subscribed_client`
  - `host_ingest_live_batch_records_approval_request`
  - unit: `peer_targets_*` in `ingest::tests` (via `host_links`)
- [x] ingest peer target lookup 使用 `host_links` 而非 `account_host_pairings`
- [ ] 三端 manual E2E（`T-proj-03`，依赖 Mobile/Web ports）

---

## 9. Phase G implementation notes (2026-08-01)

Formal gateway path for golden stream:

1. Daemon online → `HostIngestLiveBatch` chunks with host-side `projection`.
2. Backend accepts only when `agent_sessions` exists and `host_installation_id` matches (or claims empty).
3. On first insert: promote `pending → running`, record approval side effects from payload, sync formal turn/session from projection, then fan out `StreamEvent{kind: ui_event}` to topic subscribers on `/ws/client`.
4. Peer targets for legacy envelope fanout: `host_links::list_account_client_targets_for_host`.

**Not projected without hub registration:** Desktop-local-only agent sessions (no cloud `agent_sessions` row) — intentional until host registration/backfill; honesty UX is `T-proj-04`.

---

## 10. Manual E2E checklist (Desktop Link → Mobile same account)

Use when hub + Linked host + Mobile cloud login are available:

1. [ ] Desktop: account login + **Link this Mac** succeeds; host shows Linked.
2. [ ] Mobile: same account; `GET /v1/hosts` shows the host (online when daemon WS up).
3. [ ] From Mobile (or API): start agent session on that `host_installation_id` with a conversation the account can access.
4. [ ] Mobile: subscribe to `agent_session:{session_id}` (app open thread); observe stream deltas while host agent runs.
5. [ ] Mobile: send follow-up input; host receives `agent_session.send_input` and continues.
6. [ ] If agent requests approval: Mobile sheet appears; respond; host continues / stops accordingly.
7. [ ] Stop session from Mobile; host stops; status no longer running.
8. [ ] Kill host WS (quit Desktop / network): Mobile host list or command path shows offline / peer_offline — no infinite hang.
9. [ ] Two hosts (optional): start with explicit `host_installation_id`; command hits intended host only.

Automated substitute for steps 3–6 stream/approval: `cargo test -p minos-backend --test ws_gateway host_ingest_live_batch`.

---

## 11. Task slice

`T-proj-01` … `T-proj-09` in [tasks/TASKS.md](tasks/TASKS.md).
