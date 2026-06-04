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

```
┌─ Status ── [● codex ✓] [● claude ✓] [○ gemini ✗] [○ opencode ✗] ── Connected ─┐
├─ Threads ────────────┬─ Chat: codex #abc123 ───────────────────────────────────┤
│ > codex  Running ●   │                                                        │
│   claude Idle   ○    │  [You]                                                  │
│                      │  帮我写一个 Rust 的 hello world                          │
│                      │                                                        │
│                      │  [Codex]                                                │
│                      │  好的，我来帮你创建一个...                               │
│                      │    🔧 write_file                                        │
│                      │    ┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄                              │
│                      │    ✅ completed                                         │
│                      │  文件已创建在 hello.rs                                   │
│                      │                                                        │
│                      │  ▓▓ (streaming)                                         │
├──────────────────────┴────────────────────────────────────────────────────────┤
│ > _                                                                            │
└────────────────────────────────────────────────────────────────────────────────┘
```

### Panel Breakdown

| Panel | Position | Width | Content |
|-------|----------|-------|---------|
| Status Bar | Top | 100% | Detected CLI status (green ✓ / red ✗), connection state label |
| Thread List | Left | 25% | Active threads with agent name + status indicator + thread_id prefix |
| Chat Panel | Right | 75% | UiEventMessage rendering for selected thread |
| Input Bar | Bottom | 100% | Single-line input (multi-line via Shift+Enter) |

### Key Bindings

| Key | Action |
|-----|--------|
| `↑/↓` | Navigate thread list |
| `Enter` | Select thread / confirm dialog |
| `n` | New thread (agent picker) |
| `Enter` (in input) | Send message |
| `Shift+Enter` | New line in input |
| `Ctrl+C` | Interrupt current turn |
| `Ctrl+D` | Close current thread |
| `Ctrl+Q` / `Esc` | Quit |
| `PgUp/PgDn` | Scroll chat history |
| `Tab` | Toggle focus between thread list and chat |
| `e` | Expand/collapse tool call details |

## 6. Message Rendering Rules

| UiEventMessage Variant | Rendering |
|------------------------|-----------|
| `ThreadOpened` | System notice in chat (thin line separator) |
| `MessageStarted` (User) | Blue left-aligned label `[You]` |
| `MessageStarted` (Assistant) | Green left-aligned label `[Agent]` |
| `TextDelta` | Append to current message text; show blinking cursor ▓ when streaming |
| `ReasoningDelta` | Gray italic block, collapsible |
| `ToolCallPlaced` | 🔧 + tool name + folded args |
| `ToolCallCompleted` | ✅ or ❌ + output summary (3 lines max, expandable) |
| `MessageCompleted` | Remove streaming cursor |
| `Error` | Red banner across chat width |
| `Raw` | Yellow monospace block |
| `ThreadClosed` | System notice with reason |
| `ThreadTitleUpdated` | Update tab/thread title |

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

## 10. New Thread Flow

1. User presses `n` → agent picker popup shows only installed agents (from `detect_clis()`)
2. User selects an agent → workspace input (defaults to current directory)
3. Call `backend.start_agent(agent, workspace)`
4. Thread appears in thread list with `Running` or `Starting` state
5. Auto-switch to new thread
6. Input bar gains focus

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
