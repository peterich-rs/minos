# Raise Claude Runtime to Full Control-Plane Fidelity

> **Status:** Phase 0–3 **implemented** in runtime (2026-08-07). Design remains research SSOT
> for optional Phase 4+ and wire golden annex. Verified against Claude Code **2.1.222**,
> Agent SDK docs, ecosystem (VS Code / Zed / Happy / Buzz), and Minos sources.
>
> **Shipped:** `ClaudeControlSession` (bidirectional stream-json), `ToolCallCompleted`
> projection, `PendingApprovalTarget::ClaudeControl`, interrupt/resume multi-turn,
> Running-turn reject. Optional later: catalog browser (6), ACP adapter (7), IDE bridge (8).

## Feasibility Assessment

Fully feasible. Claude Code already exposes a bidirectional headless control plane
(`claude -p --output-format stream-json --input-format stream-json`) that the official
Agent SDK uses for permissions, multi-turn sessions, hooks, and MCP. Minos currently
only consumes the **outbound** half: `ClaudeNdjsonSession` spawns
`claude -p … --output-format stream-json` with `stdin = Stdio::null()`, so approval
reverse-requests are impossible regardless of Desktop/TUI work. The approval parking
machinery (`PendingApprovalTarget` + `resolve_approval` + `ApprovalModal`) already
serves Codex / Gemini ACP / Grok / OpenCode and only needs a Claude target arm. The
translator (`minos-ui-protocol/src/claude.rs`) already projects text / thinking /
tool placement and can grow tool completion + control envelopes without a protocol
crate rewrite.

**Claude does not ship native ACP or an official app-server.** Product UIs reach
Claude through one or more of:

| Plane | What it is | Who uses it |
|-------|------------|-------------|
| **A. stream-json control plane** | NDJSON stdin/stdout; canUseTool / multi-turn / interrupt | Agent SDK, headless hosts, Minos target |
| **B. Session catalog / resume** | `~/.claude/projects/<cwd-key>/*.jsonl` + `--resume` / `--continue` | CLI, VS Code, Desktop, third-party explorers |
| **C. IDE bridge (MCP over WS)** | `~/.claude/ide/<port>.lock` + loopback WebSocket | Official VS Code / JetBrains; Nova/Sublime ports |
| **D. ACP adapter (Node)** | `claude-agent-acp` wraps Agent SDK → ACP JSON-RPC | Zed, Buzz, VS Code ACP Client, Neovim |

Minos should own **A** as SSOT for agent execution, treat **B** as first-class
session continuity, optionally adopt **C** later for editor context parity, and keep
**D** as an opt-in uniformity bridge—not the primary control path.

## Current Surface Inventory

### Runtime driver

- `crates/minos-agent-runtime/src/claude_driver.rs`
  - `ClaudeNdjsonSession` — one process per turn; stdout NDJSON; stdin closed
  - `build_claude_args(...)` — `-p`, `--output-format stream-json`, `--verbose`,
    `--include-partial-messages`, optional `--model` / `--resume` / `--session-id` /
    `--mcp-config` / `--append-system-prompt`
  - `start_turn` / `close` / hard-kill on Drop
- `crates/minos-agent-runtime/src/manager.rs`
  - `claude_sessions: Arc<Mutex<HashMap<String, ClaudeNdjsonSession>>>`
  - `start_claude_agent` / `start_claude_turn` / `send_claude_prompt` /
    `resume_claude_thread`
  - Running-path `send_claude_prompt` re-spawns via `--resume` (not in-session steer)
  - Interrupt path: `child.start_kill()` only
  - `PendingApprovalTarget` variants: `Codex` | `Acp` | `GrokExtMethod` — **no Claude**
  - `resolve_approval` has no Claude reply arm

### Translator / projection

- `crates/minos-ui-protocol/src/claude.rs`
  - Handles `system`, `stream_event`, `assistant`, `result`, `error`
  - Emits `SessionOpened`, text/thinking deltas, `ToolCallPlaced`
  - **Does not** emit `ToolCallCompleted`
  - **Does not** pass through Minos `approval/*` envelopes
  - Unknown types become `Raw { kind: "claude/{other}" }`

### Domain / catalog

- `crates/minos-domain/src/agent.rs` — `AgentName::Claude`, static model discovery,
  `supports_reasoning_effort = false`
- `crates/minos-daemon/src/model_catalog.rs` — `static_claude()` aliases
  (`sonnet` / `opus` / `fable` / `haiku`), empty effort ladders

### UI / docs gaps (explicit)

- `docs/architecture-desktop.md` — “Claude 未接” on Approval / Question path
- `docs/architecture-tui.md` — “Claude 的权限/提问尚未接入”
- Desktop `user-action.ts` / TUI pending-request kinds are agent-agnostic once host
  emits normalized `approval/request`; Claude never parks one today

### Peer control planes (reuse targets, not rewrite)

- Codex: app-server WS + `commandExecution/requestApproval`
- Gemini / Grok: ACP stdio + `session/request_permission` → `PendingApprovalTarget::Acp`
- OpenCode: HTTP/SSE permission/question RPCs

## Ecosystem Research (how others get ACP-like fidelity)

This section is the research SSOT for “how do peers actually integrate Claude Code?”
Patterns below inform Minos design decisions; they are **not** a requirement to
copy architecture wholesale.

### Map: four integration styles

```text
                    ┌─────────────────────────────────────────┐
                    │         Claude Code process              │
                    │  auth / tools / MCP / transcript disk    │
                    └───┬─────────────┬─────────────┬─────────┘
          stream-json   │   --resume  │  IDE MCP    │  SDK API
          control plane │   catalog   │  lockfile   │  (Node/Py)
                        │             │             │
     ┌──────────────────┼─────────────┼─────────────┼──────────────┐
     │                  │             │             │              │
  Minos target       VS Code       VS Code/IDE    Agent SDK     claude-agent-acp
  Happy-style host   session UI    bridge (C)     (host language)   │
  pure Rust (A)      (B)           selection/diff   canUseTool      ACP clients:
                                                                   Zed, Buzz, nvim
```

### 1) Official VS Code extension — dual surface (B + C + GUI)

**What users experience:** a native panel that lists **local** Claude sessions for
the workspace, resumes full history, and continues work without re-explaining context.
Cloud web sessions can also be downloaded and continued locally (one-way; not
synced back to claude.ai).

**How it works (verified against docs + local `~/.claude` layout):**

| Mechanism | Path / contract | Role |
|-----------|-----------------|------|
| **Session catalog** | `~/.claude/projects/<encoded-cwd>/<session-id>.jsonl` | Persistent transcript SSOT for resume |
| **Live session registry** | `~/.claude/sessions/<pid>.json` | Running interactive sessions (`sessionId`, `cwd`, `status`, `name`, `entrypoint`) |
| **Resume API** | `claude --resume <id\|name>` / `--continue` / in-session `/resume` | Restore conversation + selected state |
| **IDE bridge** | `~/.claude/ide/<port>.lock` → loopback WebSocket + MCP | Selection, open editors, diagnostics, `openDiff` |
| **Env injection** | `CLAUDE_CODE_SSE_PORT` + `ENABLE_IDE_INTEGRATION` | Terminal CLI auto-discovers IDE bridge |

Local lockfile shape (observed):

```json
{
  "pid": 8883,
  "workspaceFolders": ["/path/to/workspace"],
  "ideName": "Visual Studio Code",
  "transport": "ws",
  "runningInWindows": false,
  "authToken": "<uuid>"
}
```

Live session registry shape (observed `~/.claude/sessions/<pid>.json`):

```json
{
  "pid": 86364,
  "sessionId": "da4ede8a-…",
  "cwd": "/Users/…/Minos",
  "kind": "interactive",
  "entrypoint": "cli",
  "name": "minos-f9",
  "status": "idle",
  "version": "2.1.222"
}
```

**Important semantics from official session docs:**

- Resume restores history, model (with exceptions), agent, and *most* permission
  modes; **`plan` and `bypassPermissions` are never restored**—must re-enable at launch.
- Flags like `--mcp-config`, `--settings`, `--plugin-dir`, `--add-dir` are **not**
  fully restored; re-pass on resume.
- Headless/`-p` / Agent SDK sessions **do not appear in the interactive session
  picker**, but remain resumable via **explicit session id**.
- VS Code / Desktop / CLI each maintain session *views*; underlying storage is the
  Claude project transcript model.

**Borrow for Minos:**

1. Treat **provider session id** as durable identity; already partially done via
   `handle.codex_session_id` reuse—document and harden as Claude provider id.
2. Optional later: **session browser** reading `~/.claude/projects/<cwd-key>/` for
   “continue a CLI/VS Code session inside Minos” (product differentiator).
3. Do **not** confuse IDE bridge (C) with agent control plane (A)—Minos daemon already
   owns workspace; IDE bridge is only needed if we want terminal-`claude` to drive
   Minos Desktop diffs the way VS Code does.

### 2) Zed — ACP adapter over Agent SDK (D)

Zed does **not** speak stream-json directly in the UI. It:

1. Open-sourced / uses **`claude-agent-acp`** (Apache; evolved from Zed’s adapter).
2. Spawns the adapter as an ACP agent process.
3. Speaks standard ACP (`session/new`, `session/prompt`, `session/request_permission`,
   `session/update`).
4. Adapter wraps **Claude Agent SDK** (which itself drives the CLI control plane).

Stack:

```text
Zed UI ──ACP JSON-RPC──▶ claude-agent-acp ──Agent SDK──▶ claude CLI stream-json
```

**Tradeoffs (community-reported, still accurate as of 2026):**

- Uniform agent UX with Gemini/Codex/OpenCode in ACP editors.
- **Semantic loss / lag**: slash commands, some CLI-only features, subagent nesting
  depend on adapter parity (recent adapters add nested transcripts via `_meta`).
- Extra Node hop and dual maintenance (ACP schema + Claude SDK drift).

**Borrow for Minos:**

- Keep Phase 6 as optional “ACP marketplace mode” using the same adapter Buzz uses.
- Do **not** make ACP the primary Claude path inside Minos (we already have native
  projection for Claude NDJSON and need max fidelity + pure Rust host).

### 3) Happy (slopus/happy) — wrapper CLI + remote control (A-ish + mobile)

Happy is a **mobile/web companion**, not an IDE:

- User runs `happy claude` / `happy codex` instead of the raw CLI.
- Happy CLI wraps the agent process, streams state, and can flip the session into
  **remote mode** for phone/web control.
- Push notifications for **permission requests** and completion.
- Instant desktop↔mobile handoff (press any key to reclaim desktop).
- E2E encryption; open-source app + CLI + server components.

**Borrow for Minos:**

- Minos is already closer to Happy’s product shape (remote clients + host agent)
  than to Zed. Happy validates: **host owns the Claude process**, clients only see
  normalized events + approval UX.
- Push-on-permission is a product requirement for mobile; Minos IM reliability
  program already has approval / attention paths—ClaudeControl must emit the same
  envelopes so push/attention work without a Claude-specific mobile modal.
- Device handoff: map to Minos conversation attachment + single host process
  (daemon holds `ClaudeControlSession`; Desktop/Mobile/Web are thin clients).

### 4) Buzz (`../buzz`) — ACP harness, Claude as tier-1 via adapter (D)

Buzz’s agent execution path for Claude is explicitly:

```text
Buzz Relay ──WS──▶ buzz-acp ──stdio ACP──▶ claude-agent-acp ──SDK──▶ Claude
```

From `buzz-acp` README / config (local sibling repo):

- Claude is a **compiled-in tier-1 runtime** id (`claude`), same class as goose/codex.
- Spawn command defaults to `claude-agent-acp` (also accepts legacy `claude-code-acp`).
- Permission modes mapped through ACP `session/set_config_option`
  (`default` / `acceptEdits` / `bypassPermissions` / `dontAsk` / `plan`).
- Harness concerns Minos already shares: idle timeout, max turn duration, parallel
  agent pool, cancel/rotate session, MCP injection, author gate on inbound chat.

**Borrow for Minos:**

| Buzz pattern | Minos analogue | Action |
|--------------|----------------|--------|
| Idle timeout + max turn wall clock | AgentManager turn lifecycle | Apply same safety valves on ClaudeControlSession |
| `!cancel` vs `!rotate` | interrupt vs new session | Cooperative interrupt ≠ session replace |
| PermissionMode enum + wire string | launch flags / profile | Reuse vocabulary; map to CLI `--permission-mode` |
| Claude via **adapter only** | Optional Phase 6 | Minos default stays **native stream-json** for fidelity |
| Lazy pool / multi agent | Multi-session map | Keep one Claude process per Minos session id |

Buzz proves ACP-adapter integration is production-viable for chat-driven agents,
but also shows the **cost of standardization**: Claude fidelity is capped by
`claude-agent-acp`. Minos Desktop already invests in native Claude projection;
native control plane is the better default.

### 5) Other community patterns worth knowing

| Project | Pattern | Lesson |
|---------|---------|--------|
| **claude-agent-acp** | Official-ish community ACP agent | Optional uniformity layer |
| **Xuanwo/acp-claude-code** | Early ACP bridge | Same idea as Zed adapter lineage |
| **VS Code “Claude Sessions Explorer”** | Read `~/.claude/projects`, spawn `claude --resume` | Catalog UX without owning control plane |
| **Nova / Sublime / Obsidian IDE bridges** | Implement plane **C** lockfile + MCP tools | Editor context ≠ agent control |
| **Roasbeef Go SDK / Hex Elixir SDK** | Reverse-engineered stream-json + control protocol | Good wire references; **not** SSOT—golden against local CLI |
| **code-quest / agentic wrappers** | Spawn stream-json, parse NDJSON event types | Same outbound path Minos has today |

### Research verdict → Minos stance

| Question | Answer |
|----------|--------|
| How does Claude achieve ACP-like capability? | **stream-json bidirectional control plane** (plane A), not native ACP |
| How does VS Code “see local sessions”? | **Plane B** transcript catalog + resume; optional **plane C** for editor tools |
| How does Zed integrate? | **Plane D** ACP adapter over Agent SDK |
| How does Happy integrate? | Host wrapper around Claude process + remote client (closest product analogy) |
| How does Buzz integrate? | **Plane D** via `claude-agent-acp` under `buzz-acp` |
| What should Minos implement first? | **Plane A + B (resume id)**; leave C/D optional |

## Design

### Goal

Bring Claude to **control-plane parity** with Codex / Gemini / OpenCode for the
capabilities Minos product surfaces need:

1. Interactive tool permission / user-question reverse-requests
2. Long-lived multi-turn session (same process) + cooperative interrupt
3. Faithful tool lifecycle projection (`Placed` → `Completed` / failed)
4. Durable resume via Claude provider session id (and optional catalog browser)
5. Optional subagent text visibility and effort / permission-mode knobs
6. Keep teamwork MCP injection working

Out of scope for core phases:

- Migrating Claude onto ACP as the primary transport
- Building or depending on a community Claude app-server as SSOT
- Full IDE bridge (plane C) parity with VS Code selection/diff tools
- Inventing Minos-side subagent trees when CLI still hides Task internals
  (use `--forward-subagent-text` when available; no fake spawn graph)
- Becoming a Happy-style encrypted remote wrapper (Minos already has its own
  relay/sync; only borrow UX patterns)

### Protocol choice (SSOT)

| Option | Verdict |
|--------|---------|
| **Bidirectional Claude stream-json control plane (A)** | **Primary.** Official Agent SDK transport; available on local `claude` CLI (`2.1.x+`). |
| Claude Agent SDK (TS/Python) embedded in Desktop | Rejected as runtime SSOT. Minos agent execution lives in Rust daemon/host. SDK remains the **protocol reference**. |
| Session catalog + `--resume` (B) | **Required companion** for continuity after process death and for “import CLI session” later. |
| IDE bridge lockfile MCP (C) | Optional later if product wants terminal-Claude ↔ Minos Desktop editor coupling. |
| `@agentclientprotocol/claude-agent-acp` (D) | Optional later bridge for “all agents speak ACP” product mode. Not Phase 1. Same path Buzz uses. |
| Official Claude ACP / app-server | Does not exist as native product surface today. Do not block on it. |

### Key design decisions

1. **Stay on Claude-native stream-json; open the control half.**
   Rejected: rewrite Claude as ACP-only. Reason: Claude’s full interactive surface
   (canUseTool, AskUserQuestion, hooks, stdin multi-turn) is already on stream-json.
   ACP would add a Node adapter hop and semantic loss without unlocking new Claude
   capabilities. Ecosystem (Zed/Buzz) pays that cost for *uniformity*; Minos pays for
   *fidelity* by owning plane A.

2. **Long-lived `ClaudeControlSession` replaces process-per-turn `ClaudeNdjsonSession`.**
   Rejected: keep one-shot `-p` and only patch approval via global
   `--dangerously-skip-permissions`. Reason: that removes human control instead of
   wiring it; also blocks true multi-turn / interrupt. Aligns with official
   **Streaming Input Mode** (recommended by Agent SDK docs).

3. **Normalize Claude permission frames into existing Minos `approval/request`.**
   Rejected: Claude-specific Desktop modal path. Reason: Desktop/TUI/Mobile already
   route generic `approval/request` + `minos_local_approval_decision`; only the host
   parking target is missing. Happy’s push-on-permission maps to the same envelope.

4. **`PendingApprovalTarget::ClaudeControl { request_id, session_id }` replies over
   the live session stdin**, not a side channel.
   Rejected: reusing `PendingApprovalTarget::Acp` with a fake AcpClient. Reason: wire
   shapes differ (`control_response` vs ACP JSON-RPC result).

5. **Permission policy defaults stay conservative.**
   Launch with interactive permission mode (`manual` / SDK `default`—see flag matrix).
   Do **not** default to `--dangerously-skip-permissions`. Optional profile knobs may
   set `acceptEdits` / `dontAsk` / allow-lists later.
   **Route permissions through stdio control:** pass hidden/internal
   `--permission-prompt-tool stdio` when the CLI accepts it (Agent SDK does this;
   public `--help` may omit it). Probe at capability detect time.

6. **Version-gated features via `system/init.capabilities` when present.**
   Observed on 2.1.222: `interrupt_receipt_v1`, `interrupt_cancel_queued_v1`,
   `msg_lifecycle_v1`. Feature-detect instead of version strings. Fall back
   gracefully (hard-kill interrupt, no subagent text).

7. **Tool completion is a translator correctness fix independent of control plane.**
   Map `tool_result` / completed tool blocks even before full bidirectional approval
   lands, so transcript fidelity improves in Phase 0.

8. **Effort / model catalog honesty.**
   CLI has `--effort` (`low|medium|high|xhigh|max`), but domain SSOT currently sets
   `supports_reasoning_effort = false`. Phase 5 may flip SSOT + catalog only after
   product confirms effort is a first-class Claude control. Until then do not invent
   ladders in UI.

9. **Session continuity uses plane B, not process immortality alone.**
   Live process is best; after death, `--resume <provider_session_id>`. Optional
   catalog browser can list `~/.claude/projects/...` like VS Code / community
   explorers—product phase after control plane.

10. **Running + extra prompt: reject first (product consistency).**
    CLI stream-json can queue messages mid-turn; Minos matches Gemini-style
    “turn already running” unless product later enables queue/steer explicitly.
    Document as intentional—not a protocol limitation.

11. **Stdin lifecycle = session lifecycle.**
    Closing stdin ends the interactive control session (SDK behavior). Keep stdin
    open for the whole Minos session; only close on shutdown. Permission waits must
    not close the write half.

### Target session model

```text
┌──────────────────── Desktop / TUI / Mobile / daemon RPC ───────────────┐
│ start / prompt / interrupt / resolve_approval / resume                    │
└───────────────────────────────┬────────────────────────────────────────┘
                                │
                     AgentManager (Claude arm)
                                │
              ┌─────────────────▼──────────────────┐
              │ ClaudeControlSession (long-lived)  │
              │  - child: claude -p …              │
              │  - stdin  write pump (NDJSON)      │
              │  - stdout read pump (NDJSON)       │
              │  - pending control request map     │
              │  - provider_session_id (plane B)   │
              └───────────────┬────────────────────┘
                              │ RawIngest
                              ▼
                    event_writer / translate_claude
                              │
                              ▼
                       UiEventMessage
                    (text, tools, approval/*)
```

### Launch args (target)

```bash
claude -p \
  --output-format stream-json \
  --input-format stream-json \
  --verbose \
  --include-partial-messages \
  --permission-mode manual \        # interactive; SDK name "default" aliases here
  --permission-prompt-tool stdio \  # when CLI accepts (capability probe)
  --forward-subagent-text \         # if CLI version supports (≥ 2.1.211)
  [--model <id>] \
  [--effort <level>] \              # only after SSOT flip
  [--session-id <uuid> | --resume <uuid-or-name>] \
  [--mcp-config <json> --strict-mcp-config] \
  [--append-system-prompt <minos+profile>]
  # Do NOT default: --dangerously-skip-permissions, --bare
```

#### Flag matrix (research-corrected)

| Flag / concept | CLI / behavior notes | Minos policy |
|----------------|----------------------|--------------|
| `--permission-mode` | Help lists `acceptEdits`, `auto`, `bypassPermissions`, `manual`, `dontAsk`, `plan`. SDK docs use `default`; init events report `permissionMode: "default"`. v2.1.200+ renames default UX to “Manual”; `manual` accepted as alias. | Launch interactive as `manual` (or omit). Accept both in config. Prefer value echoed by `system/init`. |
| `--permission-prompt-tool stdio` | Used by Agent SDKs to route canUseTool over control protocol; often **hidden** from `--help`. | Probe; if accepted, always pass for Desktop-managed sessions. |
| `--input-format stream-json` | Required for bidirectional control + multi-turn stdin. | Required for ClaudeControlSession. |
| `--include-partial-messages` | Token streaming via `stream_event`. | Keep (already on). |
| `--forward-subagent-text` | ≥ 2.1.211; nested text/thinking with `parent_tool_use_id`. | Phase 4 when version supports. |
| `--effort` | `low|medium|high|xhigh|max`. | Phase 5 after domain SSOT flip. |
| `--bare` | Hermetic CI; skips CLAUDE.md/hooks/OAuth keychain; needs API key. | Never default for Desktop; optional CI profile only. |
| `--no-session-persistence` | Disables plane B write. | Avoid for user sessions; breaks resume. |
| `--replay-user-messages` | Echo stdin user msgs on stdout when both stream-json I/O. | Optional debug; not required for v1. |

Notes:

- First user turn: either `-p "<text>"` **or** pure stdin user frame after spawn.
  **Phase 1 freezes one approach.** Prefer: open process with `-p` empty/minimal
  prompt only if required by CLI, then all turns as stdin user messages—matches
  SDK streaming mode and keeps control replies on the same pipe.
- `stdin` is a piped writer, never `Stdio::null()`.
- On hosts that cannot open bidirectional mode (ancient CLI), fall back to current
  one-shot path and surface a capability warning; do not silently claim approval
  support.
- Keep stdin open until session close; flush after every NDJSON line.

### Wire contracts (host view)

Exact field names are frozen in Phase 1 against a captured golden corpus from the
local Claude CLI + Agent SDK sources. Community docs disagree on type names
(`control_request` vs `sdk_control_request`, subtype `can_use_tool` vs `permission`);
**golden against the installed CLI is SSOT**, not blog posts.

Logical contracts:

#### Outbound (Minos → Claude stdin)

| Intent | Logical message |
|--------|-----------------|
| Initialize control / hooks (if required by CLI version) | `control_request` initialize (hooks / sdk_mcp_servers as needed) |
| User turn | user message frame (`type: "user"`, content blocks; `parent_tool_use_id: null`) |
| Permission decision | `control_response` success: `behavior: allow` **with `updatedInput` echo** (required on older CLIs; safe on new) or `behavior: deny` + `message` |
| Interrupt (if supported) | control interrupt / cancel (shape from golden; capabilities advertise support) |
| Shutdown | close stdin; escalate SIGTERM → SIGKILL |

#### Inbound (Claude stdout → Minos)

| Frame family | Host action |
|--------------|-------------|
| `system/hook_*` | Optional Raw; do not treat as turn terminal |
| `system/init` | Session metadata; capture `session_id`, tools, `capabilities`, `permissionMode`, MCP/plugin errors |
| `system/api_retry` | Optional status Raw / UX “retrying…” |
| `stream_event` / `assistant` | Existing translator path |
| `user` with `tool_result` | **New:** `ToolCallCompleted` |
| permission / `can_use_tool` control request | Park `PendingApproval` + emit `approval/request` |
| `AskUserQuestion` via canUseTool path | Park as **question** approval envelope (structured options), not plain Bash modal |
| `result` / `error` | Turn terminal; Idle transition |
| subagent text (`parent_tool_use_id`) | Project as nested transcript / tool children when flag on |

#### Allow response completeness

```json
{
  "type": "control_response",
  "response": {
    "subtype": "success",
    "request_id": "<id-from-request>",
    "response": {
      "behavior": "allow",
      "updatedInput": { "...": "echo tool input" }
    }
  }
}
```

Deny:

```json
{
  "behavior": "deny",
  "message": "User denied this action"
}
```

Optional later: `updatedPermissions` from SDK suggestions for “always allow”
(persists rules)—not required for Phase 2.

### Approval mapping

Claude control permission request → Minos synthetic envelope (same overlay path as
Gemini/Grok / Happy-style mobile push):

```json
{
  "method": "approval/request",
  "params": {
    "request_id": "<minos-or-claude-request-id>",
    "session_id": "<minos session id>",
    "turn_id": "",
    "method": "claude/can_use_tool",
    "params": {
      "tool_name": "Bash",
      "tool_input": { "command": "ls" }
    }
  }
}
```

AskUserQuestion maps to the existing question-shaped approval envelope (mirror
Grok `ask_user_question` / OpenCode question), not a second Claude-only UI.

Parking:

```rust
PendingApprovalTarget::ClaudeControl {
    control_request_id: String,
    // reply written on the live ClaudeControlSession stdin pump
}
```

`resolve_approval`:

1. Validate decision (`allow` / `deny` / optional message) via
   `approvals::validate_claude_control_decision`.
2. Remove pending entry.
3. Write `control_response` to session stdin (flush).
4. Emit durable `approval/resolved` ingest (existing pattern).

Desktop/TUI/Mobile: no Claude-specific modal once envelope is normalized. Update
docs to remove “Claude 未接” after Phase 2 lands.

### Multi-turn, interrupt, resume semantics

| State | Behavior |
|-------|----------|
| Idle + live process | Write next user message on stdin; set Running |
| Running + extra prompt | **Reject** (document; queue is protocol-possible but out of scope v1) |
| Interrupt | Prefer cooperative control cancel when capabilities advertise; fallback hard-kill + synthetic terminal ingest |
| Process exit | Clear `claude_sessions` entry; mark Idle / Closed; keep `provider_session_id` |
| Resume after death | New control session with `--resume <provider_id>`; re-pass mcp-config / system prompt / permission flags (not restored by CLI) |
| Import foreign session (optional) | Read plane B catalog; `--resume <id>` into Minos-owned control session |

Resume after process death keeps today’s provider session id persistence
(`handle.codex_session_id` field reused as provider session id)—rename/document
as agent-agnostic provider id when convenient.

### Translator upgrades

`translate_claude` gains:

1. `tool_result` / completed tool content → `ToolCallCompleted`
2. Passthrough for Minos synthetic `approval/request|timeout|resolved` (mirror
   `gemini.rs` / `grok.rs`)
3. Control permission frames → driver pre-normalizes to `approval/request`; translator
   stays thin
4. Optional: `parent_tool_use_id` text as nested display payload (Phase 4)
5. Tolerate new system subtypes (`hook_*`, `api_retry`, `thinking_tokens`, `status`)
   as Raw without breaking the turn state machine

### Capability flags

| Flag | Source | UI meaning |
|------|--------|------------|
| Claude installed | `list_clis` | Show @claude |
| Bidirectional control | runtime probe (`--input-format stream-json` + stdin open) | Enable approval path |
| Permission stdio routing | probe `--permission-prompt-tool stdio` | Required for interactive approvals in headless |
| Interrupt cooperative | `system/init.capabilities` contains `interrupt_*` | Soft stop vs hard-kill |
| Forward subagent text | CLI flag + version | Nested transcript option |
| Effort | domain SSOT (today false) | Hide effort control until flipped |
| Session catalog browser | product flag | List/resume `~/.claude/projects` sessions |

### Fallback matrix

| CLI capability | Minos behavior |
|----------------|----------------|
| No `--input-format stream-json` | One-shot path; mark `supports_interactive_approval = false` |
| Bidirectional but no permission control frames / no stdio prompt tool | Run tools only via allow-list / permission-mode; surface limitation |
| Kill-only interrupt | Document hard-kill; emit synthetic result if needed |
| No `--forward-subagent-text` | Task remains opaque ToolCall |
| Process dead, id known | `--resume` into new control session |

## Phased Implementation

## Phase 0: Translator fidelity (no control-plane change)

Independently shipable; improves UI even on one-shot sessions.

**File: `crates/minos-ui-protocol/src/claude.rs`**

- Map tool results to `UiEventMessage::ToolCallCompleted` (and failed variants when
  present).
- Pass through `approval/request|timeout|resolved` envelopes as `Raw` (same as
  Gemini/Grok) so later driver work lights up UI without another translator change.
- Tolerate extra `system` subtypes without breaking.
- Add unit tests with golden NDJSON fixtures for `tool_use` → `tool_result` pairs.

**File: `crates/minos-ui-protocol` tests / fixtures**

- Add fixtures under existing test style for Claude tool completion and approval
  passthrough.

Rationale: closes a known high-severity projection hole (`ToolCallCompleted` never
emitted) without waiting on stdin work.

## Phase 1: Protocol freeze + session skeleton

**File: `docs/superpowers/specs/claude-stream-json-control-protocol.md` (new annex)**
or an appendix section once corpus is captured.

Capture golden stdout/stdin from local Claude CLI (`2.1.x`) for:

- init (including `capabilities`, `permissionMode`)
- text / thinking stream
- tool_use / tool_result
- permission / can_use_tool request + allow/deny response (with `updatedInput`)
- AskUserQuestion round-trip if feasible
- interrupt (if capabilities allow)
- result

Pin frame shapes used by Minos (tolerate unknown fields). Record CLI version +
whether `--permission-prompt-tool stdio` is accepted.

**File: `crates/minos-agent-runtime/src/claude_driver.rs`**

- Introduce `ClaudeControlSession` (name can replace `ClaudeNdjsonSession` or wrap it).
- Fields:
  - `session_id`, `workspace`, `provider_session_id`
  - `child: Child`
  - `stdin_tx` (async writer queue; **never drop until close**)
  - stdout/stderr tasks
  - `pending_control: HashMap<String, …>` if needed locally
  - `capabilities: Vec<String>`
- Spawn with piped stdin + `--input-format stream-json` + permission flags.
- Keep one-shot constructor as `ClaudeLegacyNdjsonSession` fallback behind capability
  detection.
- Unit tests with fake CLI scripts (existing manager test pattern for Gemini permission).

**File: `crates/minos-agent-runtime/src/manager.rs`**

- Store `ClaudeControlSession` in `claude_sessions`.
- Split `start_claude_agent` (spawn long-lived) from `start_claude_turn` /
  `send_claude_prompt` (write user message).
- While Running, reject additional prompts (document; queue later if product wants).

## Phase 2: Approval reverse-request path (product critical)

**File: `crates/minos-agent-runtime/src/manager.rs`**

```rust
PendingApprovalTarget::ClaudeControl {
    control_request_id: String,
}
```

- Extend `resolve_approval` match arm to call
  `claude_sessions.get_mut(session_id).reply_control(...)`.

**File: `crates/minos-agent-runtime/src/approvals.rs`**

- Add `validate_claude_control_decision(decision: &Value) -> Result<Value>` producing
  the stdin `control_response` body (`allow` + `updatedInput` / `deny` + message).

**File: `crates/minos-agent-runtime/src/claude_driver.rs`**

- On inbound permission control frame:
  1. Emit normalized `approval/request` ingest.
  2. Insert `PendingApproval { session_id, target: ClaudeControl { … } }`.
- On AskUserQuestion: emit question-shaped envelope.
- On `reply_control`: write NDJSON line to stdin and flush.

**File: Desktop / TUI / Mobile (no new modal)**

- Verify existing ApprovalModal / TUI pending request / mobile attention consume Claude
  `approval/request` without agent-specific branches.
- Update:
  - `docs/architecture-desktop.md` — remove “Claude 未接”
  - `docs/architecture-tui.md` — remove “权限/提问尚未接入”

**Tests**

- Fake `claude` script that emits a permission control request and waits for stdin
  allow response before continuing (mirror
  `gemini_server_permission_request_waits_for_user_decision`).

## Phase 3: Interrupt, lifecycle, multi-turn hardening

**File: `crates/minos-agent-runtime/src/claude_driver.rs` / `manager.rs`**

- Cooperative interrupt when capabilities advertise it; else hard-kill.
- Ensure Drop / close always emits terminal ingest (`result` or synthetic
  `thread_closed` / error) so UI does not stick on Running.
- Resume path: prefer reattach live session; if dead, `--resume <provider_id>` into a
  new control session **and re-pass** mcp-config / system prompt / permission mode.
- Prevent double-child races when replacing sessions.
- Optional safety valves inspired by Buzz: idle silence timeout + max turn wall clock.

**File: `crates/minos-agent-runtime` tests**

- Interrupt mid-turn.
- Two sequential turns on one process.
- Process crash → resume with provider id.

## Phase 4: Subagent visibility + richer projection

**File: `crates/minos-agent-runtime/src/claude_driver.rs`**

- Pass `--forward-subagent-text` when CLI supports it.

**File: `crates/minos-ui-protocol/src/claude.rs`**

- Project messages with `parent_tool_use_id` into nested / linked UI events without
  inventing fake spawn trees if parent tool is Task.

**Docs**

- Update `docs/superpowers/specs/2026-06-22-tui-subagent-display-design.md` note:
  Claude can forward subagent text when flag is on; still not a first-class collab
  spawn graph like Codex.

## Phase 5 (optional): Capability catalog + effort

**File: `crates/minos-domain/src/agent.rs`**

- Only if product wants effort controls: set `supports_reasoning_effort()` true for
  Claude and define honest ladder source (`low|medium|high|xhigh|max`).

**File: `crates/minos-daemon/src/model_catalog.rs`**

- Pass `--effort` from launch config when SSOT allows.
- Keep empty efforts until SSOT flips (do not invent).

**File: `crates/minos-agent-runtime/src/claude_driver.rs`**

- Add `--effort` / richer `--permission-mode` / `--allowedTools` from profile/launch.

## Phase 6 (optional): Session catalog browser (plane B UX)

Inspired by VS Code + community “Claude Sessions Explorer”:

- Enumerate `~/.claude/projects/<encoded-workspace>/` for current workspace.
- Show title / last activity; action “Open in Minos” → start ClaudeControlSession
  with `--resume <id>` + Minos MCP/system prompt policy.
- Clear product warning: Minos-managed permissions/MCP may differ from original CLI
  session flags (CLI does not restore all launch flags).
- Do **not** claim live attach to another process’s stdin (plane B is resume-from-disk,
  not hijack of `~/.claude/sessions/<pid>.json` interactive process).

## Phase 7 (optional): ACP adapter bridge (plane D)

Only if product requires “ACP agent marketplace” uniformity (Zed/Buzz style).

**File: new `claude_acp_bridge` notes / driver variant**

- Spawn `claude-agent-acp` (or pinned `npx @agentclientprotocol/claude-agent-acp`)
  and reuse `AcpClient` + `PendingApprovalTarget::Acp`.
- Keep native stream-json as default; adapter is opt-in profile.
- Document fidelity gap vs native path.

## Phase 8 (optional): IDE bridge (plane C)

Only if product wants terminal-spawned `claude` to use Minos Desktop as the IDE:

- Write `~/.claude/ide/<port>.lock` from Desktop/daemon.
- Loopback WebSocket MCP tools: selection, open files, openDiff.
- Inject env when spawning external terminals—not required for Minos-owned
  ClaudeControlSession (daemon already has workspace FS).

## Phase 9: Verification

- `cargo test -p minos-ui-protocol claude`
- `cargo test -p minos-agent-runtime claude`
- Manual Desktop: `@claude` → tool permission modal → allow → tool completes →
  transcript shows completed tool
- Manual: AskUserQuestion surfaces question UI
- Manual: interrupt mid-turn returns Idle
- Manual: second turn without process leak (`pgrep -fl claude` sanity)
- Manual: kill process → resume same provider id continues context
- Confirm docs no longer claim Claude approval is unwired

## Architectural Notes

- **Semver / public API:** Mostly internal to `minos-agent-runtime` and projection
  crates. Daemon RPC shapes (`minos_local_approval_decision`) stay stable; Claude
  becomes another producer of existing envelopes.
- **No new required third-party runtime** for Phase 0–5 (no Node ACP adapter).
- **Protocol drift risk:** Claude control frames are less formally published than ACP.
  Mitigate with golden fixtures + capability feature detection + tolerant parsers.
  Pin tested CLI minor in CI notes.
- **Security:** Default interactive approvals. Never auto-enable
  `--dangerously-skip-permissions` for Desktop-managed sessions. Allow-lists are
  opt-in profile policy. IDE lockfile (if ever implemented) is user-local 0600 and
  loopback-only—same trust model as VS Code.
- **Auth / billing:** Host continues to use the user’s installed Claude CLI auth.
  Do not embed claude.ai login into Minos. Headless/API-key constraints remain a
  user environment concern (`--bare` only if product explicitly needs hermetic CI
  mode).
- **Teamwork MCP:** Preserve `--mcp-config` + `--strict-mcp-config` +
  `--append-system-prompt` injection; re-pass on resume; verify under bidirectional
  mode.
- **Explicitly NOT changed in core phases:**
  - Gemini/Grok ACP stack
  - Codex app-server
  - Domain agent enum membership
  - Cloud IM wire protocol
- **Why not ACP-first:** Claude’s interactive completeness already exists on stream-json;
  ACP adapter is a compatibility veneer (proven by Zed/Buzz). Minos should own the
  native control plane first, then optionally expose ACP.
- **Why not IDE-bridge-first:** Minos already hosts the agent; plane C helps *external*
  `claude` TUI attach to an editor—not Minos’s primary remote-control path.

## File Change Summary

- `crates/minos-agent-runtime/src/approvals.rs` -- Claude control decision validator
- `crates/minos-agent-runtime/src/claude_driver.rs` -- bidirectional control session;
  stdin pump; permission parking; interrupt; launch flags
- `crates/minos-agent-runtime/src/manager.rs` -- `PendingApprovalTarget::ClaudeControl`;
  long-lived Claude session lifecycle; resolve_approval arm; multi-turn/interrupt
- `crates/minos-agent-runtime/src/*tests*` -- fake CLI permission / multi-turn /
  interrupt coverage
- `crates/minos-daemon/src/model_catalog.rs` -- optional effort/model honesty (Phase 5)
- `crates/minos-domain/src/agent.rs` -- optional effort SSOT flip (Phase 5)
- `crates/minos-ui-protocol/src/claude.rs` -- ToolCallCompleted; approval passthrough;
  subagent parent ids
- `docs/architecture-desktop.md` -- remove Claude approval gap once Phase 2 done
- `docs/architecture-shared-crates.md` -- document Claude control session (not
  one-shot NDJSON only)
- `docs/architecture-tui.md` -- remove Claude permission gap once Phase 2 done
- `docs/superpowers/specs/claude-full-fidelity-design.md` -- this design
- `docs/superpowers/specs/claude-stream-json-control-protocol.md` -- Phase 1 annex
  (golden wire shapes)
- `docs/superpowers/specs/2026-06-22-tui-subagent-display-design.md` -- amend Claude
  subagent note after Phase 4

## Appendix A: Why current support feels low

| Symptom | Root cause |
|---------|------------|
| No permission modal | `stdin = null` + no control parser + no `PendingApprovalTarget::Claude*` |
| Tools look stuck | Translator never emits `ToolCallCompleted` |
| Multi-turn flaky / heavy | New process per turn + `--resume` instead of live stdin turns |
| Interrupt crude | Hard-kill only |
| Docs say “Claude 未接” | Product-visible name for the approval gap, not “agent missing” |
| “VS Code can see my sessions” envy | Plane B catalog + official extension UI; Minos never read `~/.claude/projects` |

## Appendix B: Research references

### Official

- Claude headless / programmatic: https://code.claude.com/docs/en/headless
- Claude Agent SDK overview: https://code.claude.com/docs/en/agent-sdk/overview
- Claude Agent SDK permissions: https://code.claude.com/docs/en/agent-sdk/permissions
- Approvals / canUseTool / AskUserQuestion: https://code.claude.com/docs/en/agent-sdk/user-input
- Streaming vs single input: https://code.claude.com/docs/en/agent-sdk/streaming-vs-single-mode
- Session manage / resume: https://code.claude.com/docs/en/sessions
- VS Code extension: https://code.claude.com/docs/en/vs-code

### Ecosystem

- ACP agents list (Claude via adapter): https://agentclientprotocol.com/get-started/agents
- Claude ACP adapter: https://github.com/agentclientprotocol/claude-agent-acp
- Zed Claude via ACP: https://zed.dev/blog/claude-code-via-acp
- Zed external agents: https://zed.dev/docs/ai/external-agents
- Happy (mobile/web companion): https://github.com/slopus/happy
- Buzz ACP harness (sibling `../buzz`, `crates/buzz-acp`): Claude via `claude-agent-acp`
- Community CLI protocol notes (reference, not SSOT):
  https://github.com/Roasbeef/claude-agent-sdk-go/blob/main/docs/cli-protocol.md
- IDE bridge pattern (Nova docs mirror VS Code):
  lockfile `~/.claude/ide/<port>.lock` + MCP over loopback WS

### Local verification (2026-08-07)

- Claude Code **2.1.222**
- `system/init.capabilities`: `interrupt_receipt_v1`, `interrupt_cancel_queued_v1`,
  `msg_lifecycle_v1`
- Session storage: `~/.claude/projects/<cwd-key>/<session-id>.jsonl`
- Live registry: `~/.claude/sessions/<pid>.json`
- IDE locks present for VS Code workspaces under `~/.claude/ide/*.lock`

## Appendix C: Decision summary for implementers

```text
Do:
  - Open stdin + --input-format stream-json (plane A)
  - Probe/pass --permission-prompt-tool stdio when available
  - Park Claude permissions on existing approval UI
  - Echo updatedInput on allow
  - Fix tool completion projection
  - Prefer long-lived session; resume via plane B after death
  - Re-pass mcp-config / system prompt / permission mode on --resume
  - Feature-detect capabilities from system/init

Don't:
  - Block on native Claude ACP/app-server
  - Make claude-agent-acp the default Minos path
  - Default to dangerously-skip-permissions or --bare
  - Invent effort ladders before domain SSOT flip
  - Spawn a second Claude process while one turn is Running
  - Close stdin while a permission is pending
  - Treat IDE lockfile (plane C) as a substitute for control plane (A)
  - Claim live hijack of another interactive claude PID as "resume"
```

## Appendix D: Borrow checklist (ecosystem → Minos)

| Source | Pattern | Minos phase |
|--------|---------|-------------|
| Agent SDK / headless | Streaming input + canUseTool | Phase 1–2 |
| VS Code | Session catalog + resume UX | Phase 6 optional |
| VS Code IDE | Lockfile MCP editor tools | Phase 8 optional |
| Happy | Host-owned process + push on permission | Phase 2 + mobile attention |
| Happy | Desktop/mobile handoff | Already: multi-client, single daemon session |
| Zed / Buzz | ACP adapter uniformity | Phase 7 optional |
| Buzz | Idle + max-turn safety valves | Phase 3 |
| Buzz | PermissionMode vocabulary | Launch profile knobs |
| Community explorers | Read-only `~/.claude/projects` browser | Phase 6 |

## Appendix E: Comparison — Minos vs peers (target after Phase 0–3)

| Capability | VS Code ext | Zed ACP | Happy | Buzz+adapter | Minos today | Minos target |
|------------|-------------|---------|-------|--------------|-------------|--------------|
| Stream text/tools | Yes | Yes | Yes | Yes | Partial | Yes |
| Tool completed UX | Yes | Yes | Yes | Yes | No | Yes |
| Interactive permission | Yes | Yes (ACP) | Yes + push | Yes (ACP) | No | Yes |
| Long-lived multi-turn | Yes | Yes | Yes | Yes | Weak (`--resume` per turn) | Yes |
| Cooperative interrupt | Yes | Yes | Yes | Yes | Kill only | Yes |
| Resume local sessions | Yes (catalog) | Limited | Session handoff | Per-channel ACP session | Provider id only | Yes + optional catalog |
| Native stream-json SSOT | Internal | No (adapter) | Wrapper | No (adapter) | Outbound only | Full duplex |
| Pure Rust host | N/A | N/A | No | Partial (harness) | Yes | Yes |
