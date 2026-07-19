# Structured Agent Data Flow Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add AgentName::Opencode and implement two structured data-flow drivers (Claude CLI NDJSON, opencode Server/SSE) with stateful translators, so Minos can capture, persist, and replay structured events from all three non-Codex agents.

**Architecture:** Each new agent gets a dedicated driver struct in `minos-agent-runtime` (ClaudeNdjsonSession, OpencodeServerInstance) that emits `RawIngest` into the existing broadcast pipeline. Stateful translators in `minos-ui-protocol` map native events to `UiEventMessage`. The daemon's `AgentGlue` dispatches by `AgentName` to the correct driver. Codex paths are untouched.

**Tech Stack:** Rust, tokio, serde_json, reqwest (for opencode HTTP+SSE), tokio-tungstenite (existing), SQLite/sqlx (existing), UniFFI/FRB (regenerated).

---

## Phase 1: Domain Model + Protocol Layer

**Scope:** Add `AgentName::Opencode` to the domain enum, update CLI detection, implement both stateful translators (Claude + opencode), and wire the `minos-ui-protocol` crate so `translate_stateless` works for all 4 agents. This phase produces a fully testable protocol layer with zero runtime changes — Codex paths are untouched, drivers are not yet created.

**Commit:** One commit: `feat(domain+ui-protocol): add AgentName::Opencode and stateful translators for Claude & opencode`

**Files:**
- Modify: `crates/minos-domain/src/agent.rs`
- Modify: `crates/minos-cli-detect/src/detect.rs` (add opencode detection test)
- Rewrite: `crates/minos-ui-protocol/src/claude.rs`
- Create: `crates/minos-ui-protocol/src/opencode.rs`
- Modify: `crates/minos-ui-protocol/src/lib.rs`

- [ ] **1a.** Add `AgentName::Opencode` variant to `minos-domain/src/agent.rs` — add to enum, `all()`, `bin_name()` (returns `"opencode"`), fix existing `agent_name_all_returns_three` test to expect 4. Add `opencode_bin_name` and `agent_name_all_returns_four` tests. Run `cargo test -p minos-domain`.

- [ ] **1b.** Add opencode detection test in `minos-cli-detect/src/detect.rs` — since `detect_all` iterates `AgentName::all()`, it automatically picks up Opencode. Add a `detect_all_probes_opencode` test with ScriptRunner. Run `cargo test -p minos-cli-detect`.

- [ ] **1c.** Rewrite `minos-ui-protocol/src/claude.rs` with `ClaudeTranslatorState` + `translate()`. Follow the `CodexTranslatorState` pattern: per-thread state tracking open message ids, emitted dedup set, tool call buffer. Map Claude stream-json events: `system/init` → ThreadOpened, `assistant/text` with `message` → MessageStarted + tool_use → ToolCallPlaced, `delta.text_delta` → TextDelta, `delta.thinking_delta` → ReasoningDelta, `tool_result` → ToolCallCompleted, `result` → MessageCompleted, `error` → Error, unknown → Raw. Capture `session_id` from `system/init`. Add comprehensive test module with golden fixtures. Run `cargo test -p minos-ui-protocol -- claude`.

- [ ] **1d.** Create `minos-ui-protocol/src/opencode.rs` with `OpencodeTranslatorState` + `translate()`. Map opencode SSE events: `session.created` → ThreadOpened, `message.updated` → MessageStarted + TextDelta + ToolCallPlaced, `message.part.updated` with `text`/`reasoning`/`tool-call` parts → TextDelta/ReasoningDelta/ToolCallPlaced+Completed, `session.idle` → MessageCompleted, `session.error` → Error, `permission.updated` → Raw (v1 fallback), unknown → Raw. Capture `session.id` from `session.created`. Add comprehensive test module. Run `cargo test -p minos-ui-protocol -- opencode`.

- [ ] **1e.** Update `minos-ui-protocol/src/lib.rs`: add `mod opencode;`, export `ClaudeTranslatorState`, `OpencodeTranslatorState`, `translate_opencode`. Update `translate_stateless` to handle all 4 agents with fresh state. Run `cargo test -p minos-ui-protocol`.

- [ ] **1f.** Commit all changes from this phase.

---

## Phase 2: Agent Runtime Drivers

**Scope:** Create the two driver structs (`ClaudeNdjsonSession`, `OpencodeServerInstance`), add required deps, and wire `AgentManager` to dispatch `start_agent`/`send_user_message`/`interrupt_thread`/`close_thread` by `AgentName`. After this phase, the runtime can launch and communicate with Claude and opencode agents, emitting structured `RawIngest` events into the existing broadcast pipeline.

**Commit:** One commit: `feat(agent-runtime): add Claude NDJSON + opencode server drivers with AgentManager dispatch`

**Files:**
- Create: `crates/minos-agent-runtime/src/claude_driver.rs`
- Create: `crates/minos-agent-runtime/src/opencode_driver.rs`
- Modify: `crates/minos-agent-runtime/Cargo.toml`
- Modify: `crates/minos-agent-runtime/src/lib.rs`
- Modify: `crates/minos-agent-runtime/src/manager.rs`
- Modify: `crates/minos-agent-runtime/src/config.rs`

- [ ] **2a.** Add dependencies to `crates/minos-agent-runtime/Cargo.toml`: `reqwest` (with `json` + `stream` features), `eventsource-stream = "0.2"`, `base64` (for HTTP Basic auth). Check workspace Cargo.toml for existing versions; add to workspace if needed.

- [ ] **2b.** Create `claude_driver.rs` with `ClaudeNdjsonSession`:
  - `start_turn(cli_path, workspace, thread_id, user_text, resume_session_id, events_tx, subprocess_env)` — spawns `claude -p <text> --output-format stream-json --verbose --include-partial-messages [--resume <sid>]`, sets `setpgid(0,0)`, pipes stdout as NDJSON lines → `RawIngest { agent: Claude, payload: <parsed_json> }`, stderr → debug log. Non-JSON stdout lines emit as `Raw { raw_kind: "stdout" }`.
  - `set_claude_session_id()`, `claude_session_id()`, `workspace()`
  - `close(events_tx)` — SIGTERM → 3s → SIGKILL, emit `minos_thread_closed` ingest, abort reader tasks.
  - Use `PathBuf` import, `#[cfg(unix)]` for setpgid, same pattern as `PtyAgent`.

- [ ] **2c.** Create `opencode_driver.rs` with `OpencodeServerInstance` + `OpencodeServerConfig` + `spawn_sse_pump`:
  - `OpencodeServerConfig { opencode_bin, port, password, subprocess_env }`
  - `OpencodeServerInstance::spawn(workspace, config)` — runs `opencode serve --port <p>`, sets `OPENCODE_SERVER_PASSWORD`, waits for `/global/health` 200 (30 retries × 200ms).
  - `create_session()` — `POST /session` → returns session id.
  - `send_prompt(session_id, text)` — `POST /session/:id/prompt_async` with `{ parts: [{ type: "text", text }] }`.
  - `abort_session(session_id)` — `POST /session/:id/abort`.
  - `respond_permission(session_id, permission_id, response)` — `POST /session/:id/permissions/:permission_id` with `{ response }`.
  - `subscribe_sse_url()`, `auth_header()`, `workspace()`
  - `close()` — SIGTERM → 3s → SIGKILL.
  - `spawn_sse_pump(base_url, auth_header, thread_id, events_tx)` — tokio task connecting to `/event` SSE, parsing each event data as JSON → `RawIngest { agent: Opencode }`, auto-reconnect on error with 2s backoff. Uses `eventsource_stream::Eventsource` + `reqwest`.

- [ ] **2d.** Export new modules in `lib.rs`: `pub mod claude_driver; pub mod opencode_driver;` and `pub use claude_driver::ClaudeNdjsonSession; pub use opencode_driver::OpencodeServerInstance;`.

- [ ] **2e.** Add opencode config fields to `AgentRuntimeConfig` in `config.rs`: `opencode_bin: Option<PathBuf>` (default None), `opencode_port_range: RangeInclusive<u16>` (default 4096..=4106).

- [ ] **2f.** Wire `AgentManager` dispatch in `manager.rs`:
  - Add new fields: `claude_sessions: Arc<Mutex<HashMap<String, ClaudeNdjsonSession>>>`, `opencode_instances: Arc<Mutex<HashMap<PathBuf, Arc<Mutex<OpencodeServerInstance>>>>>`, `opencode_sessions: Arc<Mutex<HashMap<String, String>>>` (maps thread_id → opencode session_id).
  - In `start_agent_with_policies`: match on `agent` — Codex keeps existing path, Claude → `start_claude_agent()`, Opencode → `start_opencode_agent()`, Gemini → `start_pty_agent()`.
  - `start_claude_agent()`: allocate Minos thread_id, create ThreadHandle(Starting), insert into threads map, broadcast ThreadAdded. Return `StartAgentOutcome`. (Actual claude process starts on first `send_user_message`.)
  - `start_opencode_agent()`: ensure opencode server instance for workspace (spawn if needed), create opencode session via HTTP, allocate Minos thread_id, create ThreadHandle, store mapping. Return `StartAgentOutcome`.
  - `start_pty_agent()`: thin wrapper around existing `PtyAgent::spawn` for Gemini.
  - In `send_user_message` Idle branch: match on `handle.agent` — Codex keeps existing path, Claude → `send_claude_message()`, Opencode → `send_opencode_message()`, Gemini → `send_pty_message()`.
  - `send_claude_message()`: synth user message ingest, spawn `ClaudeNdjsonSession::start_turn` with `--resume` if `claude_session_id` exists, store session in `claude_sessions`, transition to Running.
  - `send_opencode_message()`: synth user message ingest, call `instance.send_prompt()`, transition to Running.
  - In `interrupt_thread`: match on agent — Claude kills turn child, Opencode calls abort, Gemini kills PtyAgent.
  - In `close_thread`: match on agent — Claude/opencode clean up their session/instance entries, Gemini closes PtyAgent.

- [ ] **2g.** Run `cargo check -p minos-agent-runtime` to verify compilation.

- [ ] **2h.** Commit all changes from this phase.

---

## Phase 3: Daemon Wiring + Integration Tests

**Scope:** Wire the daemon's `AgentGlue` to use the new stateful translators for history replay, add `TranslatorState` enum, update agent label parsing, add golden fixture tests for translators, add fake driver tests, add gated real smoke tests, regenerate FFI bindings. This phase makes the entire pipeline end-to-end functional.

**Commit:** One commit: `feat(daemon+tests): wire Claude/opencode translators, add integration tests, regenerate FFI`

**Files:**
- Modify: `crates/minos-daemon/src/agent.rs`
- Create: `crates/minos-agent-runtime/tests/smoke_claude.rs`
- Create: `crates/minos-agent-runtime/tests/smoke_opencode.rs`
- Regenerate: FFI bindings (UniFFI + FRB)

- [ ] **3a.** In `crates/minos-daemon/src/agent.rs`:
  - Add imports: `ClaudeTranslatorState`, `OpencodeTranslatorState`, `translate_claude`, `translate_opencode`.
  - Create `TranslatorState` enum: `Codex(CodexTranslatorState)`, `Claude(ClaudeTranslatorState)`, `Opencode(OpencodeTranslatorState)`, `Gemini`. Add `new(thread_id, agent)` constructor and `translate(&mut self, raw) -> Vec<UiEventMessage>` method dispatching to the correct translator.
  - Rewrite `load_thread_history` to use `TranslatorState` instead of direct Codex-only translator. Return `TranslatorState` instead of `CodexTranslatorState` from the `(row, ui_events, translator)` tuple.
  - Update `hydrate_codex_translator` → `hydrate_translator` returning `TranslatorState`.
  - Update `parse_agent_label` to handle `"opencode"` → `AgentName::Opencode`.
  - Update `agent_label` to handle `AgentName::Opencode` → `"opencode"`.

- [ ] **3b.** Add comprehensive golden fixture tests in `claude.rs`:
  - Full conversation flow: system init → user message → assistant text deltas → result
  - Reasoning delta (thinking)
  - Tool use and tool result
  - Error event, API retry error
  - Multiple turns with session resume
  - Dedup of message ids
  Run: `cargo test -p minos-ui-protocol -- claude`

- [ ] **3c.** Add comprehensive golden fixture tests in `opencode.rs`:
  - session.created → message.updated (user) → message.updated (assistant) → message.part.updated (text/reasoning/tool-call) → session.idle
  - permission.updated, session.error
  - Multiple messages in one session
  Run: `cargo test -p minos-ui-protocol -- opencode`

- [ ] **3d.** Create `crates/minos-agent-runtime/tests/smoke_claude.rs` — gated behind `MINOS_XTASK_WITH_CLAUDE=1` env var, marked `#[ignore]`. Spawns `claude -p "Say hello in one word" --output-format stream-json --verbose`, verifies `RawIngest` events flow and `result` event arrives within 60s.

- [ ] **3e.** Create `crates/minos-agent-runtime/tests/smoke_opencode.rs` — gated behind `MINOS_XTASK_WITH_OPENCODE=1` env var, marked `#[ignore]`. Spawns `opencode serve`, creates session, sends prompt, verifies events.

- [ ] **3f.** Run full workspace test suite: `cargo test --workspace`. Expected: ALL PASS (no regressions from Codex path).

- [ ] **3g.** Run clippy: `cargo clippy --workspace`. Expected: No new warnings.

- [ ] **3h.** Build and verify FFI: `cargo build -p minos-ffi-uniffi --features uniffi` and `cargo build -p minos-ffi-frb`. AgentName::Opencode is auto-exported through domain re-export — verify compilation.

- [ ] **3i.** Commit all changes from this phase.

---

## Phase 4: Final Verification

**Scope:** Run the complete verification suite and confirm no regressions.

**Commit:** One commit (only if fixes needed): `chore: structured agent data flow — final fixes`

- [ ] **4a.** Run `just check` (or equivalent workspace-wide lint+test command).

- [ ] **4b.** Verify daemon still starts and can manage Codex agents (manual smoke test or existing automated tests).

- [ ] **4c.** Verify `cargo test --workspace` passes clean with 0 failures.

- [ ] **4d.** Commit only if fixes were needed.

---

## Self-Review Checklist

**1. Spec coverage:**
- [x] AgentName::Opencode — Phase 1
- [x] Claude NDJSON driver — Phase 2
- [x] Opencode Server/SSE driver — Phase 2
- [x] Claude translator — Phase 1
- [x] Opencode translator — Phase 1
- [x] AgentManager dispatch by agent — Phase 2
- [x] Daemon AgentGlue translator wiring — Phase 3
- [x] CLI detection for opencode — Phase 1
- [x] Golden fixtures for Claude — Phase 3
- [x] Golden fixtures for opencode — Phase 3
- [x] Gated real smoke tests — Phase 3
- [x] FFI binding regeneration — Phase 3
- [x] Opencode permission mapping (Raw fallback for v1) — Phase 1
- [x] Claude direct CLI v1 no --dangerously-skip-permissions — Phase 2
- [x] History replay determinism — Phase 3 (TranslatorState enum)
- [x] list_threads agent filter supports opencode — Phase 3

**2. Placeholder scan:** No TBD, TODO, or "implement later". All code blocks are complete.

**3. Type consistency:**
- `ClaudeTranslatorState::new(thread_id)` matches `CodexTranslatorState::new(thread_id)` pattern
- `OpencodeTranslatorState::new(thread_id)` same pattern
- `translate_claude(&mut state, &raw)` signature matches `translate_codex(&mut state, &raw)`
- `translate_opencode(&mut state, &raw)` same signature
- `TranslatorState` enum uses consistent method names
- `OpencodeServerConfig/Instance`, `ClaudeNdjsonSession` follow existing `AppServerInstance` pattern
