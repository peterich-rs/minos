# Desktop State Management Review — P0–P4 (+ P5 cleanup)

> Date: 2026-07-22  
> Scope: `apps/desktop` state by consumption (spec §18)  
> Spec: [2026-07-21-desktop-state-by-consumption.md](../specs/2026-07-21-desktop-state-by-consumption.md)  
> Architecture: [architecture-desktop.md](../../architecture-desktop.md)

---

## 1. Summary — what landed vs spec

| Phase | Spec intent | Code result |
|-------|-------------|-------------|
| **P0** | No live blind poll; conversation dirty → conditional messages-only re-list | **Done.** Timeline / Sessions list / Transcript intervals all gate on `livePush === false`. `applyConversationEvent` debounces (~200ms) → `hasTimelineWorkingSet` ? `loadTimeline({ quiet })` : mark `timelineDirty` only. |
| **P1** | Split detail load; `detailsOpen` gates Inspector | **Done.** `loadTimeline` / `loadInspector` with independent `timelineStatusByConversation` / `inspectorStatusByConversation`. `SessionInspector` mounts load only when `detailsOpen`; `@` mention path can quiet-ensure Inspector. No `loadConversationDetail`. |
| **P2** | SessionEntity sole status / `hasPendingApproval` writers | **Done.** `lib/session-entity.ts` + `sessionsById`; live + resolve paths use `mergeSessionEntity` / `patchSessionEntity` / `commitSessionEntity` (+ list projection). Hydrate list loaders merge into Entity then project membership for that list. |
| **P3** | SessionList only `list_project_sessions`; no `projectSessions` mirror; Attention queue vs badge | **Done.** `loadProjectSessions` → `listProjectSessions(projectId)`. No global `projectSessions` mirror. Attention `loadAttentionSessions` hydrates queue + upserts Entity; sidebar badge remains Σ `project.needsAttention` (scheme A). |
| **P4** | single-flight, hardMax, mock no-op, ReadReceipt, module inflight | **Done.** `lib/desktop-inflight.ts` (`singleFlightLoad`, resume Sets). Timeline hardMax 500 / Transcript 2000. Mock source early-returns on loads. `readMessageCountById` persisted via zustand `partialize`. No `window.__minos*`. |
| **P5** | Optional store split; cleanup / review | **Done (residual pass).** Anti-pattern sweep earlier; multi-file `store/workspace/*` split + thin `workspace-store.ts` re-export. |

**Key paths (canonical):**

| Concern | Path |
|---------|------|
| Entity API | `apps/desktop/src/lib/session-entity.ts` |
| In-flight / single-flight | `apps/desktop/src/lib/desktop-inflight.ts` |
| Store + LiveIngress | `apps/desktop/src/store/workspace-store.ts` |
| Nav / `detailsOpen` | `apps/desktop/src/store/ui-store.ts` |
| Timeline consumption | `apps/desktop/src/components/shell/Timeline.tsx` |
| Inspector consumption | `apps/desktop/src/components/shell/SessionInspector.tsx` |
| SessionList | `apps/desktop/src/components/shell/SessionsView.tsx` |

---

## 2. Verification

Commands run from `apps/desktop` (2026-07-22):

| Command | Result |
|---------|--------|
| `pnpm check` (`tsc --noEmit`) | **Pass** (after Timeline unused cleanup) |
| `pnpm test` (`src/lib/*.test.ts`) | **Pass — 138 tests, 0 fail** |

Unit coverage includes: `session-entity`, `message-history` / `hasTimelineWorkingSet` / hardMax, `transcript-history` / `hasTranscriptWorkingSet` / hardMax, `session-status`, agent-route, stick-to-bottom, etc.

**Not run (out of scope for this review):** full Tauri e2e, network-panel live RPC audit, mobile/TUI.

---

## 3. Correctness checklist

### §2.2 — Dual-layer Ingress + keyed cache

| Invariant | Verdict | Evidence |
|-----------|---------|----------|
| Ingress writes Entity + dirty, not full message/transcript mirror | **Pass** | `applyIngestEvent` / `applyManagerEvent` / `applyConversationEvent` |
| Conversation dirty without Timeline entry → **zero** `list_messages` | **Pass** | `hasTimelineWorkingSet` gate + `timelineDirtyByConversation` |
| Quiet re-list only when working set exists | **Pass** | same path; View `ensureLoaded` clears dirty on load |
| Ingest without Transcript key does not `?? []` create window | **Pass** | `hasTranscriptWorkingSet`; Entity may still elevate pending |
| View does not poll when `livePush` healthy | **Pass** | Timeline / SessionsView / transcript intervals `if (livePush) return` |

### §6.5 — Attention badge vs queue

| Invariant | Verdict | Evidence |
|-----------|---------|----------|
| Badge = Σ loaded projects’ `needsAttention` (scheme A) | **Pass** | Conversation list aggregate → project; not driven by Attention queue |
| Attention detail list hydrates on open | **Pass** | `loadAttentionSessions` |
| Attention does not invent membership from live alone | **Pass** | `projectEntityIntoLists` updates/drops existing Attention rows only |

**Scheme A limits (accepted residual):** projects never hydrated in ConversationList may under-count badge until opened/listed.

### §11 / consumption-driven load (selected)

| Invariant | Verdict | Notes |
|-----------|---------|-------|
| `detailsOpen=false` → no `list_sessions` for open conversation | **Pass** | Inspector effect requires `detailsOpen`; exception: `@` mention ensure |
| Timeline ∥ Inspector independent phase | **Pass** | Separate status maps |
| `selectConversation` / `openConversation` only change nav ids | **Pass** | `ui-store` — no load APIs |
| SessionList hydrate sole RPC `list_project_sessions` | **Pass** | `loadProjectSessions` |
| No `projectSessions` global mirror | **Pass** | only `projectSessionsByProject` keyed cache + Entity |

### §17 — Full invariant list

| # | Invariant | Verdict |
|---|-----------|---------|
| 1 | Switch project must not show previous project transcript | **Pass** (nav clears / key isolation; not re-audited e2e) |
| 2 | Timeline order by `messageSeq` | **Pass** (`sortTimelineMessages`) |
| 3 | Timeline excludes session tool stream | **Pass** (messages only) |
| 4 | `needs_approval` elevate; manager running does not demote while pending | **Pass** (`applyManagerLifecycleToEntity`) |
| 5 | Header conversation count after list ready | **Pass** (architecture / header path) |
| 6 | Same `sessionId` status consistent across surfaces via Entity | **Pass with residual** — live commits project into all lists; **hydrate-only** list loads rewrite Entity + own membership list, may leave sibling list caches stale until live event or their own hydrate |
| 7 / 16 | `livePush=true` no blind interval | **Pass** |
| 8 | Quiet re-list keeps older window | **Pass** (`mergeMessagesQuietTail`) |
| 9 | Nav persist; business lists not truth persist | **Pass** (`partialize` only ReadReceipt) |
| 10 | Views do not cross-write other slices | **Pass** (loads/use-cases in store) |
| 11 | `detailsOpen` gates Inspector RPC | **Pass** |
| 12 | Independent Timeline/Inspector phase | **Pass** |
| 13 | No implicit dual hydrate on select | **Pass** |
| 14–15 | Ingress thin + conditional quiet | **Pass** |
| 17 | Ingest working-set gate | **Pass** |
| 18 | Stable empty selectors (`EMPTY_*`) | **Pass** (components use module EMPTY constants; spot-checked) |
| 19 | Attention badge scheme A | **Pass** (limits noted) |
| 20 | SessionList only project sessions RPC | **Pass** |
| 21 | `hasPendingApproval` fallback when no transcript | **Pass** |
| 22 | Board reads list aggregates | **Pass** |
| 23 | No `projectSessions` mirror | **Pass** |
| 24 | Mock no-op loads; ReadReceipt persist; no `window.__minos*` | **Pass** |
| 25 | single-flight + hardMax | **Pass** |

---

## 4. Risks / residual gaps

| Severity | Item | Detail |
|----------|------|--------|
| ~~**Medium**~~ | ~~Entity dual list caches still projecting~~ | **Addressed** — see §7. |
| ~~**Medium**~~ | ~~`livePush` mid-session disconnect~~ | **Addressed** — see §7. |
| ~~**Low**~~ | ~~Attention scheme A under-count~~ | **Addressed** (quiet full project ConversationList) — see §7. |
| ~~**Low**~~ | ~~God `workspace-store`~~ | **Addressed** — P5 multi-file split — see §7. |
| **Low** | send / retry path convergence | `startNewAgentSession` shared; full send pipeline not fully unified (spec P4 note). |
| **Nit** | Architecture header still says “UI mock” in overview table | Stale product-stage blurb; state sections updated. |

### Bugs found this pass

| Severity | Finding | Action |
|----------|---------|--------|
| **Low** | `Timeline.tsx`: unused `AlertCircle` + dead `onRetryClick` (tsc fail) | **Fixed** — remove import; fold retrying into `handleRetry` |
| **Medium** | `respondOpencodePermission` / `respondOpencodeQuestion` updated transcript only, deferred Entity until `loadTranscript` | **Fixed** — same Entity + `commitSessionEntity` + approvalCount demote path as `resolveApproval` |
| — | Critical / High | **None found** |

### Anti-pattern grep (desktop code)

| Pattern | Result |
|---------|--------|
| `loadConversationDetail` | **Absent** in `apps/desktop` (docs only / history) |
| `projectSessions` global mirror | **Absent** (only `projectSessionsByProject` + local vars) |
| `window.__minos` | **Absent** (comment in `desktop-inflight.ts` only) |
| `setInterval` while live | **Gated** — all three poll sites return when `livePush` |
| Dual status writers bypassing Entity API | **Live/use-case paths OK**; hydrate bulk merge uses `mergeSessionEntity` then local list assign (acceptable) |

---

## 5. Suggested next steps

1. **Optional P5 structural split:** carve Connection / L3 Timeline / L3 SessionList / LiveIngress modules from `workspace-store.ts` without behavior change.
2. **livePush disconnect:** if Tauri/bridge can surface unlisten/error, set `livePush=false` to re-enable degraded poll (closes residual Medium).
3. **UI reads Entity for status:** selectors that always take `sessionsById[id].status` for pills would eliminate dual-list lag without deleting membership caches.
4. **Scheme B Attention badge** only if product requires accurate red dots for never-opened projects.
5. **E2E smoke:** open conversation with details closed → confirm network has `list_messages` only; open details → `list_sessions`; live running → no 2–2.5s quiet poll spam.

---

## 6. P5 cleanup performed this session

- Grep + confirm anti-patterns cleared / gated.
- Fix Timeline tsc unused (`AlertCircle`, `onRetryClick`).
- Align OpenCode respond paths with Entity sole writers.
- Spec §16 migration table + §18 P5 status + revision log.
- Architecture desktop live/Entity/inflight notes + review link.
- `pnpm check` + `pnpm test` green.

---

## Verdict

**P0–P4 meet the consumption-driven state contract for Desktop.** Remaining work is optional structure (P5 split), live-disconnect hardening, and dual-list projection polish — not blockers for the stated §18 acceptance criteria.

---

## 7. Residual follow-up (2026-07-22 later) — addressed

| # | Residual | Resolution |
|---|----------|------------|
| 1 | Entity dual-list lag on hydrate | Pure `lib/session-list-projection.ts` + unit tests; hydrate paths upsert Entity → `rowsFromEntities` → `projectHydratedEntities` for sibling lists; Attention re-derives from Entity when `attentionStatus.phase==='ready'`. |
| 2 | `livePush` mid-session disconnect | Tauri pumps emit `daemon://push-status` `{live:false}` when current-gen ends / subscribe fails; arm emits `live:true`. Frontend bridge listens → `set({ livePush })`. Views already re-enable degraded poll when false. |
| 3 | Attention badge under-count | After bootstrap / `refreshProjects` / `createProject`: `quietHydrateAllConversationLists` (concurrency 4) so all known projects get ConversationList + DTO `approvalCount`. Spec §6.5 updated. Attention queue still open-to-load only. |
| 4 | P5 god store | Split into `store/workspace/{types,helpers,projection,shared,connection,conversation-list,timeline,inspector,session-list,transcript,attention,live-ingress,agents-host,use-cases,create-actions}.ts`; thin `workspace-store.ts` keeps `useWorkspaceStore` export path. |

**Verification (same day residual pass):** `cd apps/desktop && pnpm check` pass; `pnpm test` **146** pass (includes new projection tests).
