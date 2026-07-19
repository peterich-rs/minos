# Agent Mention 会话作用域修复 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Fix `@Agent#hashid` candidates leaking across conversations — when inside a conversation, the mention picker should only show agent sessions belonging to that conversation.

**Implementation status (2026-06-24):** implemented in code. `ThreadSummary` / `ThreadSummaryEntry` now carry state, daemon conversation-session listing uses live-manager-state first with DB-row fallback, TUI mention candidates and short-id routing only expose existing sessions when a conversation is active, manager events keep `conversation_agent_sessions` state fresh, and targeted tests cover conversation-scoped behavior plus hiding existing sessions from the new-conversation input. Commit steps below are historical plan instructions, not completed by this document.

**Architecture:** `room_agent_mention_candidates()` reads from `self.conversation_agent_sessions` (per-conversation) only when `nav_level().conversation_id()` is `Some`; outside an active conversation it only returns installed agents. The short-id resolver `thread_id_for_agent_short_id()` uses the same scoping and does not fall back to `self.threads`. `ThreadSummaryEntry` carries a `state` field to filter out closed sessions.

**Tech Stack:** Rust, ratatui, jsonrpsee

---

## File Structure

| File | Responsibility | Change |
|------|---------------|--------|
| `crates/minos-tui/src/backend/mod.rs` | `ThreadSummaryEntry` struct | Add `state: ThreadState` field |
| `crates/minos-protocol/src/messages.rs` | `ThreadSummary` protocol struct | Add `state: ThreadState` field (already has `end_reason`) |
| `crates/minos-tui/src/ui/mod.rs` | `room_agent_mention_candidates()` | Scope to conversation when inside one |
| `crates/minos-tui/src/app/submission.rs` | `thread_id_for_agent_short_id()` | Scope to conversation when inside one |
| `crates/minos-tui/src/app_tests/` | Test file | New tests for scoping |
| `crates/minos-daemon/src/agent.rs` | `list_conversation_agent_sessions` impl | Include `state` in response, using **live-manager-state-first, DB-row-fallback** (same strategy as `get_thread`) |

---

## Task 1: Add `state` field to `ThreadSummaryEntry`

**Files:**
- Modify: `crates/minos-tui/src/backend/mod.rs:69-94`
- Modify: `crates/minos-tui/src/backend/daemon.rs:545-562`
- Modify: `crates/minos-tui/src/backend/embedded.rs:263-274`
- Modify: `crates/minos-tui/src/app_tests.rs` (test fake backend)
- Modify: `crates/minos-tui/src/update/mod.rs:182-247` (ConversationAgentStarted handler)
- Modify: `crates/minos-tui/src/ui/mod.rs:313-351` (subagent_tests fixture)

- [ ] **Step 1: Add `state` field to `ThreadSummaryEntry` struct**

File: `crates/minos-tui/src/backend/mod.rs`

Add `state: minos_agent_runtime::ThreadState` field and update `from_summary`:

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThreadSummaryEntry {
    pub thread_id: String,
    pub agent: AgentName,
    pub title: Option<String>,
    pub first_ts_ms: i64,
    pub last_ts_ms: i64,
    pub message_count: u32,
    pub ended_at_ms: Option<i64>,
    pub parent_thread_id: Option<String>,
    pub state: minos_agent_runtime::ThreadState,
}

impl ThreadSummaryEntry {
    pub fn from_summary(s: &minos_protocol::ThreadSummary) -> Self {
        Self {
            thread_id: s.thread_id.clone(),
            agent: s.agent,
            title: s.title.clone(),
            first_ts_ms: s.first_ts_ms,
            last_ts_ms: s.last_ts_ms,
            message_count: s.message_count,
            ended_at_ms: s.ended_at_ms,
            parent_thread_id: s.parent_thread_id.clone(),
            state: s.state.clone(),
        }
    }
}
```

Note: This requires `ThreadState` to derive `Clone, PartialEq, Eq`. Check `crates/minos-agent-runtime/src/state_machine.rs` — if it doesn't derive these, add them.

- [ ] **Step 2: Add `state` field to protocol `ThreadSummary`**

File: `crates/minos-protocol/src/messages.rs:739-750`

```rust
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct ThreadSummary {
    pub thread_id: String,
    pub agent: AgentName,
    pub title: Option<String>,
    pub first_ts_ms: i64,
    pub last_ts_ms: i64,
    pub message_count: u32,
    pub ended_at_ms: Option<i64>,
    pub end_reason: Option<ThreadEndReason>,
    pub parent_thread_id: Option<String>,
    pub state: ThreadState,
}
```

- [ ] **Step 3: Update daemon `list_conversation_agent_sessions` to populate `state` (live-manager-first)**

File: `crates/minos-daemon/src/agent.rs:1094-1107`

The current implementation (`agent.rs:1094`) only reads DB rows via `thread_summary_from_row` and never consults the live `AgentManager`. This means a thread that is `Running` in-memory but still `Suspended` in the DB would appear stale, and mention filtering by `Closed`/`Open` would be wrong.

Adopt the **same strategy `get_thread` already uses** (`agent.rs:763-782`): live manager snapshot first, DB row as fallback. Concretely:

```rust
pub async fn list_conversation_agent_sessions(
    &self,
    req: minos_protocol::ListConversationAgentSessionsParams,
) -> Result<minos_protocol::ListConversationAgentSessionsResponse, MinosError> {
    let rows = self
        .store
        .list_threads_by_conversation(&req.conversation_id)
        .await
        .map_err(|e| map_store_error("list_conversation_agent_sessions", e))?;

    // Build a live-state lookup from the in-memory manager, exactly like
    // `get_thread` (agent.rs:770-776) does for a single thread.
    let live_states: std::collections::HashMap<String, ProtoThreadState> = self
        .manager
        .list_threads()
        .await
        .into_iter()
        .map(|snapshot| (snapshot.thread_id.clone(), state_to_proto(&snapshot.state)))
        .collect();

    let threads = rows
        .into_iter()
        .map(|row| {
            let mut summary = thread_summary_from_row(row.clone())?;
            // Live state wins; fall back to the DB row's state, propagating
            // any conversion error exactly like `get_thread` does.
            let row_state = row_state_to_proto(&row)?;
            summary.state = live_states
                .get(&summary.thread_id)
                .cloned()
                .unwrap_or(row_state);
            Ok(summary)
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(minos_protocol::ListConversationAgentSessionsResponse { threads })
}
```

**Requirements:**
- `thread_summary_from_row` must populate `state` from `row_state_to_proto(&row)` as the baseline (see `agent.rs:1399` for the existing helper).
- After building each `ThreadSummary` from the row, override `state` with the live manager snapshot if present (same precedence as `get_thread` at `agent.rs:780`: `live_state.unwrap_or(row_state_to_proto(&row)?)`).
- `row_state_to_proto` errors are **propagated** via `?` — identical to `get_thread`'s fallback path. Do not use `unwrap_or_default()` or any silent fallback: `ProtoThreadState` does not implement `Default`, and swallowing the error would diverge from `get_thread`.
- Also update **every** other `ThreadSummary {` construction site in `crates/minos-daemon/src/` to include the new `state` field. Search for `ThreadSummary {` to find them all.

- [ ] **Step 4: Update `ConversationAgentStarted` handler to include `state`**

File: `crates/minos-tui/src/update/mod.rs:203-213`

The handler manually constructs a `ThreadSummaryEntry`. Add `state: minos_agent_runtime::ThreadState::Starting`:

```rust
ui.conversation_agent_sessions
    .push(crate::backend::ThreadSummaryEntry {
        thread_id: thread_id.clone(),
        agent,
        title,
        first_ts_ms: 0,
        last_ts_ms: 0,
        message_count: 0,
        ended_at_ms: None,
        parent_thread_id: None,
        state: minos_agent_runtime::ThreadState::Starting,
    });
```

- [ ] **Step 5: Update all test fixtures that construct `ThreadSummaryEntry`**

Search for `ThreadSummaryEntry {` across `crates/minos-tui/src/` (including `app_tests/`, `ui/mod.rs` subagent_tests). Add `state: ThreadState::Idle` (or `Starting` as appropriate) to every construction site.

Test fixture in `crates/minos-tui/src/ui/mod.rs:317-328`:
```rust
fn session(thread_id: &str, parent_thread_id: Option<&str>) -> ThreadSummaryEntry {
    ThreadSummaryEntry {
        thread_id: thread_id.into(),
        agent: AgentName::Codex,
        title: None,
        first_ts_ms: 0,
        last_ts_ms: 0,
        message_count: 0,
        ended_at_ms: None,
        parent_thread_id: parent_thread_id.map(str::to_string),
        state: minos_agent_runtime::ThreadState::Idle,
    }
}
```

- [ ] **Step 6: Compile and fix any remaining errors**

Run: `cargo check -p minos-tui -p minos-protocol -p minos-daemon`
Expected: clean compilation

- [ ] **Step 7: Run existing tests to verify no regression**

Run: `cargo test -p minos-tui --quiet`
Expected: all existing tests pass

- [ ] **Step 8: Commit**

```bash
git add -A
git commit -m "feat: add state field to ThreadSummaryEntry for mention scoping"
```

---

## Task 2: Scope `room_agent_mention_candidates()` to conversation

**Files:**
- Modify: `crates/minos-tui/src/ui/mod.rs:250-271`
- Test: `crates/minos-tui/src/ui/mod.rs` (add test module)

- [ ] **Step 1: Write failing test for conversation-scoped candidates**

Add a test module in `crates/minos-tui/src/ui/mod.rs` (or extend the existing `subagent_tests` module at line 313). The test should verify that when `nav_stack` contains a `Conversation` level, only `conversation_agent_sessions` appear as `Existing` candidates, not threads from `self.threads`:

```rust
#[cfg(test)]
mod mention_scope_tests {
    use super::*;
    use crate::backend::ThreadSummaryEntry;
    use crate::nav::NavLevel;
    use minos_agent_runtime::ThreadState;
    use minos_protocol::AgentName;

    fn make_thread_entry(id: &str, agent: AgentName, state: ThreadState) -> ThreadEntry {
        ThreadEntry {
            thread_id: id.into(),
            agent,
            workspace: std::path::PathBuf::from("."),
            state,
            parent_thread_id: None,
        }
    }

    fn make_session(id: &str, agent: AgentName, state: ThreadState) -> ThreadSummaryEntry {
        ThreadSummaryEntry {
            thread_id: id.into(),
            agent,
            title: None,
            first_ts_ms: 0,
            last_ts_ms: 0,
            message_count: 0,
            ended_at_ms: None,
            parent_thread_id: None,
            state,
        }
    }

    #[test]
    fn mention_candidates_in_conversation_only_show_conversation_sessions() {
        let mut ui = UiState::default();
        // Global threads: thread-a belongs to a DIFFERENT conversation
        ui.threads.push(make_thread_entry("thread-aaaa1111", AgentName::Codex, ThreadState::Idle));
        // Conversation sessions: thread-bbbb belongs to THIS conversation
        ui.conversation_agent_sessions.push(make_session("thread-bbbb2222", AgentName::Codex, ThreadState::Idle));
        // Set nav to inside a conversation
        ui.nav_stack = vec![
            NavLevel::Projects,
            NavLevel::Conversations { project_id: "p1".into() },
            NavLevel::Conversation { project_id: "p1".into(), conversation_id: "c1".into() },
        ];

        let candidates = ui.room_agent_mention_candidates();
        let existing_thread_ids: Vec<&str> = candidates
            .iter()
            .filter_map(|c| match &c.kind {
                AgentMentionCandidateKind::Existing { thread_id } => Some(thread_id.as_str()),
                _ => None,
            })
            .collect();

        // thread-aaaa1111 (from global threads) must NOT appear
        assert!(!existing_thread_ids.contains(&"thread-aaaa1111"));
        // thread-bbbb2222 (from conversation sessions) MUST appear
        assert!(existing_thread_ids.contains(&"thread-bbbb2222"));
    }

    #[test]
    fn mention_candidates_outside_conversation_hide_existing_threads() {
        let mut ui = UiState::default();
        ui.threads.push(make_thread_entry("thread-aaaa1111", AgentName::Codex, ThreadState::Idle));
        ui.nav_stack = vec![NavLevel::Projects, NavLevel::Conversations { project_id: "p1".into() }];

        let candidates = ui.room_agent_mention_candidates();
        let existing_thread_ids: Vec<&str> = candidates
            .iter()
            .filter_map(|c| match &c.kind {
                AgentMentionCandidateKind::Existing { thread_id } => Some(thread_id.as_str()),
                _ => None,
            })
            .collect();

        assert!(existing_thread_ids.is_empty());
    }

    #[test]
    fn mention_candidates_filter_closed_sessions_in_conversation() {
        let mut ui = UiState::default();
        ui.conversation_agent_sessions.push(make_session(
            "thread-closed111",
            AgentName::Codex,
            ThreadState::Closed { reason: minos_agent_runtime::CloseReason::UserClose },
        ));
        ui.conversation_agent_sessions.push(make_session("thread-open1111", AgentName::Codex, ThreadState::Idle));
        ui.nav_stack = vec![
            NavLevel::Projects,
            NavLevel::Conversations { project_id: "p1".into() },
            NavLevel::Conversation { project_id: "p1".into(), conversation_id: "c1".into() },
        ];

        let candidates = ui.room_agent_mention_candidates();
        let existing_thread_ids: Vec<&str> = candidates
            .iter()
            .filter_map(|c| match &c.kind {
                AgentMentionCandidateKind::Existing { thread_id } => Some(thread_id.as_str()),
                _ => None,
            })
            .collect();

        assert!(!existing_thread_ids.contains(&"thread-closed111"));
        assert!(existing_thread_ids.contains(&"thread-open1111"));
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p minos-tui mention_scope_tests -- --nocapture`
Expected: FAIL — `mention_candidates_in_conversation_only_show_conversation_sessions` fails because `thread-aaaa1111` appears (it comes from `self.threads`). The outside-conversation test fails if existing session hashes are still exposed by the new-conversation input.

- [ ] **Step 3: Implement the scoped `room_agent_mention_candidates()`**

File: `crates/minos-tui/src/ui/mod.rs:250-271`

Replace the function body. Per spec §2.1.1, the source of "Existing" candidates depends on nav level: inside a conversation, use `conversation_agent_sessions` (scoped); outside an active conversation, return installed agents only:

```rust
pub fn room_agent_mention_candidates(&self) -> Vec<AgentMentionCandidate> {
    let mut candidates: Vec<AgentMentionCandidate> = self
        .status
        .agents
        .iter()
        .map(|agent| AgentMentionCandidate::installed(agent.name, agent.status.clone()))
        .collect();

    if self.nav_level().conversation_id().is_some() {
        // Scoped: only sessions belonging to this conversation.
        candidates.extend(
            self.conversation_agent_sessions
                .iter()
                .filter(|session| session.parent_thread_id.is_none())
                .filter(|session| !matches!(session.state, ThreadState::Closed { .. }))
                .map(|session| {
                    AgentMentionCandidate::existing(
                        session.agent,
                        session.thread_id.clone(),
                        short_thread_id(&session.thread_id),
                    )
                }),
        );
    }
    candidates
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p minos-tui mention_scope_tests -- --nocapture`
Expected: all 3 tests PASS

- [ ] **Step 5: Run full test suite to check for regressions**

Run: `cargo test -p minos-tui --quiet`
Expected: all tests pass

- [ ] **Step 6: Commit**

```bash
git add -A
git commit -m "fix: scope @Agent mention candidates to current conversation"
```

---

## Task 3: Scope `thread_id_for_agent_short_id()` to conversation

**Files:**
- Modify: `crates/minos-tui/src/app/submission.rs:414-429`
- Test: `crates/minos-tui/src/app_tests/` (add test)

- [ ] **Step 1: Write failing test for scoped short-id resolution**

Add a test in `crates/minos-tui/src/app_tests/input_and_routing.rs` (or a new test file). The test should verify that `thread_id_for_agent_short_id()` only resolves thread IDs from `conversation_agent_sessions` when inside a conversation:

```rust
#[tokio::test]
async fn short_id_resolver_only_finds_conversation_sessions() {
    let (mut app, _backend) = App::test_app(TestBackend::default()).await;
    // Add a global thread that does NOT belong to this conversation
    app.ui.threads.push(crate::ui::ThreadEntry {
        thread_id: "thread-aaaa1111".into(),
        agent: minos_protocol::AgentName::Codex,
        workspace: ".".into(),
        state: minos_agent_runtime::ThreadState::Idle,
        parent_thread_id: None,
    });
    // Add a conversation session that DOES belong
    app.ui.conversation_agent_sessions.push(crate::backend::ThreadSummaryEntry {
        thread_id: "thread-bbbb2222".into(),
        agent: minos_protocol::AgentName::Codex,
        title: None,
        first_ts_ms: 0,
        last_ts_ms: 0,
        message_count: 0,
        ended_at_ms: None,
        parent_thread_id: None,
        state: minos_agent_runtime::ThreadState::Idle,
    });
    app.ui.nav_stack = vec![
        crate::nav::NavLevel::Projects,
        crate::nav::NavLevel::Conversations { project_id: "p1".into() },
        crate::nav::NavLevel::Conversation { project_id: "p1".into(), conversation_id: "c1".into() },
    ];

    // Should NOT find the global thread
    assert_eq!(
        app.thread_id_for_agent_short_id(minos_protocol::AgentName::Codex, "thread-a"),
        None,
    );
    // SHOULD find the conversation session
    assert_eq!(
        app.thread_id_for_agent_short_id(minos_protocol::AgentName::Codex, "thread-b"),
        Some("thread-bbbb2222".into()),
    );
}
```

Note: The test may need adjustment based on how `App::test_app` is structured in the codebase. Check existing tests in `input_and_routing.rs` for the exact pattern.

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p minos-tui short_id_resolver_only_finds_conversation_sessions -- --nocapture`
Expected: FAIL — the resolver finds `thread-aaaa1111` from `self.ui.threads`

- [ ] **Step 3: Implement scoped `thread_id_for_agent_short_id()`**

File: `crates/minos-tui/src/app/submission.rs:414-429`

```rust
pub(super) fn thread_id_for_agent_short_id(
    &self,
    agent: AgentName,
    short_id: &str,
) -> Option<String> {
    let short_id = short_id.to_ascii_lowercase();
    let in_conversation = self
        .ui
        .nav_stack
        .iter()
        .any(|level| level.conversation_id().is_some());

    if in_conversation {
        self.ui
            .conversation_agent_sessions
            .iter()
            .find(|session| {
                session.agent == agent
                    && (short_thread_id(&session.thread_id).to_ascii_lowercase() == short_id
                        || session.thread_id.to_ascii_lowercase().starts_with(&short_id))
            })
            .map(|session| session.thread_id.clone())
    } else {
        self.ui
            .threads
            .iter()
            .find(|thread| {
                thread.agent == agent
                    && (short_thread_id(&thread.thread_id).to_ascii_lowercase() == short_id
                        || thread.thread_id.to_ascii_lowercase().starts_with(&short_id))
            })
            .map(|thread| thread.thread_id.clone())
    }
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p minos-tui short_id_resolver_only_finds_conversation_sessions -- --nocapture`
Expected: PASS

- [ ] **Step 5: Run full test suite**

Run: `cargo test -p minos-tui --quiet`
Expected: all tests pass

- [ ] **Step 6: Commit**

```bash
git add -A
git commit -m "fix: scope thread_id_for_agent_short_id to current conversation"
```

---

## Task 4: Sync `conversation_agent_sessions` state with live manager events

**Files:**
- Modify: `crates/minos-tui/src/app/` (wherever manager events update `ui.threads` state)

When a thread transitions state (e.g. Idle → Running → Idle), the `conversation_agent_sessions` entry must also update. Currently these events update `ui.threads` but not `conversation_agent_sessions`.

- [ ] **Step 1: Find where thread state transitions update `ui.threads`**

Search in `crates/minos-tui/src/app/lifecycle.rs` and `crates/minos-tui/src/update/` for where `ThreadEntry.state` is mutated on manager events (e.g. `ThreadStateChanged`, `ThreadClosed`).

- [ ] **Step 2: Add symmetric update to `conversation_agent_sessions`**

At each location where `ui.threads[i].state` is updated, also update the matching entry in `ui.conversation_agent_sessions` (if it exists for the same `thread_id`):

```rust
if let Some(session) = ui
    .conversation_agent_sessions
    .iter_mut()
    .find(|s| s.thread_id == thread_id)
{
    session.state = new_state.clone();
}
```

Key transition points to handle:
- Thread started (Starting → Idle/Running)
- Thread running (→ Running)
- Thread idle (→ Idle)
- Thread closed (→ Closed)
- Thread suspended (→ Suspended)

- [ ] **Step 3: Run full test suite**

Run: `cargo test -p minos-tui --quiet`
Expected: all tests pass

- [ ] **Step 4: Commit**

```bash
git add -A
git commit -m "fix: sync conversation_agent_sessions state with live manager events"
```

---

## Task 5: Final verification and cleanup

- [ ] **Step 1: Run the full TUI test suite**

Run: `cargo test -p minos-tui --quiet`
Expected: all pass

- [ ] **Step 2: Run protocol and daemon tests**

Run: `cargo test -p minos-protocol --quiet && cargo test -p minos-daemon --quiet`
Expected: all pass

- [ ] **Step 3: Run agent-runtime tests**

Run: `cargo test -p minos-agent-runtime --lib -j1 --quiet`
Expected: all pass

- [ ] **Step 4: Manual verification (if possible)**

Build and run the TUI, open a conversation, type `@`, verify only that conversation's agent sessions appear as candidates.

- [ ] **Step 5: Final commit if any cleanup needed**

```bash
git add -A
git commit -m "test: verify agent mention scoping across conversations"
```
