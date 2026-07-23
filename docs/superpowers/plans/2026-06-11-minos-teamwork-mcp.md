# Implement Minos Teamwork MCP

## Breaking Change Notice

This plan intentionally changes the current MCP surface.

- Binary/server name changes from `minos-mcp` to `minos-teamwork-mcp`.
- Tool names change:
  - `list_chat_messages` -> `list_room_messages`
  - `request_agent_help` -> `delegate_to_agent`
  - `mention_user` -> `post_room_update`
- The MCP child process remains the MCP protocol adapter, but every Minos custom tool executes in the Minos host process through Unix socket IPC.
- No compatibility aliases are planned unless a later product decision explicitly asks for them.

Migration steps for internal callers:

1. Start `minos-teamwork-mcp` instead of `minos-mcp`.
2. Remove any `--db-path` MCP child-process argument.
3. Use `--socket-path`, `--room-id`, and `--source-agent`.
4. Update tool calls to the renamed tool names.
5. Update injected skills and prompt guidance to reference the teamwork server and current tool names.

## Execution Status

Implemented on June 11, 2026:

- Renamed the binary and MCP server surface to `minos-teamwork-mcp` / `minos_teamwork`.
- Renamed the original tools to `list_room_messages`, `delegate_to_agent`, and `post_room_update`.
- Added `get_delegation_status`, `cancel_delegation`, `ask_user_question`, `check_user_feedback`, and `react_to_message`.
- Routed every custom MCP tool through Unix socket IPC into the Minos host process.
- Extracted a `teamwork_mcp` catalog with per-tool schema, permission, socket-request mapping, and skill refs.
- Connected TUI skill installation to catalog-declared `SkillRef`s.
- Added durable SQLite state for delegation, user feedback, and message reactions.

Verification run:

```sh
cargo fmt
cargo check -p minos-chat-store -p minos-agent-runtime -p minos-tui -p minos-daemon
cargo test -p minos-chat-store -p minos-agent-runtime -p minos-tui -p minos-daemon
```

## Feasibility Assessment

The code has the right transport boundary: `crates/minos-chat-store/src/mcp_server.rs` handles MCP stdio protocol, `crates/minos-chat-store/src/mcp_handler.rs` handles Unix socket frames, and TUI/daemon host code executes socket requests in `crates/minos-tui/src/app.rs` and `crates/minos-daemon/src/agent.rs`. The previous IPC review feedback is addressed by starting host socket listeners and routing custom tools through the host.

## Current Surface Inventory

- `crates/minos-chat-store/src/mcp_server.rs` -- MCP stdio protocol adapter; currently owns tool schemas and argument parsing.
- `crates/minos-chat-store/src/mcp_socket.rs` -- socket request and response protocol between MCP child and Minos host.
- `crates/minos-chat-store/src/mcp_handler.rs` -- host-side Unix socket listener and frame dispatch.
- `crates/minos-chat-store/src/bin/minos-teamwork-mcp.rs` -- MCP binary entry point.
- `crates/minos-chat-store/Cargo.toml` -- current MCP binary declaration.
- `crates/minos-chat-store/src/lib.rs` -- chat persistence; target home for teamwork business state.
- `crates/minos-tui/src/app.rs` -- embedded TUI host executor for MCP socket requests.
- `crates/minos-tui/src/backend/embedded.rs` -- starts the embedded host socket handler.
- `crates/minos-tui/src/main.rs` -- hidden MCP subcommand and embedded MCP configuration.
- `crates/minos-daemon/src/agent.rs` -- daemon host executor and socket handler startup.
- `crates/minos-agent-runtime/src/config.rs` -- MCP server binary/socket configuration.
- `crates/minos-agent-runtime/src/manager.rs` -- injects MCP server config into Codex, Claude, Gemini, and OpenCode.
- `crates/minos-tui/skills/minos-teamwork/SKILL.md` -- existing high-level teamwork skill to evolve into server-level guidance.

## Design

### Key Design Decisions

1. Use one MCP server for the teamwork domain.
   - Chosen: `minos-teamwork-mcp` server with a catalog of modular tools.
   - Rejected: one MCP server per tool.
   - Reason: the tools share room, source agent, socket, permissions, and teamwork skill context.

2. Split server, catalog, tool, and host executor responsibilities.
   - Chosen: protocol adapter owns MCP JSON-RPC, catalog owns tool metadata, individual tools own schemas and argument mapping, host executors own side effects.
   - Rejected: one large `match` in the MCP server and host code.
   - Reason: independent tools need independent schema, permissions, tests, and skill references.

3. Execute every custom tool in the Minos host.
   - Chosen: MCP child maps tool call arguments to `SocketRequest` and waits for `SocketResponse`.
   - Rejected: read-only tools directly opening SQLite in the child.
   - Reason: a single host execution path improves permissions, observability, and lifecycle control.

4. Persist workflow state, not IPC state.
   - Chosen: delegation, feedback, and reaction state live in SQLite.
   - Rejected: resurrecting a DB command queue.
   - Reason: delegation and feedback are durable product state; IPC remains Unix socket request/response.

5. Attach skills to tools declaratively.
   - Chosen: tools declare `SkillRef`s, and `AgentRuntime` injects skills based on enabled tools.
   - Rejected: tool executors injecting skills.
   - Reason: execution and agent startup are different lifecycles.

### Concrete Type Definitions

`crates/minos-chat-store/src/teamwork_mcp/tools/mod.rs`

```rust
pub trait TeamworkMcpTool: Send + Sync {
    fn name(&self) -> &'static str;
    fn description(&self) -> &'static str;
    fn permission(&self) -> TeamworkMcpPermission;
    fn input_schema(&self) -> serde_json::Value;
    fn to_socket_request(
        &self,
        ctx: ToolCallContext,
        args: serde_json::Value,
    ) -> anyhow::Result<crate::mcp_socket::SocketRequest>;
    fn skill_refs(&self) -> &'static [SkillRef];
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TeamworkMcpPermission {
    ReadRoom,
    DelegateToAgent,
    UpdateRoom,
    AskUser,
    React,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SkillRef {
    pub id: &'static str,
    pub path: &'static str,
    pub inject_when: SkillInjectWhen,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkillInjectWhen {
    ServerEnabled,
    ToolEnabled,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolCallContext {
    pub room_id: String,
    pub source_agent: Option<minos_domain::AgentName>,
}
```

`crates/minos-chat-store/src/teamwork_mcp/catalog.rs`

```rust
pub struct TeamworkMcpToolCatalog {
    tools: Vec<Box<dyn TeamworkMcpTool>>,
}

impl TeamworkMcpToolCatalog {
    pub fn default_catalog() -> Self;
    pub fn tool_schemas(&self, permissions: TeamworkMcpPermissions) -> Vec<serde_json::Value>;
    pub fn socket_request_for_call(
        &self,
        permissions: TeamworkMcpPermissions,
        ctx: ToolCallContext,
        name: &str,
        args: serde_json::Value,
    ) -> anyhow::Result<crate::mcp_socket::SocketRequest>;
    pub fn skill_refs(&self, permissions: TeamworkMcpPermissions) -> Vec<SkillRef>;
}
```

### Tool Set

- `list_room_messages` -- read room history with cursor pagination.
- `delegate_to_agent` -- assign a concrete task to another room agent.
- `get_delegation_status` -- inspect a delegation's durable status.
- `cancel_delegation` -- cancel a pending or running delegation.
- `ask_user_question` -- create a non-blocking user feedback request.
- `check_user_feedback` -- inspect whether the user answered.
- `post_room_update` -- append a concise user-visible room update.
- `react_to_message` -- add or remove a lightweight emoji reaction.

### Usage Example

MCP child process:

```rust
let catalog = TeamworkMcpToolCatalog::default_catalog();
let request = catalog.socket_request_for_call(
    config.permissions,
    ToolCallContext {
        room_id: config.room_id.clone(),
        source_agent: config.source_agent,
    },
    tool_name,
    tool_args,
)?;
let result = send_socket_request(&config.socket_path, request).await?;
```

Agent runtime skill injection:

```rust
let skill_refs = TeamworkMcpToolCatalog::default_catalog()
    .skill_refs(config.permissions);
inject_teamwork_skills(agent, skill_refs);
```

## Phase 1: Rename Server And Existing Tools

**File: `crates/minos-chat-store/Cargo.toml`**

- Rename bin entry from `minos-mcp` to `minos-teamwork-mcp`.
- Point it at `src/bin/minos-teamwork-mcp.rs`.

**File: `crates/minos-chat-store/src/bin/minos-mcp.rs`**

- Move to `src/bin/minos-teamwork-mcp.rs`.
- Remove old binary file.
- Keep MCP stdio argument shape: `--socket-path`, `--room-id`, `--source-agent`, permission disables.

**File: `crates/minos-chat-store/src/mcp_server.rs`**

- Rename server info from `minos-mcp` to `minos-teamwork-mcp`.
- Rename tools:
  - `list_chat_messages` -> `list_room_messages`
  - `request_agent_help` -> `delegate_to_agent`
  - `mention_user` -> `post_room_update`

**File: `crates/minos-chat-store/src/mcp_socket.rs`**

- Rename variants:
  - `ListChatMessages` -> `ListRoomMessages`
  - `RequestAgentHelp` -> `DelegateToAgent`
  - `MentionUser` -> `PostRoomUpdate`

**File: `crates/minos-agent-runtime/src/config.rs`**

- Default MCP binary becomes `minos-teamwork-mcp`.

**File: `crates/minos-agent-runtime/src/manager.rs`**

- Update tests and injected server args to use `minos-teamwork-mcp`.

**File: `crates/minos-tui/src/main.rs`**

- Hidden subcommand becomes `minos-teamwork-mcp`.

**File: `crates/minos-tui/src/app.rs` and `crates/minos-daemon/src/agent.rs`**

- Update host match arms for renamed socket variants.

Rationale: preserve behavior under product-level names before adding new workflow state.

## Phase 2: Extract Tool Catalog

**File: `crates/minos-chat-store/src/teamwork_mcp/mod.rs`**

- Add `catalog`, `permissions`, and `tools` modules.

**File: `crates/minos-chat-store/src/teamwork_mcp/tools/list_room_messages.rs`**

- Move schema and args parsing out of `mcp_server.rs`.
- Convert args to `SocketRequest::ListRoomMessages`.

**File: `crates/minos-chat-store/src/teamwork_mcp/tools/delegate_to_agent.rs`**

- Move schema and args parsing out of `mcp_server.rs`.
- Convert args to `SocketRequest::DelegateToAgent`.

**File: `crates/minos-chat-store/src/teamwork_mcp/tools/post_room_update.rs`**

- Move schema and args parsing out of `mcp_server.rs`.
- Convert args to `SocketRequest::PostRoomUpdate`.

**File: `crates/minos-chat-store/src/mcp_server.rs`**

- Remove per-tool hardcoded schema functions.
- Use `TeamworkMcpToolCatalog` for `tools/list` and `tools/call`.

Rationale: adding a tool becomes module registration plus host executor implementation.

## Phase 3: Add Delegation State

**File: `crates/minos-chat-store/src/lib.rs`**

- Add durable table `teamwork_delegations`.
- Add types:
  - `TeamworkDelegation`
  - `NewTeamworkDelegation`
  - `TeamworkDelegationStatus`
- Add methods:
  - `create_delegation`
  - `update_delegation_status`
  - `get_delegation`
  - `cancel_delegation`

**File: `crates/minos-chat-store/src/mcp_socket.rs`**

- Add:
  - `GetDelegationStatus`
  - `CancelDelegation`
- Extend `DelegateToAgent` with `task`, `expected_output`, and optional `context_message_ids`.

**File: `crates/minos-tui/src/app.rs`**

- Create delegation row before dispatch.
- Update delegation status after dispatch scheduling.
- Implement status and cancel requests.

**File: `crates/minos-daemon/src/agent.rs`**

- Create delegation row before `AgentManager::dispatch_message`.
- Update status with target session id on success.
- Implement status and cancel requests.

Rationale: delegation is business state and should survive process restarts.

## Phase 4: Add User Feedback State

**File: `crates/minos-chat-store/src/lib.rs`**

- Add durable table `teamwork_feedback_requests`.
- Add types and methods for creating, answering, and reading feedback requests.

**File: `crates/minos-chat-store/src/mcp_socket.rs`**

- Add:
  - `AskUserQuestion`
  - `CheckUserFeedback`

**File: `crates/minos-tui/src/app.rs`**

- `ask_user_question` appends a visible room question and creates pending feedback.
- `check_user_feedback` reads feedback state.

**File: `crates/minos-daemon/src/agent.rs`**

- Same host behavior as TUI, writing to chat store.

Rationale: user questions should be non-blocking; agents poll status when needed.

## Phase 5: Add Room Reactions

**File: `crates/minos-chat-store/src/lib.rs`**

- Add durable table `teamwork_message_reactions`.
- Add methods:
  - `add_reaction`
  - `remove_reaction`
  - `list_message_reactions`

**File: `crates/minos-chat-store/src/mcp_socket.rs`**

- Add `ReactToMessage`.

**File: `crates/minos-tui/src/ui/group_chat.rs`**

- Render reactions under or beside room messages.

**File: `crates/minos-tui/src/app.rs` and `crates/minos-daemon/src/agent.rs`**

- Implement add/remove reaction host execution.

Rationale: reactions provide low-noise acknowledgement without appending room text.

## Phase 6: Add Tool Skills

**File: `crates/minos-tui/skills/minos-teamwork/SKILL.md`**

- Update server-level collaboration guidance for `minos-teamwork-mcp`.

**File: `crates/minos-tui/skills/minos-teamwork/tools/list-room-messages/SKILL.md`**

- Document pagination, message fields, and when to read history.

**File: `crates/minos-tui/skills/minos-teamwork/tools/delegation/SKILL.md`**

- Document delegation etiquette, status polling, cancellation, and avoiding loops.

**File: `crates/minos-tui/skills/minos-teamwork/tools/user-feedback/SKILL.md`**

- Document asking concise questions and checking feedback asynchronously.

**File: `crates/minos-tui/skills/minos-teamwork/tools/reactions/SKILL.md`**

- Document lightweight acknowledgement with reactions.

**File: `crates/minos-agent-runtime/src/manager.rs`**

- Collect skill refs from the catalog and inject enabled tool skills into agent startup prompts/config where supported.

Rationale: tool availability and agent guidance stay in sync.

## Phase 7: Verification

Run:

```bash
cargo fmt
cargo check -p minos-chat-store -p minos-agent-runtime -p minos-tui -p minos-daemon
cargo test -p minos-chat-store -p minos-agent-runtime -p minos-tui -p minos-daemon
rg -- "minos-mcp|list_chat_messages|request_agent_help|mention_user|chat_mcp_commands" crates/
```

## Architectural Notes

- Semver impact: internal crate API and hidden CLI names change; no compatibility aliases are planned.
- Object safety: `TeamworkMcpTool` should be object-safe by avoiding generic methods and returning owned `Value`s.
- Side effects: delegation, feedback, and reactions create durable rows; MCP IPC remains request/response only.
- Not changed: MCP protocol methods remain in the child process; only custom tool execution is host-side.
- Cross-crate dependencies: no new dependency is required for phases 1 and 2; later phases may use existing `uuid`, `serde_json`, and `chrono` already present in workspace crates.

## File Change Summary

- `crates/minos-agent-runtime/src/config.rs` -- rename default teamwork MCP binary.
- `crates/minos-agent-runtime/src/manager.rs` -- inject renamed server and collect future tool skill refs.
- `crates/minos-chat-store/Cargo.toml` -- rename MCP binary.
- `crates/minos-chat-store/src/bin/minos-teamwork-mcp.rs` -- new teamwork MCP entry point.
- `crates/minos-chat-store/src/bin/minos-mcp.rs` -- removed old MCP binary.
- `crates/minos-chat-store/src/lib.rs` -- add delegation, feedback, and reaction persistence.
- `crates/minos-chat-store/src/mcp_server.rs` -- keep MCP protocol adapter, delegate tool metadata to catalog.
- `crates/minos-chat-store/src/mcp_socket.rs` -- rename and add teamwork socket requests.
- `crates/minos-chat-store/src/teamwork_mcp/*` -- add catalog, permissions, and per-tool definitions.
- `crates/minos-daemon/src/agent.rs` -- execute teamwork socket requests in daemon host.
- `crates/minos-tui/src/app.rs` -- execute teamwork socket requests in embedded TUI host.
- `crates/minos-tui/src/main.rs` -- expose hidden `minos-teamwork-mcp` subcommand.
- `crates/minos-tui/skills/minos-teamwork/*` -- update server and tool-level agent guidance.
