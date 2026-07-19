# Minos TUI Design Spec

> Date: 2026-06-04
> Status: Draft
> Author: Peter Rich

## 1. Problem

Minos supports four agent CLIs (Codex, Claude Code, Gemini, Opencode) on the host side, but debugging and previewing their output requires running the macOS menu-bar app. This makes iteration slow and prevents usage on Linux. We need a terminal-based interface that can be launched directly from a local shell without depending on the macOS app or the relay/backend.

## 2. Scope

Build a `minos-tui` crate that:

- Embeds `AgentManager` to spawn and control agent CLIs locally
- Translates `RawIngest` events into `UiEventMessage` via the existing `minos-ui-protocol` translators
- Renders a multi-thread chat TUI using ratatui
- Supports sending messages, interrupting turns, and closing threads
- Runs on macOS and Linux (any terminal with crossterm support)

Out of scope for the initial implementation:

- Connecting to a running minos-daemon via RPC (trait is defined but no implementation)
- Approval requests handling (Codex-specific; deferred)
- Thread history persistence (TUI is ephemeral; restart loses history)
- Relay/backend connectivity

## 3. Architecture

### 3.1 Crate Structure

```
crates/minos-tui/
├── Cargo.toml
├── src/
│   ├── main.rs              # CLI entry (clap), tokio runtime, App launch
│   ├── app.rs               # App state machine + event loop (ratatui main)
│   ├── backend/
│   │   ├── mod.rs           # AgentBackend trait definition
│   │   └── embedded.rs      # EmbeddedBackend (wraps AgentManager)
│   ├── ui/
│   │   ├── mod.rs           # ratatui component composition
│   │   ├── thread_list.rs   # Left panel: active thread list
│   │   ├── chat.rs          # Right panel: message rendering
│   │   ├── input_bar.rs     # Bottom: text input
│   │   ├── status_bar.rs    # Top: CLI detection status + connection state
│   │   └── theme.rs         # Color/style theme
│   ├── event.rs             # Unified AppEvent enum (terminal + backend)
│   └── translation.rs       # RawIngest → UiEventMessage routing
```

### 3.2 Dependencies

| Dependency | Purpose |
|------------|---------|
| `minos-agent-runtime` | AgentManager, RawIngest, ThreadState, ManagerEvent |
| `minos-ui-protocol` | UiEventMessage, per-agent translators + state |
| `minos-domain` | AgentName, AgentDescriptor, MinosError |
| `minos-cli-detect` | detect_clis() for status bar |
| `ratatui` | TUI framework |
| `crossterm` | Terminal backend (ratatui compatible) |
| `tokio` | Async runtime |
| `clap` | CLI argument parsing |
| `humantime` | Duration parsing for CLI args |

Not depended upon: `minos-daemon`, `minos-transport`, `minos-protocol`, `minos-backend`.

## 4. AgentBackend Trait

```rust
#[async_trait]
pub trait AgentBackend: Send + Sync {
    async fn detect_clis(&self) -> Result<Vec<AgentDescriptor>>;
    async fn start_agent(&self, agent: AgentName, workspace: PathBuf) -> Result<StartAgentOutcome>;
    async fn send_message(&self, thread_id: &str, text: &str) -> Result<()>;
    async fn interrupt_thread(&self, thread_id: &str) -> Result<()>;
    async fn close_thread(&self, thread_id: &str) -> Result<()>;
    async fn list_threads(&self) -> Result<Vec<ThreadSnapshot>>;
    async fn subscribe_ingest(&self) -> broadcast::Receiver<RawIngest>;
    async fn subscribe_manager_events(&self) -> broadcast::Receiver<ManagerEvent>;
}
```

### EmbeddedBackend

Holds `Arc<AgentManager>` constructed with `AgentRuntimeConfig`. Every method delegates directly to `AgentManager`'s corresponding API. Constructed in `main.rs` based on CLI arguments.

Future `DaemonRpcBackend` will implement the same trait by connecting to the daemon's JSON-RPC server.

## 5. UI Layout

The interaction model is **not** “one agent thread = one user conversation”. The primary object is a **chat room thread** that aggregates multiple agents. Individual agent transcripts are secondary detail views.

### 5.1 Overview Mode

```
┌─ Status ───────────────────────────────────────────────────────────────────────┐
├─ Threads ────────────┬─ Chat Room: thread #abc123 ──────────────┬─ Agents ────┤
│ > launch-planning    │ [You]                                     │ > codex     │
│   release-cut        │ @codex summarize the current blockers     │   claude    │
│   ci-triage          │                                           │   gemini    │
│                      │ [Codex]                                   │             │
│                      │ The current blockers are...               │             │
│                      │                                           │             │
│                      │ [Claude]                                  │             │
│                      │ I found one extra risk in...              │             │
├──────────────────────┴───────────────────────────────────────────┴─────────────┤
│ Chat room input: @agent route or send a room message                            │
└──────────────────────────────────────────────────────────────────────────────────┘
```

### 5.2 Agent Detail Mode

When the user selects an agent from the right column, the TUI switches from room overview to agent inspection. The thread list is hidden temporarily because the user is already “inside” one chat room thread.

```
┌─ Status ───────────────────────────────────────────────────────────────────────┐
├─ Chat Room: thread #abc123 ──────────────┬─ Agents ─────┬─ Agent Detail ──────┤
│ [You]                                    │ > codex      │ [You]                │
│ @codex summarize the current blockers    │   claude     │ Please rewrite as... │
│                                          │   gemini     │                      │
│ [Codex]                                  │              │ [Agent]              │
│ The current blockers are...              │              │ I inspected the repo │
│                                          │              │ and updated...       │
├──────────────────────────────────────────┴──────────────┴──────────────────────┤
│ Chat room input                          │ Agent detail input                   │
└──────────────────────────────────────────────────────────────────────────────────┘
```

Pressing `Esc` while the agent-detail pane is focused closes that pane and returns to overview mode.

### 5.3 Panel Breakdown

| Panel | Mode | Role |
|-------|------|------|
| Status Bar | Both | CLI detection status, backend state, high-level key hints |
| Thread List | Overview only | Lists chat room threads, not agent execution sessions |
| Chat Room Transcript | Both | Shows user messages plus agent result messages relevant to the room |
| Agent List | Both | Lists agents participating in the selected chat room |
| Agent Detail Transcript | Agent detail only | Full execution transcript for the selected agent |
| Chat Room Input | Both | Send a room message or route with `@agent` |
| Agent Detail Input | Agent detail only | Talk directly to the selected agent |

### 5.4 Key Bindings

| Key | Action |
|-----|--------|
| `↑/↓` | Navigate the focused list or move inside multiline input |
| `←/→` | Move cursor inside input, or navigate within the focused pane where applicable |
| `Enter` | Select thread / select agent / send input / confirm dialog |
| `Shift+Enter` | New line in input |
| `Tab` | Cycle focus between visible panes |
| `Esc` | Cancel picker, step focus back, or close agent detail pane |
| `PgUp/PgDn` | Scroll transcript history |
| `Home/End` | Jump to line start/end in input or transcript edge in scrollable panes |
| `Ctrl+A` / `Ctrl+E` | Move input cursor to line start/end |
| `Ctrl+W` / `Alt+D` | Delete previous / next word in input |
| `Alt+B` / `Alt+F` | Move input cursor by word |
| `Ctrl+C` | Interrupt the selected running agent thread |
| `Ctrl+D` | Close current agent thread |

## 6. Message Rendering Rules

| UiEventMessage Variant | Rendering |
|------------------------|-----------|
| Chat room user message | Blue left-aligned label `[You]` in the room transcript |
| Chat room agent result | Green left-aligned label `[Agent]` in the room transcript, only showing the result-level content relevant to the room |
| Agent detail `MessageStarted` / `TextDelta` | Full assistant/user transcript in the agent-detail pane |
| `ReasoningDelta` | Shown only in the agent-detail pane |
| `ToolCallPlaced` / `ToolCallCompleted` | Shown only in the agent-detail pane |
| `MessageCompleted` | Removes the streaming cursor / pending state from the agent-detail pane |
| `Error` | Room-level summary in the chat room transcript, with full details in agent detail |
| `Raw` | Suppressed from the room transcript; may be exposed in agent detail for debugging |
| `ThreadClosed` | System notice with reason in the relevant pane |
| `ThreadTitleUpdated` | Update the chat room thread label |

### Code Block Detection

Text content matching `` ```lang ... ``` `` patterns is rendered as a separate styled block with a subtle border and syntax-hint label.

## 7. Event Loop & Data Flow

### AppEvent (unified event enum)

```rust
enum AppEvent {
    Ingest(RawIngest),
    ManagerEvent(ManagerEvent),
    Key(KeyEvent),
    Resize(u16, u16),
    Tick,
}
```

### Event Loop Architecture

Three async pumps feed into a single `mpsc::Receiver<AppEvent>`:

1. **Ingest pump**: subscribes to `backend.subscribe_ingest()`, forwards as `AppEvent::Ingest`
2. **State pump**: subscribes to `backend.subscribe_manager_events()`, forwards as `AppEvent::ManagerEvent`
3. **Terminal pump**: crossterm `EventStream`, maps key/resize to `AppEvent::Key` / `AppEvent::Resize`

A 200ms `Tick` event drives the streaming cursor blink and periodic redraw.

### Data Flow Path

```
Agent CLI process
  │
  ▼
AgentManager (spawns + manages process, speaks native protocol)
  │
  ▼ RawIngest (broadcast channel)
ChatState.translation_state.translate(payload) (per-thread per-agent stateful translator)
  │
  ▼ Vec<UiEventMessage>
ChatState.apply_ui_events() (updates RenderedMessage list)
  │
  ▼
ratatui render (on Tick or after event processing)
```

## 8. Translation

Per-thread translation state lives directly in `ChatState`. The `minos-ui-protocol` crate provides stateful translator functions (`translate_codex`, `translate_claude`, `translate_gemini`, `translate_opencode`) each taking a `&mut XxxTranslatorState` and a `&RawIngest` payload, returning `Vec<UiEventMessage>`.

```rust
enum AgentTranslationState {
    Codex(CodexTranslatorState),
    Claude(ClaudeTranslatorState),
    Gemini(GeminiTranslatorState),
    Opencode(OpencodeTranslatorState),
}

impl AgentTranslationState {
    fn translate(&mut self, payload: &serde_json::Value) -> Vec<UiEventMessage> {
        match self {
            Self::Codex(s) => translate_codex(s, payload),
            Self::Claude(s) => translate_claude(s, payload),
            Self::Gemini(s) => translate_gemini(s, payload),
            Self::Opencode(s) => translate_opencode(s, payload),
        }
    }
}
```

## 9. ChatState

One per active thread. Owns its own translation state so stateful translators accumulate correctly across the thread's lifetime.

```rust
pub struct ChatState {
    thread_id: String,
    agent: AgentName,
    translation_state: AgentTranslationState,
    messages: Vec<RenderedMessage>,
    scroll_offset: u16,
    auto_scroll: bool,
}

pub struct RenderedMessage {
    message_id: String,
    role: MessageRole,
    text_parts: Vec<TextPart>,
    tool_calls: Vec<ToolCallBlock>,
    reasoning: Option<String>,
    is_streaming: bool,
    error: Option<String>,
}

pub enum TextPart {
    Plain(String),
    Code { lang: String, code: String },
}

pub struct ToolCallBlock {
    tool_call_id: String,
    name: String,
    args_summary: String,
    output_summary: Option<String>,
    is_error: bool,
    is_expanded: bool,
}
```

## 10. Thread Flow

For the next iteration, thread creation is not the main design problem. The TUI may start with a default initial chat room thread and defer explicit “new thread” affordances until the room/agent-detail split is solid.

The important flow is:

1. User focuses a chat room thread from the left list.
2. User sends a room message or `@agent` routed message through the chat room input.
3. The room transcript shows only result-level messages from participating agents.
4. The right column lists available agents for that room.
5. Selecting an agent opens agent-detail mode and reveals that agent’s full transcript plus a dedicated direct-input box.
6. `Esc` from agent detail closes that pane and restores the thread list.

## 11. CLI Interface

```bash
# Interactive launch (pick agent in TUI)
minos-tui

# Start with a specific agent and workspace
minos-tui --agent codex --workspace ~/projects/my-app

# Read-only mode (no input bar)
minos-tui --readonly

# Agent runtime configuration
minos-tui --max-instances 4 --idle-timeout 10m
```

```rust
#[derive(Parser)]
#[command(name = "minos-tui", about = "Minos Agent TUI - local debug console")]
struct Cli {
    #[arg(short, long)]
    agent: Option<AgentName>,

    #[arg(short, long, default_value = ".")]
    workspace: PathBuf,

    #[arg(long)]
    readonly: bool,

    #[arg(long)]
    max_instances: Option<usize>,

    #[arg(long)]
    idle_timeout: Option<humantime::Duration>,
}
```

## 12. Error Handling

| Error Source | Display | Recovery |
|-------------|---------|----------|
| Agent CLI not installed | Status bar red ✗ | Agent not selectable in picker |
| Agent process crash | Red banner in chat + ThreadState → Closed(Crashed) | User can create new thread |
| AgentManager internal error | Status bar flash red for 3 seconds | Auto-clears |
| RawIngest parse failure | Rendered as Raw UiEventMessage (yellow block) | Graceful degradation, no crash |
| Terminal resize | Auto re-layout | No action needed |
| broadcast channel lag | Discard old events, continue from latest state | Possible gap in chat (acceptable for debug tool) |

## 13. Shutdown Flow

1. `Ctrl+Q` or `Esc` → if active threads exist, show confirmation dialog
2. On confirm: call `backend.close_thread()` for each active thread
3. `AgentManager` Drop sends SIGTERM → 3s grace → SIGKILL to all child processes
4. Restore terminal to original mode via crossterm
5. Exit with code 0

## 14. Testing Strategy

- **Unit tests**: TranslationRouter (feed known RawIngest payloads, assert UiEventMessage output), ChatState event application logic, RenderedMessage assembly
- **Integration tests**: EmbeddedBackend creation with mock AgentManager, event pump lifecycle
- **Manual smoke test**: `cargo run -p minos-tui -- --agent codex` in a real workspace

No snapshot tests for TUI rendering (too brittle across terminal sizes). Visual verification is manual.
