# MCP Unix Socket IPC 重构实现计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 将 MCP Server 与 Minos 主进程之间的通信从 SQLite DB 轮询改为 Unix Domain Socket 实时通信，同时将子命令从 `chat-mcp` 重命名为 `minos-mcp`。

**Architecture:** Minos 主进程（TUI/Daemon）启动时创建一个 Unix Domain Socket 监听器。每个 Agent 的 MCP Server 子进程连接到此 Socket。MCP Server 从 stdin 接收 Agent 的 JSON-RPC 请求后，对需要主进程执行的 tool call（`request_agent_help`、`mention_user`），通过 Socket 转发给主进程处理并同步等待结果；对纯读操作（`list_chat_messages`），MCP Server 自行通过共享的 ChatStore（Socket 连接时传递 DB 路径）查询。主进程退出后 Socket 关闭，MCP Server 检测到断连后自动退出。

**Tech Stack:** Rust, tokio (UnixListener/UnixStream), serde_json, JSON-RPC 2.0 over Unix Socket

---

## 文件变更总览

| 操作 | 文件 | 职责 |
|------|------|------|
| **新建** | `crates/minos-chat-store/src/mcp_socket.rs` | Socket IPC 协议定义：请求/响应类型、帧编解码 |
| **新建** | `crates/minos-chat-store/src/mcp_handler.rs` | 主进程侧 Socket 请求处理器（处理 tool call，调用 App 层回调） |
| **新建** | `crates/minos-chat-store/src/mcp_server.rs` | 新 MCP Server 实现（stdio ↔ socket 代理） |
| **删除** | `crates/minos-chat-store/src/bin/minos-chat-mcp.rs` | 旧的独立 MCP binary |
| **新建** | `crates/minos-chat-store/src/bin/minos-mcp.rs` | 新的独立 MCP binary（`minos-mcp`） |
| **重写** | `crates/minos-chat-store/src/mcp.rs` | 保留 serve_stdio 入口但改为 socket 代理模式，移除旧的 DB 写入逻辑 |
| **重写** | `crates/minos-chat-store/src/lib.rs` | 移除 `chat_mcp_commands` 表、`ChatMcpCommand`/`ChatMcpCommandStatus`/`NewChatMcpCommand`/`ChatMcpCommandKind` 类型及相关 DB 方法 |
| **重写** | `crates/minos-chat-store/Cargo.toml` | 新增 `[[bin]] name = "minos-mcp"` |
| **修改** | `crates/minos-agent-runtime/src/config.rs` | `ChatMcpConfig` 改为 `McpConfig`，新增 socket_path 字段，`server_bin` 改为 `minos-mcp` |
| **修改** | `crates/minos-agent-runtime/src/manager.rs` | 更新 MCP Server 解析逻辑，传递 `--socket-path` 替代 `--db-path`/`--room-id` |
| **修改** | `crates/minos-tui/src/main.rs` | 子命令 `chat-mcp` → `minos-mcp`，CLI flags 重命名，移除 `--db-path`/`--room-id`，新增 `--socket-path` |
| **修改** | `crates/minos-tui/src/backend/embedded.rs` | 启动 Socket 监听器，传入 socket_path 给 MCP 配置 |
| **重写** | `crates/minos-tui/src/group_chat.rs` | 移除 `claim_pending_mcp_commands`/`complete_mcp_command`/`fail_mcp_command` |
| **重写** | `crates/minos-tui/src/app.rs` | 移除 `process_pending_mcp_commands`/`process_mcp_command` 轮询逻辑，改为 Socket handler 回调触发 `dispatch_prompt_to_agent` |
| **修改** | `crates/minos-daemon/src/agent.rs` | 更新 MCP 配置，传入 socket_path |
| **修改** | `crates/minos-agent-runtime/src/claude_driver.rs` | 更新测试中的 `minos-chat-mcp` 引用 |
| **修改** | `crates/minos-acp-protocol/src/types.rs` | 更新测试中的 `chat-mcp` 引用 |
| **修改** | 各文件测试 | 更新所有受影响的测试 |

---

## Task 1: 定义 Socket IPC 协议类型

**Files:**
- Create: `crates/minos-chat-store/src/mcp_socket.rs`

这是所有后续任务的基础类型定义。

- [ ] **Step 1: 创建 `mcp_socket.rs` 协议模块**

```rust
// crates/minos-chat-store/src/mcp_socket.rs

use std::io::{self, Read, Write};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

const FRAME_HEADER_LEN: usize = 4;
const MAX_FRAME_LEN: u32 = 4 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "method", rename_all = "snake_case")]
pub enum SocketRequest {
    ListChatMessages {
        room_id: String,
        before_seq: Option<u64>,
        limit: Option<u32>,
    },
    RequestAgentHelp {
        room_id: String,
        source_agent: Option<String>,
        target_agent: String,
        prompt: String,
    },
    MentionUser {
        room_id: String,
        source_agent: Option<String>,
        message: String,
    },
    Ping,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum SocketResponse {
    Ok {
        #[serde(skip_serializing_if = "Option::is_none")]
        data: Option<serde_json::Value>,
    },
    Error {
        message: String,
    },
    Pong,
}

pub fn encode_frame(value: &SocketResponse) -> Result<Vec<u8>> {
    let payload = serde_json::to_vec(value).context("failed to serialize socket frame")?;
    let len = u32::try_from(payload.len()).context("socket frame payload too large")?;
    let mut buf = Vec::with_capacity(FRAME_HEADER_LEN + payload.len());
    buf.extend_from_slice(&len.to_be_bytes());
    buf.extend_from_slice(&payload);
    Ok(buf)
}

pub fn read_frame<R: Read>(reader: &mut R) -> Result<Option<SocketRequest>> {
    let mut header = [0u8; FRAME_HEADER_LEN];
    match reader.read_exact(&mut header) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(e.into()),
    }
    let len = u32::from_be_bytes(header);
    anyhow::ensure!(len <= MAX_FRAME_LEN, "socket frame too large: {len} bytes");
    let mut payload = vec![0u8; len as usize];
    reader.read_exact(&mut payload)?;
    let request: SocketRequest =
        serde_json::from_slice(&payload).context("failed to deserialize socket frame")?;
    Ok(Some(request))
}

pub fn write_response<W: Write>(writer: &mut W, response: &SocketResponse) -> Result<()> {
    let frame = encode_frame(response)?;
    writer.write_all(&frame)?;
    writer.flush()?;
    Ok(())
}
```

- [ ] **Step 2: 在 `lib.rs` 中注册模块**

在 `crates/minos-chat-store/src/lib.rs` 中添加 `pub mod mcp_socket;` 声明（在已有的 `pub mod mcp;` 旁边）。

- [ ] **Step 3: 编译验证**

Run: `cargo check -p minos-chat-store`
Expected: 编译通过

- [ ] **Step 4: Commit**

```bash
git add crates/minos-chat-store/src/mcp_socket.rs crates/minos-chat-store/src/lib.rs
git commit -m "feat(chat-store): add Unix socket IPC protocol types for MCP"
```

---

## Task 2: 重写 MCP Server — stdio ↔ Socket 代理

**Files:**
- Create: `crates/minos-chat-store/src/mcp_server.rs`
- Create: `crates/minos-chat-store/src/bin/minos-mcp.rs`
- Modify: `crates/minos-chat-store/Cargo.toml`

新 MCP Server 的职责：
1. 接收 Agent 的 stdio JSON-RPC（MCP 协议）
2. 对 `list_chat_messages`：直接通过 DB 查询（自己打开 ChatStore）
3. 对 `request_agent_help` / `mention_user`：通过 Unix Socket 转发给主进程，同步等待结果
4. 维持心跳：定期 ping 主进程 Socket，连续失败 N 次后退出

- [ ] **Step 1: 创建 `mcp_server.rs`**

```rust
// crates/minos-chat-store/src/mcp_server.rs

use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result};
use minos_domain::AgentName;
use serde_json::{json, Map, Value};

use crate::mcp_socket::{SocketRequest, SocketResponse};
use crate::ChatStore;

const SERVER_INSTRUCTIONS: &str = "This MCP server exposes the Minos teamwork chat room bound to the current agent session. Use list_chat_messages to read recent room history before answering when the user or another agent refers to chat context, coordination, previous replies, or current room state. Use request_agent_help to ask another Minos agent in the same room for focused help. Use mention_user only for concise user-visible updates that should appear in the room.";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct McpToolPermissions {
    pub read_chat: bool,
    pub mention_agent: bool,
    pub mention_user: bool,
}

impl Default for McpToolPermissions {
    fn default() -> Self {
        Self {
            read_chat: true,
            mention_agent: true,
            mention_user: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpServerConfig {
    pub socket_path: PathBuf,
    pub db_path: PathBuf,
    pub room_id: String,
    pub source_agent: Option<AgentName>,
    pub permissions: McpToolPermissions,
}

pub async fn serve_stdio(config: McpServerConfig) -> Result<()> {
    let store = ChatStore::open(&config.db_path).await?;
    let socket_path = config.socket_path.clone();
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout();

    for line in stdin.lock().lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let request: Value = match serde_json::from_str(&line) {
            Ok(value) => value,
            Err(error) => {
                write_json(
                    &mut stdout,
                    &json!({
                        "jsonrpc": "2.0",
                        "id": null,
                        "error": {"code": -32700, "message": error.to_string()}
                    }),
                )?;
                continue;
            }
        };
        let Some(id) = request.get("id").cloned() else {
            continue;
        };
        let response = handle_request(&store, &config, &socket_path, id, request).await;
        write_json(&mut stdout, &response?)?;
    }
    Ok(())
}

async fn handle_request(
    store: &ChatStore,
    config: &McpServerConfig,
    socket_path: &Path,
    id: serde_json::Value,
    request: serde_json::Value,
) -> Result<serde_json::Value> {
    let method = request
        .get("method")
        .and_then(Value::as_str)
        .context("MCP request missing method")?;
    match method {
        "initialize" => Ok(json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": {
                "protocolVersion": request
                    .get("params")
                    .and_then(|params| params.get("protocolVersion"))
                    .and_then(Value::as_str)
                    .unwrap_or("2025-06-18"),
                "capabilities": { "tools": {} },
                "serverInfo": {
                    "name": "minos-mcp",
                    "version": env!("CARGO_PKG_VERSION")
                },
                "instructions": SERVER_INSTRUCTIONS
            }
        })),
        "tools/list" => Ok(json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": { "tools": tools_for_permissions(config.permissions) }
        })),
        "tools/call" => {
            let params = request.get("params").cloned().unwrap_or_else(|| json!({}));
            match handle_tool_call(store, config, socket_path, id.clone(), params).await {
                Ok(response) => Ok(response),
                Err(error) => Ok(error_response(id, -32602, &error.to_string())),
            }
        }
        "ping" => Ok(json!({ "jsonrpc": "2.0", "id": id, "result": {} })),
        _ => Ok(error_response(id, -32601, &format!("unsupported MCP method: {method}"))),
    }
}

async fn handle_tool_call(
    store: &ChatStore,
    config: &McpServerConfig,
    socket_path: &Path,
    id: serde_json::Value,
    params: serde_json::Value,
) -> Result<serde_json::Value> {
    let name = params.get("name").and_then(Value::as_str).unwrap_or_default();
    let args = params.get("arguments").cloned().unwrap_or_else(|| json!({}));

    match name {
        "list_chat_messages" => {
            ensure_tool_enabled(config.permissions.read_chat, "list_chat_messages")?;
            let room_id = bound_room_id(&args, config, "list_chat_messages")?;
            let before_seq = args.get("before_seq").and_then(Value::as_u64);
            let limit = args
                .get("limit")
                .and_then(Value::as_u64)
                .and_then(|value| u32::try_from(value).ok());
            let page = store.list_messages_desc(room_id, before_seq, limit).await?;
            tool_response(id, serde_json::to_value(&page)?)
        }
        "request_agent_help" => {
            ensure_tool_enabled(config.permissions.mention_agent, "request_agent_help")?;
            let room_id = bound_room_id(&args, config, "request_agent_help")?;
            let target_agent = parse_agent_arg(required_string_arg(&args, "target_agent")?)?;
            let prompt = required_string_arg(&args, "prompt")?.trim().to_owned();
            anyhow::ensure!(!prompt.is_empty(), "prompt must not be empty");
            let request = SocketRequest::RequestAgentHelp {
                room_id: room_id.to_owned(),
                source_agent: config.source_agent.map(|a| a.bin_name().to_owned()),
                target_agent: target_agent.bin_name().to_owned(),
                prompt,
            };
            let result = send_socket_request(socket_path, request).await?;
            tool_response(id, result)
        }
        "mention_user" => {
            ensure_tool_enabled(config.permissions.mention_user, "mention_user")?;
            let room_id = bound_room_id(&args, config, "mention_user")?;
            let message = required_string_arg(&args, "message")?.trim().to_owned();
            anyhow::ensure!(!message.is_empty(), "message must not be empty");
            let request = SocketRequest::MentionUser {
                room_id: room_id.to_owned(),
                source_agent: config.source_agent.map(|a| a.bin_name().to_owned()),
                message,
            };
            let result = send_socket_request(socket_path, request).await?;
            tool_response(id, result)
        }
        _ => anyhow::bail!("unknown Minos MCP tool: {name}"),
    }
}

async fn send_socket_request(
    socket_path: &Path,
    request: SocketRequest,
) -> Result<serde_json::Value> {
    use std::os::unix::net::UnixStream;
    let stream = UnixStream::connect(socket_path)
        .with_context(|| format!("failed to connect to Minos socket at {}", socket_path.display()))?;
    stream.set_read_timeout(Some(Duration::from_secs(30)))?;
    stream.set_write_timeout(Some(Duration::from_secs(10)))?;
    let mut stream = tokio::task::spawn_blocking(move || -> Result<(SocketResponse, UnixStream)> {
        use crate::mcp_socket::{read_frame, write_response, SocketResponse};
        let payload = serde_json::to_vec(&request)?;
        let len = u32::try_from(payload.len()).context("request payload too large")?;
        let mut buf = Vec::with_capacity(4 + payload.len());
        buf.extend_from_slice(&len.to_be_bytes());
        buf.extend_from_slice(&payload);
        {
            use std::io::Write;
            stream.write_all(&buf)?;
            stream.flush()?;
        }
        let mut stream_ref = &stream;
        let response = read_frame::<&std::os::unix::net::UnixStream>(&mut stream_ref)?;
        match response {
            None => anyhow::bail!("Minos socket closed before response"),
            Some(SocketResponse::Ok { data }) => Ok((SocketResponse::Ok { data }, stream)),
            Some(SocketResponse::Error { message }) => anyhow::bail!("Minos socket error: {message}"),
            Some(SocketResponse::Pong) => Ok((SocketResponse::Pong, stream)),
        }
    })
    .await??;
    match stream.0 {
        SocketResponse::Ok { data } => Ok(data.unwrap_or(serde_json::Value::Null)),
        SocketResponse::Error { message } => anyhow::bail!("{message}"),
        SocketResponse::Pong => Ok(serde_json::Value::Null),
    }
}

fn tools_for_permissions(permissions: McpToolPermissions) -> Vec<Value> {
    let mut tools = Vec::new();
    if permissions.read_chat {
        tools.push(list_chat_messages_tool());
    }
    if permissions.mention_agent {
        tools.push(request_agent_help_tool());
    }
    if permissions.mention_user {
        tools.push(mention_user_tool());
    }
    tools
}

fn list_chat_messages_tool() -> Value {
    json!({
        "name": "list_chat_messages",
        "description": "Read messages from the Minos chat room bound to this MCP server, newest-first with cursor pagination.",
        "inputSchema": {
            "type": "object",
            "properties": pagination_properties()
        }
    })
}

fn request_agent_help_tool() -> Value {
    let mut properties = Map::new();
    properties.insert(
        "target_agent".into(),
        json!({
            "type": "string",
            "enum": agent_name_values(),
            "description": "The Minos agent to mention in this room."
        }),
    );
    properties.insert(
        "prompt".into(),
        json!({
            "type": "string",
            "description": "The prompt to send to the target agent."
        }),
    );
    json!({
        "name": "request_agent_help",
        "description": "Ask another Minos agent in this chat room to help with a prompt.",
        "inputSchema": {
            "type": "object",
            "properties": properties,
            "required": ["target_agent", "prompt"]
        }
    })
}

fn mention_user_tool() -> Value {
    let mut properties = Map::new();
    properties.insert(
        "message".into(),
        json!({
            "type": "string",
            "description": "A concise user-visible message to post in the room."
        }),
    );
    json!({
        "name": "mention_user",
        "description": "Post a user-visible mention into this Minos chat room.",
        "inputSchema": {
            "type": "object",
            "properties": properties,
            "required": ["message"]
        }
    })
}

fn pagination_properties() -> Map<String, Value> {
    Map::from_iter([
        (
            "before_seq".into(),
            json!({
                "type": "integer",
                "minimum": 1,
                "description": "Return messages with seq lower than this cursor."
            }),
        ),
        (
            "limit".into(),
            json!({
                "type": "integer",
                "minimum": 1,
                "maximum": 500,
                "description": "Maximum messages to return. Defaults to 100."
            }),
        ),
    ])
}

fn bound_room_id<'a>(
    args: &Value,
    config: &'a McpServerConfig,
    tool_name: &str,
) -> Result<&'a str> {
    anyhow::ensure!(
        args.get("room_id").is_none(),
        "{tool_name} does not accept room_id; this MCP server is bound to a single room at startup"
    );
    Ok(config.room_id.as_str())
}

fn required_string_arg<'a>(args: &'a Value, name: &str) -> Result<&'a str> {
    args.get(name)
        .and_then(Value::as_str)
        .with_context(|| format!("{name} is required"))
}

fn parse_agent_arg(value: &str) -> Result<AgentName> {
    let normalized = value.trim().to_ascii_lowercase();
    AgentName::all()
        .iter()
        .copied()
        .find(|agent| agent.bin_name() == normalized.as_str())
        .with_context(|| format!("unknown agent: {value}"))
}

fn agent_name_values() -> Vec<&'static str> {
    AgentName::all().iter().map(|agent| agent.bin_name()).collect()
}

fn ensure_tool_enabled(enabled: bool, name: &str) -> Result<()> {
    anyhow::ensure!(enabled, "{name} is disabled by this MCP server policy");
    Ok(())
}

fn tool_response(id: Value, payload: Value) -> Result<Value> {
    Ok(json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": {
            "content": [{
                "type": "text",
                "text": serde_json::to_string_pretty(&payload)?
            }],
            "structuredContent": payload,
            "isError": false
        }
    }))
}

fn error_response(id: Value, code: i64, message: &str) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": { "code": code, "message": message }
    })
}

fn write_json(stdout: &mut std::io::Stdout, response: &Value) -> Result<()> {
    serde_json::to_writer(&mut *stdout, response)?;
    stdout.write_all(b"\n")?;
    stdout.flush()?;
    Ok(())
}
```

- [ ] **Step 2: 创建 `bin/minos-mcp.rs`**

```rust
// crates/minos-chat-store/src/bin/minos-mcp.rs

use std::path::PathBuf;

use anyhow::Result;
use clap::Parser;

#[derive(Debug, Parser)]
#[command(
    name = "minos-mcp",
    about = "Expose Minos features over MCP stdio, proxied to the Minos main process via Unix socket"
)]
struct Args {
    #[arg(long)]
    socket_path: PathBuf,

    #[arg(long)]
    db_path: PathBuf,

    #[arg(long)]
    room_id: String,

    #[arg(long)]
    source_agent: Option<String>,

    #[arg(long)]
    disable_read_chat: bool,

    #[arg(long)]
    disable_mention_agent: bool,

    #[arg(long)]
    disable_mention_user: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    let source_agent = args
        .source_agent
        .as_deref()
        .map(parse_agent_name)
        .transpose()?;
    minos_chat_store::mcp_server::serve_stdio(minos_chat_store::mcp_server::McpServerConfig {
        socket_path: args.socket_path,
        db_path: args.db_path,
        room_id: args.room_id,
        source_agent,
        permissions: minos_chat_store::mcp_server::McpToolPermissions {
            read_chat: !args.disable_read_chat,
            mention_agent: !args.disable_mention_agent,
            mention_user: !args.disable_mention_user,
        },
    })
    .await
}

fn parse_agent_name(value: &str) -> Result<minos_domain::AgentName> {
    let normalized = value.trim().to_ascii_lowercase();
    minos_domain::AgentName::all()
        .iter()
        .copied()
        .find(|agent| agent.bin_name() == normalized.as_str())
        .ok_or_else(|| anyhow::anyhow!("unknown agent: {value}"))
}
```

- [ ] **Step 3: 更新 `Cargo.toml`**

在 `crates/minos-chat-store/Cargo.toml` 中：
- 将 `[[bin]] name = "minos-chat-mcp" path = "src/bin/minos-chat-mcp.rs"` 替换为 `[[bin]] name = "minos-mcp" path = "src/bin/minos-mcp.rs"`

完整的新 Cargo.toml：

```toml
[package]
name = "minos-chat-store"
version.workspace = true
edition.workspace = true
license.workspace = true
authors.workspace = true
homepage.workspace = true
repository.workspace = true
rust-version.workspace = true

[[bin]]
name = "minos-mcp"
path = "src/bin/minos-mcp.rs"

[dependencies]
minos-domain = { path = "../minos-domain", version = "0.1.0" }
minos-protocol = { path = "../minos-protocol", version = "0.1.0" }
anyhow = { workspace = true }
chrono = { workspace = true }
clap = { workspace = true }
serde = { workspace = true }
serde_json = { workspace = true }
sqlx = { workspace = true }
tokio = { workspace = true }
tracing = { workspace = true }

[dev-dependencies]
tempfile = { workspace = true }

[lints]
workspace = true
```

- [ ] **Step 4: 编译验证**

Run: `cargo check -p minos-chat-store`
Expected: 编译通过

- [ ] **Step 5: Commit**

```bash
git add crates/minos-chat-store/
git commit -m "feat(chat-store): add new MCP server with Unix socket proxy"
```

---

## Task 3: 创建主进程侧 Socket 请求处理器

**Files:**
- Create: `crates/minos-chat-store/src/mcp_handler.rs`

主进程侧的 Socket 监听器：接收 MCP Server 发来的请求，调用回调处理，返回结果。

- [ ] **Step 1: 创建 `mcp_handler.rs`**

```rust
// crates/minos-chat-store/src/mcp_handler.rs

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result};
use tokio::net::UnixListener;
use tokio::sync::oneshot;
use tracing::{debug, warn};

use crate::mcp_socket::{SocketRequest, SocketResponse};

pub type ToolCallback = Arc<dyn Fn(SocketRequest) -> tokio::task::JoinHandle<Result<SocketResponse>> + Send + Sync>;

pub struct McpSocketHandler {
    socket_path: PathBuf,
    callback: ToolCallback,
}

impl McpSocketHandler {
    pub fn new(socket_path: PathBuf, callback: ToolCallback) -> Self {
        Self { socket_path, callback }
    }

    pub async fn run(&self) -> Result<()> {
        if self.socket_path.exists() {
            let _ = std::fs::remove_file(&self.socket_path);
        }
        if let Some(parent) = self.socket_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let listener = UnixListener::bind(&self.socket_path)
            .with_context(|| format!("failed to bind MCP socket at {}", self.socket_path.display()))?;
        debug!(
            target: "minos_mcp_handler",
            path = %self.socket_path.display(),
            "MCP socket listener started"
        );
        loop {
            match listener.accept().await {
                Ok((stream, _addr)) => {
                    let callback = self.callback.clone();
                    tokio::spawn(async move {
                        if let Err(error) = Self::handle_connection(stream, callback).await {
                            debug!(
                                target: "minos_mcp_handler",
                                error = %error,
                                "MCP socket connection ended"
                            );
                        }
                    });
                }
                Err(error) => {
                    warn!(
                        target: "minos_mcp_handler",
                        error = %error,
                        "failed to accept MCP socket connection"
                    );
                }
            }
        }
    }

    async fn handle_connection(
        stream: tokio::net::UnixStream,
        callback: ToolCallback,
    ) -> Result<()> {
        let (read_half, write_half) = stream.into_split();
        let mut reader = tokio::io::BufReader::new(read_half);
        let mut writer = tokio::io::BufWriter::new(write_half);
        loop {
            let payload_len = read_u32(&mut reader).await?;
            let mut payload = vec![0u8; payload_len as usize];
            tokio::io::AsyncReadExt::read_exact(&mut reader, &mut payload).await?;
            let request: SocketRequest = serde_json::from_slice(&payload)
                .context("failed to deserialize socket request")?;
            debug!(
                target: "minos_mcp_handler",
                ?request,
                "received MCP socket request"
            );
            let handle = callback(request);
            let response = match handle.await {
                Ok(Ok(response)) => response,
                Ok(Err(error)) => SocketResponse::Error { message: error.to_string() },
                Err(join_error) => SocketResponse::Error { message: join_error.to_string() },
            };
            let response_bytes = serde_json::to_vec(&response)?;
            let response_len = u32::try_from(response_bytes.len())
                .context("response payload too large")?;
            use tokio::io::AsyncWriteExt;
            writer.write_all(&response_len.to_be_bytes()).await?;
            writer.write_all(&response_bytes).await?;
            writer.flush().await?;
        }
    }
}

impl Drop for McpSocketHandler {
    fn drop(&mut self) {
        if self.socket_path.exists() {
            let _ = std::fs::remove_file(&self.socket_path);
        }
    }
}

async fn read_u32<R: tokio::io::AsyncRead + Unpin>(reader: &mut R) -> Result<u32> {
    let mut buf = [0u8; 4];
    tokio::io::AsyncReadExt::read_exact(reader, &mut buf).await?;
    Ok(u32::from_be_bytes(buf))
}
```

- [ ] **Step 2: 在 `lib.rs` 中注册模块**

在 `crates/minos-chat-store/src/lib.rs` 中添加 `pub mod mcp_handler;`

- [ ] **Step 3: 编译验证**

Run: `cargo check -p minos-chat-store`
Expected: 编译通过

- [ ] **Step 4: Commit**

```bash
git add crates/minos-chat-store/src/mcp_handler.rs crates/minos-chat-store/src/lib.rs
git commit -m "feat(chat-store): add MCP socket handler for main process side"
```

---

## Task 4: 清理旧的 MCP 代码和 DB 基础设施

**Files:**
- Delete: `crates/minos-chat-store/src/mcp.rs` (旧的全部删除)
- Delete: `crates/minos-chat-store/src/bin/minos-chat-mcp.rs`
- Modify: `crates/minos-chat-store/src/lib.rs`

彻底移除旧的 DB 轮询基础设施。

- [ ] **Step 1: 删除 `crates/minos-chat-store/src/mcp.rs`**

删除整个文件。所有旧代码（`serve_stdio`、`ChatMcpServerConfig`、`ChatMcpToolPermissions`、`handle_request`、`handle_tool_call` 等）都被 Task 2 的新 `mcp_server.rs` 替代。

- [ ] **Step 2: 删除 `crates/minos-chat-store/src/bin/minos-chat-mcp.rs`**

删除整个文件。被 Task 2 的 `bin/minos-mcp.rs` 替代。

- [ ] **Step 3: 清理 `lib.rs` — 移除旧类型和 DB 方法**

从 `crates/minos-chat-store/src/lib.rs` 中移除：
- `ChatMcpCommand` struct (line 80-93)
- `NewChatMcpCommand` struct (line 95-101)
- `ChatMcpCommandKind` enum (line 103-108)
- `ChatMcpCommandStatus` enum (line 110-117)
- `ChatStore::enqueue_mcp_command()` (line 375-399)
- `ChatStore::claim_pending_mcp_commands()` (line 401-441)
- `ChatStore::complete_mcp_command()` (line 443-455)
- `ChatStore::fail_mcp_command()` (line 457-470)
- `ChatStore::get_mcp_command()` (line 472-478)
- `ChatMcpCommandKind::as_db()` / `from_db()` (line 681-696)
- `ChatMcpCommandStatus::from_db()` (line 698-708)
- `chat_mcp_command_from_row()` (line 795-812)
- DB migration 中的 `chat_mcp_commands` 表和索引 (line 550-573)
- `pub mod mcp;` 声明 → 替换为 `pub mod mcp_server;`

- [ ] **Step 4: 移除旧的 MCP 相关测试**

从 `lib.rs` 的 `#[cfg(test)] mod tests` 中移除：
- `mcp_commands_can_be_claimed_and_completed` 测试 (line 963-1008)

- [ ] **Step 5: 编译验证**

Run: `cargo check -p minos-chat-store`
Expected: 编译通过（此时其他 crate 会报错，因为依赖了旧类型）

- [ ] **Step 6: Commit**

```bash
git add -A crates/minos-chat-store/
git commit -m "refactor(chat-store): remove old MCP DB polling infrastructure"
```

---

## Task 5: 更新 Agent Runtime 配置和注入逻辑

**Files:**
- Modify: `crates/minos-agent-runtime/src/config.rs`
- Modify: `crates/minos-agent-runtime/src/manager.rs`

- [ ] **Step 1: 重写 `config.rs`**

将 `ChatMcpConfig` 重命名为 `McpConfig`，用 `socket_path` 替代 `db_path`，`server_bin` 默认为 `minos-mcp`：

```rust
// config.rs 中相关的改动

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpConfig {
    pub server_bin: PathBuf,
    pub server_args: Vec<String>,
    pub socket_path: PathBuf,
    pub db_path: PathBuf,
    pub permissions: minos_chat_store::mcp_server::McpToolPermissions,
}
```

更新 `AgentRuntimeConfig` 中 `chat_mcp: Option<ChatMcpConfig>` → `mcp: Option<McpConfig>`。

更新方法：
- `enable_default_chat_mcp()` → `enable_default_mcp()`，默认 `server_bin: "minos-mcp"`
- `enable_chat_mcp_with_command()` → `enable_mcp_with_command()`，新增 `socket_path` 参数

- [ ] **Step 2: 更新 `manager.rs` 中的解析和注入**

- `ResolvedChatMcpServer` → `ResolvedMcpServer`
- `resolve_chat_mcp_server()` → `resolve_mcp_server()`
- 参数从 `--db-path`/`--room-id`/`--source-agent` 改为 `--socket-path`/`--db-path`/`--room-id`/`--source-agent`
- 所有引用 `config.chat_mcp` 的地方改为 `config.mcp`
- 所有引用 `minos-chat-mcp` 的地方改为 `minos-mcp`
- `chat_mcp_permission_args()` → `mcp_permission_args()`
- 测试中的 `"chat-mcp"` 全部改为对应的新参数

关键的 `resolve_mcp_server` 函数：

```rust
fn resolve_mcp_server(
    config: Option<&McpConfig>,
    workspace: &Path,
    source_agent: AgentName,
) -> Option<ResolvedMcpServer> {
    let config = config?;
    let mut args = config.server_args.clone();
    args.extend([
        "--socket-path".into(),
        config.socket_path.display().to_string(),
        "--db-path".into(),
        config.db_path.display().to_string(),
        "--room-id".into(),
        minos_chat_store::room_id_for_workspace(workspace),
        "--source-agent".into(),
        source_agent.bin_name().into(),
    ]);
    args.extend(mcp_permission_args(config.permissions));
    Some(ResolvedMcpServer {
        name: "minos_chat".into(),
        command: config.server_bin.display().to_string(),
        args,
    })
}
```

- [ ] **Step 3: 编译验证**

Run: `cargo check -p minos-agent-runtime`
Expected: 编译通过

- [ ] **Step 4: Commit**

```bash
git add crates/minos-agent-runtime/
git commit -m "refactor(agent-runtime): rename ChatMcpConfig to McpConfig, use socket-path"
```

---

## Task 6: 更新 Claude 驱动和 ACP 协议测试引用

**Files:**
- Modify: `crates/minos-agent-runtime/src/claude_driver.rs`
- Modify: `crates/minos-acp-protocol/src/types.rs`

- [ ] **Step 1: 更新 `claude_driver.rs` 中的测试引用**

将测试中的 `"minos-chat-mcp"` 引用改为 `"minos-mcp"`。

- [ ] **Step 2: 更新 `minos-acp-protocol/src/types.rs` 中的测试引用**

将测试中的 `"chat-mcp"` 引用改为 `"minos-mcp"`。

- [ ] **Step 3: 编译验证**

Run: `cargo check -p minos-agent-runtime -p minos-acp-protocol`
Expected: 编译通过

- [ ] **Step 4: Commit**

```bash
git add crates/minos-agent-runtime/src/claude_driver.rs crates/minos-acp-protocol/src/types.rs
git commit -m "refactor: update test references from minos-chat-mcp to minos-mcp"
```

---

## Task 7: 重构 TUI — 启动 Socket 监听，移除 DB 轮询

**Files:**
- Modify: `crates/minos-tui/src/main.rs`
- Modify: `crates/minos-tui/src/backend/embedded.rs`
- Rewrite: `crates/minos-tui/src/group_chat.rs`
- Modify: `crates/minos-tui/src/app.rs`

这是最核心的集成任务。

- [ ] **Step 1: 重写 `main.rs`**

- 子命令 `ChatMcp` → `MinosMcp`
- `#[command(name = "chat-mcp")]` → `#[command(name = "minos-mcp")]`
- `ChatMcpArgs` → `MinosMcpArgs`
- CLI flags: `--chat-mcp-disable-*` → `--mcp-disable-*`
- `has_chat_mcp_policy_overrides` → `has_mcp_policy_overrides`
- `chat_mcp_permissions_from_cli` → `mcp_permissions_from_cli`
- `chat-mcp` 子命令处理：调用 `minos_chat_store::mcp_server::serve_stdio`，传入 `McpServerConfig`（含 `socket_path`、`db_path`、`room_id`）
- 移除对旧 `mcp` 模块的引用，改用 `mcp_server`

- [ ] **Step 2: 重写 `embedded.rs`**

- 启动时创建临时 Unix Domain Socket 路径（`$MINOS_HOME/run/mcp-{random}.sock`）
- 启动 `McpSocketHandler` 为后台 tokio task
- 将 `socket_path` 传入 `McpConfig`
- 回调函数直接通过 `AppEvent` 发送 tool call 到 App 的事件循环

```rust
pub struct EmbeddedBackend {
    manager: Arc<AgentManager>,
    mcp_event_tx: tokio::sync::mpsc::UnboundedSender<McpToolEvent>,
}

pub struct McpToolEvent {
    pub request: SocketRequest,
    pub response_tx: oneshot::Sender<Result<SocketResponse>>,
}
```

- [ ] **Step 3: 重写 `group_chat.rs`**

移除：
- `claim_pending_mcp_commands()`
- `complete_mcp_command()`
- `fail_mcp_command()`

这些方法不再需要。

- [ ] **Step 4: 重写 `app.rs` 中的 MCP 处理**

移除：
- `process_pending_mcp_commands()` 方法
- `process_mcp_command()` 方法
- `tick()` 中的 `process_pending_mcp_commands()` 调用
- `use minos_chat_store::{ChatMcpCommand, ChatMcpCommandKind}` 导入

新增：
- 处理 `AppEvent::McpToolCall(McpToolEvent)` 事件
- 在事件处理中调用已有的 `dispatch_prompt_to_agent()` 和群聊消息追加逻辑
- 通过 `response_tx` 同步返回结果

```rust
// 在 handle_event 中
AppEvent::McpToolCall(event) => {
    let response = self.handle_mcp_tool_call(event.request).await;
    let _ = event.response_tx.send(response);
    true
}
```

- [ ] **Step 5: 更新测试**

移除/重写 `app.rs` 中的 MCP 相关测试：
- `tick_processes_mcp_agent_help_command` → 改为通过 `AppEvent::McpToolCall` 测试
- `tick_processes_mcp_user_mention_command` → 同上

- [ ] **Step 6: 编译验证**

Run: `cargo check -p minos-tui`
Expected: 编译通过

- [ ] **Step 7: Commit**

```bash
git add crates/minos-tui/
git commit -m "refactor(tui): replace MCP DB polling with Unix socket, rename chat-mcp to minos-mcp"
```

---

## Task 8: 更新 Daemon 模式

**Files:**
- Modify: `crates/minos-daemon/src/agent.rs`

- [ ] **Step 1: 更新 `AgentGlue::new()`**

- `cfg.enable_default_chat_mcp()` → `cfg.enable_default_mcp()`
- 传入 socket_path（daemon 模式使用 `$MINOS_HOME/run/mcp-daemon.sock`）
- 启动 `McpSocketHandler` 后台任务
- daemon 模式的 tool call 处理需要通过类似的事件机制或直接调用 manager 方法

- [ ] **Step 2: 编译验证**

Run: `cargo check -p minos-daemon`
Expected: 编译通过

- [ ] **Step 3: Commit**

```bash
git add crates/minos-daemon/
git commit -m "refactor(daemon): update MCP config to use socket path"
```

---

## Task 9: 全局编译验证和测试

**Files:**
- 可能微调各文件修复编译错误

- [ ] **Step 1: 全 workspace 编译**

Run: `cargo check --workspace`
Expected: 编译通过

- [ ] **Step 2: 运行所有测试**

Run: `cargo test --workspace`
Expected: 所有测试通过

- [ ] **Step 3: 运行 Clippy**

Run: `cargo clippy --workspace`
Expected: 无新的 warning

- [ ] **Step 4: 修复任何剩余问题并 Commit**

```bash
git add -A
git commit -m "fix: resolve remaining compilation and test issues"
```

---

## Task 10: 清理遗留代码

**Files:**
- 全局搜索并清理

- [ ] **Step 1: 搜索并移除所有残留的 `chat_mcp`/`chat-mcp`/`minos-chat-mcp` 引用**

Run: `rg "chat.mcp|minos.chat.mcp|ChatMcp|chat_mcp" crates/`

确保无遗漏。

- [ ] **Step 2: 搜索并确认 `chat_mcp_commands` 表完全移除**

Run: `rg "chat_mcp_commands" crates/`

确保无遗漏。

- [ ] **Step 3: 最终全量验证**

Run: `cargo test --workspace && cargo clippy --workspace`
Expected: 全部通过

- [ ] **Step 4: Commit**

```bash
git add -A
git commit -m "chore: clean up all legacy chat-mcp references"
```

---

## 架构图

```
┌─────────────────────┐
│  Agent (Codex等)     │
│  MCP client         │
└──────┬──────────────┘
       │ stdin/stdout (JSON-RPC)
       ▼
┌──────────────────────┐
│  minos-mcp 子进程     │
│  (mcp_server.rs)     │
│                      │
│  initialize/tools/list/tools/call: MCP 协议适配
│  list_chat_messages:  直接查 DB ──────────┐
│  request_agent_help:  ──Socket──┐         │
│  mention_user:        ──Socket──┤         │
└──────────────────────────────────┤         │
       Unix Domain Socket          │         │
       (长度前缀 + JSON 帧)        │         │
┌──────────────────────────────────┤         │
│  Minos 主进程 (TUI/Daemon)        │         │
│  (mcp_handler.rs)                │         │
│                      ◄───────────┘         │
│  McpSocketHandler:                             │
│    ├─ 接收 SocketRequest                   │
│    ├─ 调用回调 (AppEvent)                  │
│    ├─ request_agent_help → dispatch_prompt │
│    ├─ mention_user → append_group_chat     │
│    └─ 返回 SocketResponse                  │
│                                  ◄─────────┘
│  ChatStore (DB 直接读写)         SQLite
└─────────────────────────────────────────────┘

生命周期:
- Minos 主进程启动 → 创建 Socket 文件 → 启动 McpSocketHandler 监听
- Agent 启动 → MCP 子进程连接 Socket
- Minos 退出 → Socket 文件删除 → MCP 子进程检测断连 → 自动退出
```

## 关键设计决策

| 决策 | 选择 | 原因 |
|------|------|------|
| Socket 类型 | Unix Domain Socket | 本地通信，无需端口管理，文件系统权限控制 |
| 帧协议 | 4字节长度前缀 + JSON | 简单可靠，与 MCP stdio JSON-RPC 解耦 |
| 读操作路由 | MCP Server 直接查 DB | 避免不必要的 Socket 往返，list_chat_messages 是纯读操作 |
| 写操作路由 | Socket 转发到主进程 | 需要主进程的 App 上下文（启动 Agent、更新 UI） |
| 生命周期管理 | Socket 断连 → MCP Server 退出 | 无需额外心跳，Unix Socket 天然感知对端关闭 |
| 命名 | `minos-mcp` | chat 只是 MCP 暴露能力的一部分，未来可扩展更多 tool |
