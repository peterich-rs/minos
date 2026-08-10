# Separate Bot Identity from Per-Conversation Session Context

| Field | Value |
|-------|-------|
| Status | Proposed |
| Date | 2026-08-10 |
| Author | opencode review |
| Related | [ADR 0021](../../adr/0021-agent-as-conversation-bot-participant.md), [agent-participant-delivery](2026-08-09-agent-participant-delivery.md), [client-im-sync-engine](2026-08-03-client-im-sync-engine.md), [architecture-messaging.md](../../architecture-messaging.md) |
| Non-goals | E2EE; cloud-side CLI execution; giving agents human Account login; backward-compat layers for old wire shapes |

## Breaking Change Notice

This spec changes **three** things that are semver-breaking for in-repo consumers:

1. **Daemon `conversation_agent_members` key changes** from `(conversation_id, agent_runtime_string)` to `(conversation_id, bot_id)`. All daemon store queries, roster logic, and mention resolution must be updated atomically. The daemon local SQLite is wiped-on-migrate (latest-only policy), so no ALTER migration is needed — the fresh `0001` schema is the target.

2. **Backend `agents` table gains identity-layer columns** (`system_prompt`, `reasoning_effort`, `env_json`, `display_name`) that previously lived only in daemon `agent_profiles`. The `agent_profiles` table on the daemon becomes a local cache of cloud-owned bot identities, not the identity SSOT.

3. **Wire `ChatMessageSummary.sender` for agent messages** stops abusing `UserSummary.account_id = agent_id`. A new `BotSender` discriminated field is introduced. Transitional clients that branch on `sender_type` still work; clients that read `sender.account_id` for agent rows must migrate to `sender.bot_id`.

**Migration steps for in-repo consumers:**
1. Daemon: replace all `agent: &str` (runtime name) parameters in store/roster/mention APIs with `bot_id: &str`. The runtime is resolved from the bot identity row, not passed as a bare string.
2. Clients: when rendering agent messages, read `sender.bot_id` + `sender.display_name` instead of `sender.account_id` (which was carrying the agent_id).
3. Backend callers of `ensure_host_runtime_agent` receive a richer `AgentRow` that now includes identity-layer fields; populate them from the daemon's cloud-sync uplink.

---

## Feasibility Assessment

The codebase already implements the *product* model described in ADR 0021: agents are bot participants, conversations are multi-participant, mentions are polymorphic, and the agent dispatch queue is the bot inbox. What is **not** yet implemented is the separation of *bot identity* (the digital body: runtime, model, system prompt, reasoning effort — stable across conversations and sessions) from *session context* (the per-conversation execution thread: turn history, pause/resume state, provider reattach token).

Concrete evidence this is achievable without architectural upheaval:

- Backend `agents` table (`crates/minos-backend/migrations/sqlite/0001_initial.sql:120-132`) already holds the identity seed (`runtime_agent`, `model`, `workspace_path`, `owner_account_id`) — it just lacks `system_prompt` / `reasoning_effort` / `env_json` / `display_name`.
- Backend `agent_sessions` table (`0001_initial.sql:307-318`) already links `conversation_id → agent_id → host_installation_id` — the session-context layer exists; it is *underused* because the daemon currently keys roster membership on the runtime name string, not the bot id.
- Daemon `agent_profiles` (`crates/minos-daemon/migrations/0001_initial.sql:211-222`) already models the full identity tuple (`runtime_agent`, `model`, `reasoning_effort`, `env_json`, `instructions`) — this is exactly the "digital body" the user describes. It just lives host-local with no cloud mirror, so it cannot be the cross-device identity SSOT.
- Daemon `sessions` table (`0001_initial.sql:79-99`) already stores per-session execution context (`status`, `provider_session_id`, `needs_continue`, `last_seq`) — the session-context layer is correct; it is keyed by session_id, which is the right grain.

The work is therefore **wiring**, not invention: (a) promote the daemon's `agent_profiles` identity tuple into the cloud `agents` table, (b) re-key the daemon's `conversation_agent_members` from runtime-string to bot-id, (c) make one `bot-{uuid}` own multiple per-conversation sessions, and (d) ensure session resume uses the bot identity to reconstruct launch options. Fully feasible.

---

## Current Surface Inventory

### Backend — identity layer (agents table + store)

- `crates/minos-backend/migrations/sqlite/0001_initial.sql:120-132` — `agents` table (identity seed, missing prompt/effort/env/display_name).
- `crates/minos-backend/migrations/postgres/0001_initial.sql:110-122` — Postgres parity.
- `crates/minos-backend/src/store/social/agents.rs:46-106` — `register_agent_with_source` (mints `bot-{uuid}`).
- `crates/minos-backend/src/store/social/agents.rs:108-176` — `find_host_runtime_agent` / `ensure_host_runtime_agent` (stable `(owner, runtime)` → bot-id mapping).
- `crates/minos-backend/src/store/social/agents.rs:576-751` — `insert_agent_message_with_session_in_tx` (agent reply bubble; writes `sender_agent_id`).
- `crates/minos-backend/src/store/social/agents.rs:660-661` — audit-FK hack: binds `owner_account_id` into `sender_account_id` for agent rows.

### Backend — session-context layer (agent_sessions table)

- `crates/minos-backend/migrations/sqlite/0001_initial.sql:307-318` — `agent_sessions` table (`conversation_id → agent_id → host_installation_id`).
- `crates/minos-backend/src/agent_sessions/use_case.rs:248-425` — `start` (idempotent session creation; deterministic UUID).
- `crates/minos-backend/src/agent_sessions/use_case.rs:427-547` — `send_input` (resume existing session or error).
- `crates/minos-backend/src/http/v1/social.rs:223-343` — `plan_agent_deliveries` (reply-to → mentions → sole-agent routing).
- `crates/minos-backend/src/http/v1/social.rs:1516-1599` — `try_agent_dispatch` (post-commit agent inbox enqueue).

### Backend — conversation membership

- `crates/minos-backend/migrations/sqlite/0001_initial.sql:223-229` — `conversation_agent_members` (`conversation_id, agent_id` — already bot-id keyed).
- `crates/minos-backend/src/store/social/conversations.rs:663-725` — `list_conversation_members` / `list_conversation_member_profiles`.

### Backend — wire types

- `crates/minos-backend/src/conversations/use_case.rs:1213-1226` — `agent_sender_summary` (synthesizes `UserSummary` with `account_id = agent_id` — the abuse to fix).
- `crates/minos-protocol/src/*.rs` — `ChatMessageSummary`, `UserSummary`, `SenderType`.

### Daemon — identity layer (agent_profiles, currently host-local)

- `crates/minos-daemon/migrations/0001_initial.sql:211-222` — `agent_profiles` table (the full identity tuple: runtime, model, effort, env, instructions).
- `crates/minos-daemon/src/agent.rs:3172-3253` — `create_agent_profile` / `update_agent_profile` / `delete_agent_profile` (CRUD on host-local identity).
- `crates/minos-daemon/src/agent.rs:3065-3107` — `resolve_launch_options` (reads profile to build CLI launch: model + effort + instructions → the "digital body" materialization).

### Daemon — roster (THE GAP: keyed on runtime string, not bot-id)

- `crates/minos-daemon/migrations/0001_initial.sql:66-74` — `conversation_agent_members` table, PK = `(conversation_id, agent)` where `agent` is a **runtime name string** ("codex", "claude", etc.).
- `crates/minos-daemon/src/store/mod.rs:765-912` — all roster queries (`list_conversation_roster`, `is_conversation_agent_member`, `set/add/remove_conversation_agent_member`) operate on `agent: &str` (runtime name).
- `crates/minos-daemon/src/agent.rs:3004-3018` — `start_agent_in_conversation` membership check uses runtime label.
- `crates/minos-daemon/src/agent.rs:4919-4937` — mention token format `@codex#abc12345` (runtime + session-short, not bot-id).

### Daemon — session-context layer

- `crates/minos-daemon/migrations/0001_initial.sql:79-99` — `sessions` table (per-session execution context; keyed by session_id — correct grain).
- `crates/minos-daemon/src/agent.rs:519-638` — `start_agent_with_session_id_in_conversation` (Hub collab path: fixed session_id, conversation binding).
- `crates/minos-daemon/src/agent.rs:2982-3154` — `start_agent_in_conversation` (local/Desktop path).
- `crates/minos-daemon/src/agent.rs:1085-1191` — `resume_session` (reattach + needs_continue).
- `crates/minos-daemon/src/conversation_completion.rs:624-727` — turn completion → agent reply bubble (writes `agent = runtime_name` in local messages).

### Clients

- `apps/desktop/src/features/chat/MessageRow.tsx:80-167` — renders agent messages; reads `message.agent` (runtime key) and `message.sessionId`.
- `apps/mobile/lib/ui/features/social/widgets/conversation_message_row.dart:44-92` — same pattern; `senderType == SenderType.agent` + session short.
- `apps/web/src/lib/store.ts` — no IM timeline; mock only.

---

## Design

### Core model: two-layer separation

```
┌─────────────────────────────────────────────────────┐
│  IDENTITY LAYER (global, cloud-owned SSOT)          │
│                                                     │
│  Bot {                                             │
│    bot_id: "bot-{uuid}"          // stable forever │
│    owner_account_id              // who owns it    │
│    display_name                  // "Codex Pro"    │
│    runtime_agent                 // codex/claude/… │
│    model                         // gpt-5 / …      │
│    reasoning_effort              // high/medium/…  │
│    system_prompt                 // digital body   │
│    env_json                      // API keys etc.  │
│    workspace_path                // default cwd    │
│  }                                                 │
│                                                     │
│  Lives in: backend `agents` table                  │
│  Mirrored to: daemon `agent_profiles` (cache)      │
└─────────────────────────────────────────────────────┘
          │ owns
          ▼
┌─────────────────────────────────────────────────────┐
│  SESSION-CONTEXT LAYER (per-conversation)           │
│                                                     │
│  Session {                                         │
│    session_id: "{uuid}"          // per-conversation│
│    conversation_id               // which room     │
│    bot_id  ──────────────────► Bot identity        │
│    host_installation_id          // which Mac runs │
│    status                        // running/suspended│
│    provider_session_id           // reattach token │
│    turn_state, needs_continue    // resume context │
│  }                                                 │
│                                                     │
│  Lives in:                                          │
│    - backend `agent_sessions` table (cloud)        │
│    - daemon `sessions` table (local execution)     │
│                                                     │
│  One bot → N sessions (one per conversation)       │
│  One conversation → N bot sessions (multi-agent)   │
└─────────────────────────────────────────────────────┘
```

### Key design decisions

1. **Bot identity is cloud-owned, daemon-cached.**
   - *Choice*: The `agents` table in the backend is the identity SSOT. The daemon's `agent_profiles` table becomes a read-through cache of cloud-owned bot identities.
   - *Rejected*: Keep identity host-local (current). Reason: multi-device requires the same bot identity to be visible from any client (Desktop, Mobile, Web). If identity lives only on the daemon, Mobile/Web cannot show bot profiles, cannot @-mention by display name, and cannot resolve bot availability.
   - *Rejected*: Give bots their own `account_id` / login. Reason: ADR 0021 explicitly forbids this; bots have no human auth. The `bot-{uuid}` id is the stable identity, not an account.

2. **`conversation_agent_members` is keyed on `bot_id`, everywhere.**
   - *Choice*: Both backend and daemon `conversation_agent_members` tables use `(conversation_id, bot_id)` as PK. The daemon's current runtime-string key is replaced.
   - *Rejected*: Keep daemon keyed on runtime name. Reason: the user explicitly states "conversation 只持有 membership" and identity is global. Keying on runtime means two profiles of the same runtime (e.g., two codex bots with different system prompts) cannot coexist as separate roster members — directly contradicting "bot 身份全局唯一".
   - *Consequence*: The daemon's local `conversation_agent_members` schema, store queries, roster logic, and mention resolution all change from `agent: &str` to `bot_id: &str`.

3. **One bot owns N per-conversation sessions; sessions are never the identity.**
   - *Choice*: `agent_sessions.bot_id` (already a FK in the backend) is the link. Starting a bot in conversation C creates/reuses a session row keyed `(conversation_id, bot_id)`. If the session is suspended/closed, a new session_id may be minted, but the bot_id stays the same — the bot is the same participant.
   - *Rejected*: Session-scoped identity (current daemon model where `chat_messages.agent = runtime_name` + `session_id`). Reason: if a session closes and a new one starts, the timeline shows a "different" agent. The user wants the bot to be the persistent participant; sessions are ephemeral execution contexts.
   - *Consequence*: `chat_messages.sender_agent_id` (backend) already carries `bot_id`. The daemon's local `chat_messages.agent` column must change from runtime-name to `bot_id` (or the daemon resolves bot_id → display_name at render time from the cached identity).

4. **Identity-layer fields move from daemon `agent_profiles` to cloud `agents`.**
   - *Choice*: Add `system_prompt`, `reasoning_effort`, `env_json`, `display_name` to the backend `agents` table. The daemon's `agent_profiles` table is retained as a local cache (for offline launch), but the cloud table is the SSOT. When the daemon starts a session, it resolves launch options from the cached identity row; if the cache is stale, it syncs from cloud first.
   - *Rejected*: Merge `agent_profiles` into `agents` on the daemon only. Reason: Mobile/Web need to *display and edit* bot identities (system prompt, model) without talking to the daemon. Identity must be cloud-editable.
   - *Rejected*: Keep identity split (cloud has runtime+model, daemon has prompt+effort). Reason: the user explicitly calls this the "数字肉身" — it is one cohesive identity, not split across two stores.

5. **Mention tokens use display_name, resolved to bot_id at write time.**
   - *Choice*: `@Codex Pro` or `@codex-pro` in message body resolves to `bot_id` via conversation roster lookup. The structured mention carries `target_kind=agent, target_id=bot_id`. No `#session_short` in the canonical mention — sessions are execution detail, not identity.
   - *Rejected*: `@codex#abc12345` (current). Reason: this mentions a *session*, not a *bot*. If the session rotates, the mention is stale. The user wants the bot to be the addressable entity.
   - *Session hint*: `#session_short` may still appear in the UI as a visual indicator of which execution context produced a message, but it is not part of the mention routing key.

6. **Session resume reconstructs launch options from bot identity.**
   - *Choice*: `resume_session` on the daemon reads the bot identity row (runtime, model, effort, system_prompt, env) to rebuild CLI launch options, then reattaches via `provider_session_id`. The identity *is* the launch spec; the session *is* the execution state.
   - *Current gap*: `resolve_launch_options` (`agent.rs:3065-3107`) reads the host-local `agent_profiles` table. After this change, it reads from the bot-identity cache (which is synced from cloud).

7. **Wire sender model: `BotSender` discriminated, no more `account_id` abuse.**
   - *Choice*: `ChatMessageSummary.sender` becomes a tagged union:
     ```rust
     pub enum MessageSender {
         Account { account_id: String, display_name: String, avatar_url: Option<String> },
         Bot { bot_id: String, display_name: String, runtime_agent: String, avatar_emoji: Option<String> },
     }
     ```
   - *Rejected*: Keep `UserSummary.account_id = agent_id` (current). Reason: this is a type lie — `account_id` is a human auth principal, not a bot id. Clients that join on `account_id` get phantom matches. The user's framing ("可以视为一个用户...只是他是一个特殊的 bot") maps to a *display-level* equivalence, not a *schema-level* one.

### Bot lifecycle (target)

```
CREATE BOT
  → backend registers agent row (bot-{uuid}, owner, runtime, model, prompt, …)
  → daemon syncs identity to local cache (agent_profiles or new bot_identities table)

ADD BOT TO CONVERSATION
  → backend inserts conversation_agent_members (conversation_id, bot_id)
  → daemon syncs roster (conversation_agent_members keyed on bot_id)

@BOT IN CONVERSATION
  → user types "@Codex Pro fix the bug"
  → backend resolves display_name → bot_id via roster
  → chat_message_mentions (target_kind=agent, target_id=bot_id)
  → agent inbox enqueue (origin_message_id, bot_id)
  → worker: find-or-create session (conversation_id, bot_id) on bound host
  → daemon: start/resume CLI session using bot identity as launch spec
  → CLI output → turn completion → agent reply bubble (sender=Bot{bot_id, …})

BOT SESSION SUSPEND/RESUME
  → daemon suspends session (daemon stop / user interrupt)
  → session row: status=suspended, provider_session_id preserved
  → resume: re-read bot identity, rebuild launch, reattach provider session
  → bot_id unchanged; session_id may be reused (same provider session) or new

REMOVE BOT FROM CONVERSATION
  → backend deletes conversation_agent_members row
  → daemon syncs roster deletion
  → active session for (conversation, bot) is closed
  → bot identity itself is unaffected (can be re-added to same or other conversations)
```

---

## Phased Implementation

### Phase 1: Backend identity-layer enrichment — **DONE (core)**

Shipped in latest-only `0001_initial.sql` agents digital-body columns + `AgentSummary` / register-update APIs (see global-bot-identity-design Phase 1).

**File: `crates/minos-backend/migrations/sqlite/0001_initial.sql`** (lines 120-132)
- Add columns to `agents` table:
  ```sql
  display_name     TEXT NOT NULL DEFAULT '',
  system_prompt    TEXT NOT NULL DEFAULT '',
  reasoning_effort TEXT NOT NULL DEFAULT '',
  env_json         TEXT NOT NULL DEFAULT '[]',
  ```
- The `name` column remains as the internal handle; `display_name` is the user-facing label (defaults to `name`).

**File: `crates/minos-backend/migrations/postgres/0001_initial.sql`** (lines 110-122)
- Mirror the same column additions for Postgres parity.

**File: `crates/minos-backend/src/store/social/agents.rs`**
- Update `AGENT_SELECT_COLS` to include the new columns.
- Update `AgentRow` struct to carry the new fields.
- Update `register_agent_with_source` to accept and bind the new fields.
- Add `update_bot_identity(store, bot_id, display_name, system_prompt, reasoning_effort, env_json, model)` — the cloud-editable identity mutation API.
- Update `ensure_host_runtime_agent` to accept the full identity tuple (called when daemon uplinks its local profiles).

**File: `crates/minos-backend/src/http/v1/agents.rs`** (or equivalent)
- Add `PATCH /v1/agents/{bot_id}` — update bot identity (display_name, system_prompt, model, reasoning_effort, env_json).
- Add `GET /v1/agents` — list bots owned by the authenticated account.
- These endpoints let Mobile/Web/Desktop edit bot identities without talking to the daemon.

**File: `crates/minos-backend/src/http/v1/conversations.rs`**
- `POST /v1/conversations/{id}/agents/add` now accepts `bot_id` (not just `runtime_agent`). If the bot doesn't exist, returns 404 (no implicit creation).
- Add `POST /v1/agents/ensure-host-runtime` to accept the full identity tuple from the daemon uplink (called when daemon syncs its local profiles to cloud).

Rationale: the identity layer must be cloud-editable before clients can manage bots independently of the daemon.

---

### Phase 2: Daemon roster re-keyed to bot_id — **DONE (core)**

Daemon local roster is bot_id-keyed: `conversation_agent_members(conversation_id, bot_id)`, `bot_identities` replaces `agent_profiles`, `chat_messages.bot_id`, `sessions.bot_id` (nullable). Offline create still accepts runtime labels and maps them via `ensure_local_runtime_bot` → `local-rt-{runtime}` (`source=host_runtime_seed`). Wire AgentProfile* RPCs map `id` ↔ `bot_id`.

**File: `crates/minos-daemon/migrations/0001_initial.sql`** (lines 66-74)
- Replace `conversation_agent_members` schema:
  ```sql
  CREATE TABLE conversation_agent_members (
      conversation_id  TEXT NOT NULL REFERENCES conversations(conversation_id) ON DELETE CASCADE,
      bot_id           TEXT NOT NULL,
      joined_at_ms     INTEGER NOT NULL,
      brief            TEXT NOT NULL DEFAULT '',
      PRIMARY KEY (conversation_id, bot_id),
      CHECK(length(bot_id) > 0),
      CHECK(length(brief) <= 500)
  );
  ```
- The `agent` column (runtime name) is removed. Runtime is resolved from the bot identity cache at query time.

**File: `crates/minos-daemon/migrations/0001_initial.sql`** (lines 211-222)
- Rename/evolve `agent_profiles` into a bot-identity cache keyed on `bot_id`:
  ```sql
  CREATE TABLE bot_identities (
      bot_id            TEXT PRIMARY KEY NOT NULL,
      owner_account_id  TEXT NOT NULL,
      display_name      TEXT NOT NULL,
      runtime_agent     TEXT NOT NULL,
      model             TEXT NOT NULL,
      reasoning_effort  TEXT NOT NULL DEFAULT '',
      system_prompt     TEXT NOT NULL DEFAULT '',
      env_json          TEXT NOT NULL DEFAULT '[]',
      workspace_path    TEXT,
      source            TEXT NOT NULL DEFAULT 'host_runtime',
      synced_at_ms      INTEGER NOT NULL,
      created_at_ms     INTEGER NOT NULL,
      updated_at_ms     INTEGER NOT NULL
  );
  ```
- This is the daemon-side cache of the cloud `agents` table. It replaces `agent_profiles` (which had no `bot_id` and no cloud sync).

**File: `crates/minos-daemon/migrations/0001_initial.sql`** (lines 117-136)
- `chat_messages.agent` column renamed to `chat_messages.bot_id` (TEXT, FK to `bot_identities.bot_id`).
- The CHECK constraint changes:
  ```sql
  CHECK(
      (sender_role = 'user' AND session_id IS NULL AND bot_id IS NULL)
      OR
      (sender_role = 'agent' AND session_id IS NOT NULL AND bot_id IS NOT NULL)
  )
  ```

**File: `crates/minos-daemon/src/store/mod.rs`** (lines 755-912)
- All roster queries change from `agent: &str` to `bot_id: &str`:
  - `list_conversation_roster` → returns `bot_id` instead of runtime name.
  - `is_conversation_agent_member(conversation_id, bot_id)`.
  - `set/add/remove_conversation_agent_member` → take `bot_id`.
- Add `bot_identities` CRUD: `upsert_bot_identity`, `get_bot_identity`, `list_bot_identities`, `delete_bot_identity`.
- `ConversationAgentMemberRow.agent` field renamed to `bot_id`.

**File: `crates/minos-daemon/src/agent.rs`**
- `start_agent_in_conversation` (line 2982): membership check uses `bot_id` instead of `agent_label(req.agent)`. Runtime is resolved from the bot identity row.
- `resolve_launch_options` (line 3065): reads from `bot_identities` cache (keyed by `bot_id`) instead of `agent_profiles` (keyed by profile id). The bot identity *is* the launch spec.
- `start_agent_with_session_id_in_conversation` (line 519): accepts `bot_id`, resolves runtime from identity, passes identity-derived launch options to the manager.
- `conversation_completion.rs` (line 675): agent reply bubble writes `bot_id` instead of runtime name into `chat_messages.bot_id`.
- Mention token resolution (`agent.rs:4919-4937`): `@DisplayName` resolves to `bot_id` via roster. The `#session_short` suffix is removed from routing; it may remain as a UI-only visual hint.

**File: `crates/minos-daemon/src/relay_client.rs`** and **`crates/minos-daemon/src/rpc_server.rs`**
- Host command params for `agent_session.start` / `agent_session.send_input` carry `bot_id` (already present in backend command shape). The daemon resolves `bot_id → runtime + launch` from its identity cache.
- Add a cloud-sync path: on relay connect and periodically, the daemon pulls bot identities for its owner account from the backend and upserts into `bot_identities`. Conversely, if the daemon creates a local profile (Desktop UI), it uplinks to cloud.

**File: `crates/minos-daemon/src/roster.rs`**
- `format_roster_briefing` and `enrich_roster_with_profile_briefs` operate on `bot_id` + `display_name` from the identity cache, not runtime names.

Rationale: this is the largest phase because the daemon's entire roster + mention + launch pipeline is re-keyed. It must land as one atomic schema + code change (latest-only policy, no incremental ALTER).

---

### Phase 3: Backend agent dispatch + session uses bot_id end-to-end — **DONE (core)**

**File: `crates/minos-backend/src/http/v1/social.rs`** (lines 223-343)
- `plan_agent_deliveries`: all agent references use `bot_id`. Reply-to-agent resolution extracts `bot_id` from the replied message's `sender_agent_id`. Mention resolution maps `target_id` (already `bot_id` in the polymorphic mentions table) directly.
- `forward_agent_dispatch` / `execute_claimed_dispatch`: host command params carry `bot_id`. The daemon resolves runtime from its identity cache.

**File: `crates/minos-backend/src/agent_sessions/use_case.rs`** (lines 248-547)
- `start`: idempotency key includes `bot_id`. Session creation binds `(conversation_id, bot_id, host_installation_id)`.
- `send_input`: finds the active session for `(conversation_id, bot_id)` and resumes. If no active session, calls `start`.
- Session lifecycle: when a session ends (status=ended/failed), the next `@bot` in the same conversation creates a new `session_id` but the same `bot_id`. The timeline shows continuity because `sender_agent_id = bot_id` is stable.

**File: `crates/minos-backend/src/conversations/use_case.rs`** (lines 1213-1226)
- Replace `agent_sender_summary` (which synthesizes `UserSummary { account_id = agent_id }`) with a proper `BotSender` construction:
  ```rust
  fn bot_sender_summary(agent: &AgentRow) -> MessageSender {
      MessageSender::Bot {
          bot_id: agent.agent_id.clone(),
          display_name: if agent.display_name.is_empty() {
              agent.name.clone()
          } else {
              agent.display_name.clone()
          },
          runtime_agent: agent.runtime_agent.clone(),
          avatar_emoji: Some("🤖".to_string()),
      }
  }
  ```

**File: `crates/minos-backend/src/store/social/agents.rs`** (line 660)
- Stop binding `owner_account_id` into `sender_account_id` for agent rows. Make `sender_account_id` nullable for agent messages (or drop the NOT NULL and use `sender_agent_id` as the authoritative sender for `sender_type=agent`).

**File: `crates/minos-backend/migrations/sqlite/0001_initial.sql`** (line 237)
- `sender_account_id TEXT NOT NULL` → `sender_account_id TEXT` (nullable). For `sender_type=agent`, `sender_account_id` is NULL and `sender_agent_id` is authoritative.

Rationale: the backend must stop treating agent messages as "owned by the human account" at the row level. The bot is the sender; the owner is metadata on the `agents` row, not on every message.

---

### Phase 4: Wire protocol — `MessageSender` discriminated union — **DONE**

**File: `crates/minos-protocol/src/messages.rs`** (ChatMessageSummary.sender) and clients
- Introduced `MessageSender` enum on HTTP/WS chat summaries:
  ```rust
  #[derive(Debug, Clone, Serialize, Deserialize)]
  #[serde(tag = "kind", rename_all = "snake_case")]
  pub enum MessageSender {
      Account {
          account_id: String,
          display_name: String,
          avatar_url: Option<String>,
      },
      Bot {
          bot_id: String,
          display_name: String,
          runtime_agent: String,
          avatar_emoji: Option<String>,
      },
  }
  ```
- `ChatMessageSummary.sender` changes from `UserSummary` to `MessageSender`.
- `SenderType` enum is retained for backward filtering but derived from the `MessageSender::variant`.

**File: `crates/minos-backend/src/conversations/use_case.rs`**
- All message-to-wire serialization uses `MessageSender`.
- Human messages: `MessageSender::Account { account_id, display_name, avatar_url }`.
- Agent messages: `MessageSender::Bot { bot_id, display_name, runtime_agent, avatar_emoji }`.

Rationale: this is the wire break. Clients can no longer accidentally treat a bot sender as an account. The `bot_id` is first-class on the wire.

---

### Phase 5: Client rendering — bot as a first-class participant everywhere — **DONE (core)**

**File: `apps/desktop/src/features/chat/MessageRow.tsx`** (lines 80-167)
- Read `sender.kind === "bot"` instead of `message.role === "agent"`.
- Render bot avatar from `sender.display_name` + `sender.avatar_emoji` (not from runtime-keyed `KnownAgent` map).
- Deep-link to session transcript via `message.sessionId` (unchanged — session is execution detail).

**File: `apps/desktop/src/features/chat/Composer.tsx`** (or equivalent @-picker)
- @-picker lists participants from `GET /v1/conversations/{id}/participants` → `{ humans, bots }`.
- Bot entries show `display_name` + `runtime_agent` badge.
- Inserting a bot mention puts `@DisplayName` in the body and structures `target_id = bot_id`.

**File: `apps/mobile/lib/ui/features/social/widgets/conversation_message_row.dart`** (lines 44-92)
- Same change as Desktop: read `sender.kind === SenderKind.bot`, render `sender.displayName` + bot avatar.
- `_MessageAvatar` uses `sender.avatarEmoji` instead of hardcoded gear icon (configurable per-bot identity).

**File: `apps/mobile/lib/application/social_providers.dart`**
- Participant list provider fetches unified `participants` endpoint.
- @-picker (if exists) shows humans + bots.

**File: `apps/web/src/lib/store.ts`**
- If Web gains an IM timeline in this program: use `MessageSender` from the start. No legacy `account_id = agent_id` path.

Rationale: clients must render the bot as a stable participant (by `bot_id` + `display_name`), not as a runtime-name + session-short composite. This is the user-visible "Discord bot" experience.

---

### Phase 6: Multi-device seamless continuation

**File: `apps/mobile/lib/infrastructure/social_cache_store.dart`**
- Add a `topic_cursors` table to the mobile SQLite cache:
  ```sql
  CREATE TABLE topic_cursors (
      topic       TEXT PRIMARY KEY,
      last_seq    INTEGER NOT NULL,
      updated_at  INTEGER NOT NULL
  );
  ```
- Persist cursor on every `update_seq` call in `crates/minos-mobile/src/realtime/subscription.rs:109-115`.
- On cold start, load cursors from SQLite and send `resume_after` in `Subscribe`.

**File: `crates/minos-mobile/src/realtime/session.rs`** (lines 36-73)
- `resume_after_map()` reads from the persisted cursor store, not from in-process `SubscriptionManager.cursors`.
- On `SnapshotRequired`, clear the persisted cursor for that topic and trigger full REST hydrate.

**File: `apps/desktop/src/shared/lib/hub-cursors.ts`**
- Already persists to localStorage. No change needed for cursor persistence. But:
- Add `draft_by_conversation` to the persisted store (currently in-memory `ui-store.ts`). Drafts survive page reload but not device switch (draft sync is a future enhancement).

**File: `apps/desktop/src/store/workspace/conversation-list.ts`**
- Migrate conversation list source from daemon-primary to Hub-primary (`architecture-messaging.md:763` gap).
- Daemon rows are merged as enrichment (git status, worktree), not as the list SSOT.
- This ensures Desktop, Mobile, and Web all see the same conversation list from the Hub.

**File: `apps/web/src/lib/relay-socket.ts`**
- Add cursor tracking + `resume_after` on reconnect (port Desktop's `hub-realtime.ts` pattern).
- Add client-side outbox (port Desktop's `im-outbox.ts` pattern, using IndexedDB instead of Tauri SQLite).
- Without these, Web cannot be a real IM client.

**File: `apps/web/src/lib/store.ts`**
- Add `messagesByConversation`, `outbox`, `topicCursors` to the Zustand store.
- This is a substantial build; it may be deferred if Web IM is not in the immediate scope.

Rationale: the user wants "切换设备丝滑继续对话". The critical path is: (a) Mobile cold-start cursor persistence (so app reopen resumes instead of full reload), (b) Desktop conversation list Hub-SSOT (so all devices agree on the list), (c) Web IM engine (so Web is a real third client). Draft syncing across devices is a future enhancement (requires a `drafts` table on the backend).

---

### Phase 7: Cloud-sync daemon bot identities

**File: `crates/minos-daemon/src/relay_http.rs`**
- Add `GET /v1/agents` (account-scoped) to pull all bot identities for the linked account.
- Add `PUT /v1/agents/{bot_id}` to upsert a bot identity from the daemon (when Desktop creates/edits a profile locally).

**File: `crates/minos-daemon/src/relay_client.rs`**
- On `Hello` (relay connect), trigger a bot-identity sync: pull cloud identities → upsert into `bot_identities` cache.
- If the daemon has local identities not yet in cloud (legacy `agent_profiles` rows), uplink them with `source = host_runtime`.

**File: `crates/minos-daemon/src/agent.rs`** (lines 3172-3253)
- `create_agent_profile` / `update_agent_profile` now write to `bot_identities` *and* uplink to cloud.
- The cloud `agents` table is the SSOT; the daemon cache is eventually consistent.

Rationale: bot identities must be editable from any client (Desktop, Mobile, Web) and visible on all devices. The daemon is one editor among many; the cloud is the arbiter.

---

### Phase 8: Verification

**File: `crates/minos-backend/tests/`**
- Test: bot identity CRUD (create, update display_name/system_prompt, delete).
- Test: one bot added to two conversations → two independent sessions, same `bot_id`.
- Test: agent message `sender` is `MessageSender::Bot` with correct `bot_id`.
- Test: `sender_account_id` is NULL for agent messages.
- Test: mention `@DisplayName` resolves to `bot_id` via roster.

**File: `crates/minos-daemon/src/agent.rs` (test module)**
- Test: roster keyed on `bot_id`; two bots with same runtime but different `bot_id` coexist.
- Test: `resolve_launch_options` reads from `bot_identities` cache.
- Test: session suspend/resume preserves `bot_id`, rebuilds launch from identity.
- Test: mention token `@DisplayName` resolves to `bot_id`.

**File: `apps/desktop/src/features/chat/` (tests)**
- Test: `MessageRow` renders bot sender correctly from `MessageSender::Bot`.
- Test: @-picker shows bots from unified participants API.

**File: `apps/mobile/test/`**
- Test: `conversation_message_row.dart` renders bot sender from `SenderKind.bot`.
- Test: topic cursors persist across cold start (simulated process kill).

---

## Architectural Notes

- **Semver impact**: Breaking. The wire `ChatMessageSummary.sender` type changes. The daemon local schema is wiped-and-rebuilt (latest-only policy). Backend schema gains columns (additive) but `sender_account_id` becomes nullable (breaking for any query that assumes NOT NULL).
- **ADR 0021 alignment**: This spec *implements* ADR 0021's vision fully. ADR 0021 says "agent is a first-class participant (bot), not a human Account." The current code half-implements this (bot table exists but daemon rosters by runtime name; wire sender abuses account_id). This spec completes the implementation.
- **No dual-write / no compatibility layer**: Per AGENTS.md Development-State Compatibility Policy, the daemon schema is rebuilt fresh (no ALTER migration). The wire type changes in one break. Clients update atomically with the backend.
- **Object identity invariant**: A bot's `bot_id` is immutable and globally unique. A bot can be added to N conversations, removed, re-added. Its identity (model, prompt, effort) is editable but its `bot_id` never changes. Sessions come and go; the bot persists.
- **Session isolation invariant**: Sessions for `(conversation_id, bot_id)` are independent across conversations. Bot in conversation A does not share turn history with bot in conversation B. This is the "每个 conversation 为该 bot 维护独立 session 作为执行上下文" requirement.
- **Multi-device continuation**: A user on Desktop sends a message @-mentioning a bot. The message lands on the Hub. Mobile (same account, different device) receives it via `conversation:{id}` durable fanout. The bot replies on the Hub. Both devices see the reply. The user switches to Mobile and continues typing — the conversation state (cursor, timeline) is synced via the Hub durable log + per-device cursor resume.
- **What is explicitly NOT changed**:
  - The dual-principal auth model (Account + Host) from ADR 0020.
  - The `agent_dispatch_queue` (Agent inbox) physical table and worker.
  - The durable event log / outbox / topic_metadata sequence authority.
  - The `/ws/host` relay connection model (daemon authenticates as host installation, not as user).
  - The per-conversation message_seq allocation.
- **New cross-crate dependency**: `minos-protocol` gains `MessageSender` enum, consumed by backend + all clients. No new external dependencies.

---

## File Change Summary

- `crates/minos-backend/migrations/sqlite/0001_initial.sql` -- add identity columns to `agents`; make `chat_messages.sender_account_id` nullable.
- `crates/minos-backend/migrations/postgres/0001_initial.sql` -- Postgres parity for identity columns + nullable sender_account_id.
- `crates/minos-backend/src/store/social/agents.rs` -- AgentRow + register/ensure/update identity; stop binding owner into sender_account_id for agent messages.
- `crates/minos-backend/src/store/social/conversation_messages.rs` -- agent message insert no longer sets sender_account_id for agent rows.
- `crates/minos-backend/src/conversations/use_case.rs` -- replace agent_sender_summary with BotSender construction; MessageSender wire type.
- `crates/minos-backend/src/agent_sessions/use_case.rs` -- session lifecycle keyed on (conversation_id, bot_id); resume/create logic.
- `crates/minos-backend/src/http/v1/social.rs` -- plan_agent_deliveries uses bot_id; dispatch params carry bot_id.
- `crates/minos-backend/src/http/v1/conversations.rs` -- agents/add accepts bot_id; participants API returns unified humans+bots.
- `crates/minos-backend/src/http/v1/agents.rs` -- new: PATCH /v1/agents/{bot_id}, GET /v1/agents for identity CRUD.
- `crates/minos-protocol/src/lib.rs` -- new MessageSender enum (Account | Bot); ChatMessageSummary.sender type change.
- `crates/minos-protocol/src/realtime.rs` -- MessageSender in durable event payloads.
- `crates/minos-daemon/migrations/0001_initial.sql` -- conversation_agent_members keyed on bot_id; bot_identities table replaces agent_profiles; chat_messages.agent → bot_id.
- `crates/minos-daemon/src/store/mod.rs` -- all roster queries re-keyed to bot_id; bot_identities CRUD.
- `crates/minos-daemon/src/agent.rs` -- resolve_launch_options reads bot_identities; start/resume keyed on bot_id; mention resolution by display_name→bot_id.
- `crates/minos-daemon/src/conversation_completion.rs` -- agent reply writes bot_id into chat_messages.
- `crates/minos-daemon/src/roster.rs` -- briefing/enrich operates on bot_id + display_name.
- `crates/minos-daemon/src/relay_http.rs` -- bot identity cloud sync (pull + uplink).
- `crates/minos-daemon/src/relay_client.rs` -- identity sync on connect; host command params use bot_id.
- `crates/minos-daemon/src/rpc_server.rs` -- dispatch uses bot_id, resolves runtime from identity cache.
- `crates/minos-daemon/src/local_rpc.rs` -- profile CRUD RPCs write to bot_identities + uplink.
- `apps/desktop/src/features/chat/MessageRow.tsx` -- read MessageSender.Bot; render display_name + emoji.
- `apps/desktop/src/features/chat/Composer.tsx` -- @-picker from unified participants.
- `apps/desktop/src/store/workspace/conversation-list.ts` -- Hub-primary list (close daemon-primary gap).
- `apps/mobile/lib/ui/features/social/widgets/conversation_message_row.dart` -- SenderKind.bot rendering.
- `apps/mobile/lib/application/social_providers.dart` -- unified participants provider.
- `apps/mobile/lib/infrastructure/social_cache_store.dart` -- topic_cursors table for cold-start resume.
- `apps/mobile/crates/minos-mobile/src/realtime/session.rs` -- resume_after from persisted cursors.
- `apps/mobile/crates/minos-mobile/src/realtime/subscription.rs` -- persist cursor on update_seq.
- `apps/web/src/lib/relay-socket.ts` -- cursor tracking + resume_after.
- `apps/web/src/lib/store.ts` -- messagesByConversation, outbox, topicCursors (IM engine build-out).
- `docs/adr/0022-bot-identity-session-separation.md` -- new ADR documenting the two-layer model.
- `docs/architecture-messaging.md` -- update bot identity + session model sections.
- `docs/architecture-daemon.md` -- update roster + identity + session sections.
