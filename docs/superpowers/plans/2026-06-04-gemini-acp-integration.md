# Gemini CLI ACP Integration Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Integrate Gemini CLI into Minos host via ACP (Agent Client Protocol), enabling structured JSON-RPC communication with full session management, tool approval, and UI event translation.

**Architecture:** New `minos-acp-protocol` crate mirrors `minos-codex-protocol` pattern — hand-written ACP v1 types + typed request/notification traits. New `AcpClient` (stdio pump) mirrors `CodexClient` (WS pump). New `gemini_driver.rs` manages ACP lifecycle. Rewrite `gemini.rs` translator for full ACP event coverage.

**Tech Stack:** Rust, tokio (async runtime), serde/serde_json (serialization), ACP v1 spec (JSON-RPC 2.0 over stdio)

---

## Task 1: Create minos-acp-protocol crate skeleton

**Files:**
- Create: `crates/minos-acp-protocol/Cargo.toml`
- Create: `crates/minos-acp-protocol/src/lib.rs`
- Create: `crates/minos-acp-protocol/src/jsonrpc.rs`
- Modify: `Cargo.toml` (workspace — already uses `members = ["crates/*"]`, so auto-discovered)

- [ ] **Step 1: Create Cargo.toml**

```toml
[package]
name = "minos-acp-protocol"
version = "0.1.0"
edition.workspace = true
license.workspace = true
authors.workspace = true
description = "Typed Rust mirror of the ACP (Agent Client Protocol) v1 JSON-RPC protocol."

[lib]
doctest = false

[dependencies]
serde = { workspace = true, features = ["derive"] }
serde_json = { workspace = true }

[dev-dependencies]
pretty_assertions = { workspace = true }

[lints]
workspace = true
```

- [ ] **Step 2: Create lib.rs with public exports**

```rust
#![forbid(unsafe_code)]

pub mod client_notification;
pub mod client_request;
pub mod jsonrpc;
pub mod server_notification;
pub mod server_request;
pub mod types;

pub use client_notification::AcpClientNotification;
pub use client_request::AcpClientRequest;
pub use jsonrpc::*;
pub use server_notification::*;
pub use server_request::*;
pub use types::*;
```

- [ ] **Step 3: Create jsonrpc.rs — ACP uses strict JSON-RPC 2.0 (includes `jsonrpc` field)**

Unlike `minos-codex-protocol` which omits `jsonrpc`, ACP requires it per spec.

```rust
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcRequest<P> {
    pub jsonrpc: &'static str,
    pub id: serde_json::Value,
    pub method: String,
    pub params: P,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcResponse<R> {
    pub jsonrpc: &'static str,
    pub id: serde_json::Value,
    pub result: R,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcError {
    pub jsonrpc: &'static str,
    pub id: serde_json::Value,
    pub error: JsonRpcErrorPayload,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcErrorPayload {
    pub code: i64,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub data: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcNotification<P> {
    pub jsonrpc: &'static str,
    pub method: String,
    pub params: P,
}

const JSONRPC_VERSION: &str = "2.0";

pub fn make_request(id: serde_json::Value, method: &str, params: serde_json::Value) -> serde_json::Value {
    serde_json::json!({
        "jsonrpc": JSONRPC_VERSION,
        "id": id,
        "method": method,
        "params": params,
    })
}

pub fn make_notification(method: &str, params: serde_json::Value) -> serde_json::Value {
    serde_json::json!({
        "jsonrpc": JSONRPC_VERSION,
        "method": method,
        "params": params,
    })
}

pub fn make_response(id: serde_json::Value, result: serde_json::Value) -> serde_json::Value {
    serde_json::json!({
        "jsonrpc": JSONRPC_VERSION,
        "id": id,
        "result": result,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn make_request_includes_jsonrpc_field() {
        let frame = make_request(serde_json::json!(1), "initialize", serde_json::json!({}));
        assert_eq!(frame["jsonrpc"], "2.0");
        assert_eq!(frame["id"], 1);
        assert_eq!(frame["method"], "initialize");
    }

    #[test]
    fn make_notification_omits_id_field() {
        let frame = make_notification("session/cancel", serde_json::json!({}));
        assert_eq!(frame["jsonrpc"], "2.0");
        assert!(frame.get("id").is_none(), "notifications must not carry id");
        assert_eq!(frame["method"], "session/cancel");
    }

    #[test]
    fn make_response_includes_jsonrpc_field() {
        let frame = make_response(serde_json::json!("req-1"), serde_json::json!({"outcome": "allow"}));
        assert_eq!(frame["jsonrpc"], "2.0");
        assert_eq!(frame["id"], "req-1");
        assert_eq!(frame["result"]["outcome"], "allow");
    }
}
```

- [ ] **Step 4: Create empty placeholder files for remaining modules**

Create `crates/minos-acp-protocol/src/types.rs`:
```rust
//! ACP v1 base types.
```

Create `crates/minos-acp-protocol/src/client_request.rs`:
```rust
use serde::de::DeserializeOwned;
use serde::Serialize;

pub trait AcpClientRequest: Serialize {
    const METHOD: &'static str;
    type Response: DeserializeOwned;
}

pub trait AcpClientNotification: Serialize {
    const METHOD: &'static str;
}
```

Create `crates/minos-acp-protocol/src/client_notification.rs`:
```rust
//! Re-export from client_request for convenience.
pub use crate::client_request::AcpClientNotification;
```

Create `crates/minos-acp-protocol/src/server_request.rs`:
```rust
//! ACP Agent→Client request types (session/request_permission, fs/*, terminal/*).
```

Create `crates/minos-acp-protocol/src/server_notification.rs`:
```rust
//! ACP Agent→Client notification types (session/update and all variants).
```

- [ ] **Step 5: Verify crate compiles**

Run: `cargo check -p minos-acp-protocol`
Expected: compilation succeeds with no errors

- [ ] **Step 6: Commit**

```bash
git add crates/minos-acp-protocol/
git commit -m "feat: add minos-acp-protocol crate skeleton with JSON-RPC 2.0 framing"
```

---

## Task 2: Implement ACP v1 base types

**Files:**
- Modify: `crates/minos-acp-protocol/src/types.rs`

- [ ] **Step 1: Write tests for base types**

Add to `crates/minos-acp-protocol/src/types.rs`:

```rust
use serde::{Deserialize, Serialize};

pub type SessionId = String;

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum StopReason {
    EndTurn,
    MaxTokens,
    MaxTurnRequests,
    Refusal,
    Cancelled,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentBlock {
    Text { text: String },
    Image { data: String, mime_type: String },
    Audio { data: String, mime_type: String },
    Resource { resource: ResourceContent },
    ResourceLink { uri: String, #[serde(skip_serializing_if = "Option::is_none", default)] name: Option<String> },
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct ResourceContent {
    pub uri: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub mime_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub text: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ToolCallKind {
    Edit,
    Diff,
    Terminal,
    Other,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ToolCallStatus {
    Pending,
    InProgress,
    Completed,
    Cancelled,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ToolCallUpdate {
    pub tool_call_id: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub title: Option<String>,
    pub kind: ToolCallKind,
    pub status: ToolCallStatus,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub content: Option<Vec<ToolCallContent>>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ToolCallContent {
    Content { content: ContentBlock },
    Terminal { terminal_id: String },
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RequestPermissionOutcome {
    Allow,
    Deny,
    Cancelled,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct PermissionOption {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub description: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Implementation {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub version: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct ClientCapabilities {
    #[serde(default)]
    pub fs: FsCapabilities,
    #[serde(default)]
    pub terminal: bool,
}

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct FsCapabilities {
    #[serde(default)]
    pub read_text_file: bool,
    #[serde(default)]
    pub write_text_file: bool,
}

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct AgentCapabilities {
    #[serde(default)]
    pub load_session: bool,
    #[serde(default)]
    pub prompt_capabilities: PromptCapabilities,
    #[serde(default)]
    pub mcp_capabilities: McpCapabilities,
    #[serde(default)]
    pub auth: AgentAuthCapabilities,
    #[serde(default)]
    pub session_capabilities: SessionCapabilities,
}

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct PromptCapabilities {
    #[serde(default)]
    pub image: bool,
    #[serde(default)]
    pub audio: bool,
    #[serde(default)]
    pub embedded_context: bool,
}

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct McpCapabilities {
    #[serde(default)]
    pub http: bool,
    #[serde(default)]
    pub sse: bool,
}

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct AgentAuthCapabilities {
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub logout: Option<LogoutCapabilities>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct LogoutCapabilities {}

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct SessionCapabilities {
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub close: Option<CloseCapability>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub list: Option<ListCapability>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub resume: Option<ResumeCapability>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub additional_directories: Option<AdditionalDirectoriesCapability>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct CloseCapability {}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ListCapability {}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ResumeCapability {}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct AdditionalDirectoriesCapability {}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct AuthMethod {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub description: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct McpServer {
    pub name: String,
    #[serde(flatten)]
    pub transport: McpTransport,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(tag = "transport_type", rename_all = "snake_case")]
pub enum McpTransport {
    Stdio { command: String, #[serde(default)] args: Vec<String>, #[serde(default)] env: std::collections::HashMap<String, String> },
    Http { url: String },
    Sse { url: String },
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct SessionModeState {
    pub current_mode_id: SessionModeId,
    pub available_modes: Vec<SessionMode>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct SessionMode {
    pub id: SessionModeId,
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub description: Option<String>,
}

pub type SessionModeId = String;

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct SessionConfigOption {
    pub id: SessionConfigId,
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub description: Option<String>,
    pub values: Vec<SessionConfigValue>,
    pub current_value_id: SessionConfigValueId,
}

pub type SessionConfigId = String;
pub type SessionConfigValueId = String;

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct SessionConfigValue {
    pub id: SessionConfigValueId,
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub description: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct SessionInfo {
    pub session_id: SessionId,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub cwd: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub created_at: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn stop_reason_round_trips() {
        let reason = StopReason::EndTurn;
        let json = serde_json::to_string(&reason).unwrap();
        assert_eq!(json, r#""end_turn""#);
        let back: StopReason = serde_json::from_str(&json).unwrap();
        assert_eq!(back, StopReason::EndTurn);
    }

    #[test]
    fn content_block_text_round_trips() {
        let block = ContentBlock::Text { text: "hello".into() };
        let json = serde_json::to_string(&block).unwrap();
        assert!(json.contains(r#""type":"text""#));
        let back: ContentBlock = serde_json::from_str(&json).unwrap();
        assert_eq!(back, block);
    }

    #[test]
    fn tool_call_status_round_trips() {
        let status = ToolCallStatus::InProgress;
        let json = serde_json::to_string(&status).unwrap();
        assert_eq!(json, r#""in_progress""#);
        let back: ToolCallStatus = serde_json::from_str(&json).unwrap();
        assert_eq!(back, ToolCallStatus::InProgress);
    }

    #[test]
    fn client_capabilities_default() {
        let caps = ClientCapabilities::default();
        assert!(!caps.fs.read_text_file);
        assert!(!caps.fs.write_text_file);
        assert!(!caps.terminal);
    }

    #[test]
    fn permission_outcome_round_trips() {
        let outcome = RequestPermissionOutcome::Allow;
        let json = serde_json::to_string(&outcome).unwrap();
        assert_eq!(json, r#""allow""#);
        let back: RequestPermissionOutcome = serde_json::from_str(&json).unwrap();
        assert_eq!(back, RequestPermissionOutcome::Allow);
    }
}
```

- [ ] **Step 2: Run tests**

Run: `cargo test -p minos-acp-protocol`
Expected: all tests pass

- [ ] **Step 3: Commit**

```bash
git add crates/minos-acp-protocol/src/types.rs
git commit -m "feat: add ACP v1 base types (ContentBlock, StopReason, ToolCallUpdate, capabilities)"
```

---

## Task 3: Implement ACP client request/notification types

**Files:**
- Modify: `crates/minos-acp-protocol/src/client_request.rs`
- Modify: `crates/minos-acp-protocol/src/client_notification.rs`

- [ ] **Step 1: Write client request types**

Replace `crates/minos-acp-protocol/src/client_request.rs`:

```rust
use serde::de::DeserializeOwned;
use serde::Serialize;

use crate::types::*;

pub trait AcpClientRequest: Serialize {
    const METHOD: &'static str;
    type Response: DeserializeOwned;
}

pub trait AcpClientNotification: Serialize {
    const METHOD: &'static str;
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct InitializeParams {
    pub protocol_version: u32,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub client_capabilities: Option<ClientCapabilities>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub client_info: Option<Implementation>,
}

impl AcpClientRequest for InitializeParams {
    const METHOD: &'static str = "initialize";
    type Response = InitializeResponse;
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct InitializeResponse {
    pub protocol_version: u32,
    #[serde(default)]
    pub agent_capabilities: AgentCapabilities,
    #[serde(default)]
    pub auth_methods: Vec<AuthMethod>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub agent_info: Option<Implementation>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct AuthenticateParams {
    pub method_id: String,
}

impl AcpClientRequest for AuthenticateParams {
    const METHOD: &'static str = "authenticate";
    type Response = AuthenticateResponse;
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct AuthenticateResponse {}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct NewSessionParams {
    pub cwd: String,
    #[serde(default)]
    pub mcp_servers: Vec<McpServer>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub additional_directories: Option<Vec<String>>,
}

impl AcpClientRequest for NewSessionParams {
    const METHOD: &'static str = "session/new";
    type Response = NewSessionResponse;
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct NewSessionResponse {
    pub session_id: SessionId,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub modes: Option<SessionModeState>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub config_options: Option<Vec<SessionConfigOption>>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct LoadSessionParams {
    pub session_id: SessionId,
    pub cwd: String,
    #[serde(default)]
    pub mcp_servers: Vec<McpServer>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub additional_directories: Option<Vec<String>>,
}

impl AcpClientRequest for LoadSessionParams {
    const METHOD: &'static str = "session/load";
    type Response = LoadSessionResponse;
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct LoadSessionResponse {
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub modes: Option<SessionModeState>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub config_options: Option<Vec<SessionConfigOption>>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ResumeSessionParams {
    pub session_id: SessionId,
    pub cwd: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub mcp_servers: Option<Vec<McpServer>>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub additional_directories: Option<Vec<String>>,
}

impl AcpClientRequest for ResumeSessionParams {
    const METHOD: &'static str = "session/resume";
    type Response = ResumeSessionResponse;
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ResumeSessionResponse {
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub modes: Option<SessionModeState>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub config_options: Option<Vec<SessionConfigOption>>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct PromptParams {
    pub session_id: SessionId,
    pub prompt: Vec<ContentBlock>,
}

impl AcpClientRequest for PromptParams {
    const METHOD: &'static str = "session/prompt";
    type Response = PromptResponse;
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct PromptResponse {
    pub stop_reason: StopReason,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct CloseSessionParams {
    pub session_id: SessionId,
}

impl AcpClientRequest for CloseSessionParams {
    const METHOD: &'static str = "session/close";
    type Response = CloseSessionResponse;
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct CloseSessionResponse {}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct SetSessionModeParams {
    pub session_id: SessionId,
    pub mode_id: SessionModeId,
}

impl AcpClientRequest for SetSessionModeParams {
    const METHOD: &'static str = "session/set_mode";
    type Response = SetSessionModeResponse;
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct SetSessionModeResponse {}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct SetConfigOptionParams {
    pub session_id: SessionId,
    pub config_id: SessionConfigId,
    pub value: SessionConfigValueId,
}

impl AcpClientRequest for SetConfigOptionParams {
    const METHOD: &'static str = "session/set_config_option";
    type Response = SetConfigOptionResponse;
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct SetConfigOptionResponse {
    pub config_options: Vec<SessionConfigOption>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ListSessionsParams {
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub cwd: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub cursor: Option<String>,
}

impl AcpClientRequest for ListSessionsParams {
    const METHOD: &'static str = "session/list";
    type Response = ListSessionsResponse;
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ListSessionsResponse {
    pub sessions: Vec<SessionInfo>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub next_cursor: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct LogoutParams {}

impl AcpClientRequest for LogoutParams {
    const METHOD: &'static str = "logout";
    type Response = LogoutResponse;
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct LogoutResponse {}
```

- [ ] **Step 2: Write client notification types**

Replace `crates/minos-acp-protocol/src/client_notification.rs`:

```rust
use crate::client_request::AcpClientNotification;
use crate::types::SessionId;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct CancelNotification {
    pub session_id: SessionId,
}

impl AcpClientNotification for CancelNotification {
    const METHOD: &'static str = "session/cancel";
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client_request::AcpClientRequest;
    use crate::types::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn initialize_method_constant() {
        assert_eq!(InitializeParams::METHOD, "initialize");
    }

    #[test]
    fn prompt_method_constant() {
        assert_eq!(PromptParams::METHOD, "session/prompt");
    }

    #[test]
    fn cancel_method_constant() {
        assert_eq!(CancelNotification::METHOD, "session/cancel");
    }

    #[test]
    fn new_session_params_serializes() {
        let params = NewSessionParams {
            cwd: "/workspace".into(),
            mcp_servers: vec![],
            additional_directories: None,
        };
        let json = serde_json::to_value(&params).unwrap();
        assert_eq!(json["cwd"], "/workspace");
        assert!(json.get("additional_directories").is_none());
    }

    #[test]
    fn prompt_response_deserializes_end_turn() {
        let json = r#"{"stop_reason":"end_turn"}"#;
        let resp: PromptResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.stop_reason, StopReason::EndTurn);
    }
}
```

- [ ] **Step 3: Update lib.rs re-exports**

Replace `crates/minos-acp-protocol/src/lib.rs`:

```rust
#![forbid(unsafe_code)]

pub mod client_notification;
pub mod client_request;
pub mod jsonrpc;
pub mod server_notification;
pub mod server_request;
pub mod types;

pub use client_notification::*;
pub use client_request::*;
pub use jsonrpc::*;
pub use server_notification::*;
pub use server_request::*;
pub use types::*;
```

- [ ] **Step 4: Run tests**

Run: `cargo test -p minos-acp-protocol`
Expected: all tests pass

- [ ] **Step 5: Commit**

```bash
git add crates/minos-acp-protocol/
git commit -m "feat: add ACP client request/notification types (initialize, session/*, prompt)"
```

---

## Task 4: Implement ACP server request/notification types

**Files:**
- Modify: `crates/minos-acp-protocol/src/server_request.rs`
- Modify: `crates/minos-acp-protocol/src/server_notification.rs`

- [ ] **Step 1: Write server request types (Agent→Client)**

Replace `crates/minos-acp-protocol/src/server_request.rs`:

```rust
use crate::types::*;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct RequestPermissionParams {
    pub session_id: SessionId,
    pub tool_call: ToolCallUpdate,
    pub options: Vec<PermissionOption>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct RequestPermissionResponse {
    pub outcome: RequestPermissionOutcome,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct FsReadTextFileParams {
    pub session_id: SessionId,
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub line: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub limit: Option<u32>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct FsReadTextFileResponse {
    pub content: String,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct FsWriteTextFileParams {
    pub session_id: SessionId,
    pub path: String,
    pub content: String,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct FsWriteTextFileResponse {}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct TerminalCreateParams {
    pub session_id: SessionId,
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub cwd: Option<String>,
    #[serde(default)]
    pub env: Vec<EnvVariable>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub output_byte_limit: Option<u64>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct EnvVariable {
    pub name: String,
    pub value: String,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct TerminalCreateResponse {
    pub terminal_id: String,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct TerminalOutputParams {
    pub session_id: SessionId,
    pub terminal_id: String,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct TerminalOutputResponse {
    pub output: String,
    pub truncated: bool,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub exit_status: Option<TerminalExitStatus>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct TerminalExitStatus {
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub exit_code: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub signal: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct TerminalReleaseParams {
    pub session_id: SessionId,
    pub terminal_id: String,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct TerminalReleaseResponse {}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct TerminalKillParams {
    pub session_id: SessionId,
    pub terminal_id: String,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct TerminalKillResponse {}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct TerminalWaitForExitParams {
    pub session_id: SessionId,
    pub terminal_id: String,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct TerminalWaitForExitResponse {
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub exit_code: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub signal: Option<String>,
}
```

- [ ] **Step 2: Write server notification types (session/update variants)**

Replace `crates/minos-acp-protocol/src/server_notification.rs`:

```rust
use crate::types::*;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct SessionUpdateNotification {
    pub session_id: SessionId,
    pub update: SessionUpdate,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(tag = "sessionUpdate", rename_all = "snake_case")]
pub enum SessionUpdate {
    AgentMessageChunk {
        content: ContentBlock,
    },
    ToolCall {
        tool_call_id: String,
        #[serde(skip_serializing_if = "Option::is_none", default)]
        title: Option<String>,
        kind: ToolCallKind,
        status: ToolCallStatus,
    },
    ToolCallUpdate {
        tool_call_id: String,
        status: ToolCallStatus,
        #[serde(skip_serializing_if = "Option::is_none", default)]
        content: Option<Vec<ToolCallContent>>,
    },
    Plan {
        entries: Vec<PlanEntry>,
    },
    Thought {
        content: ContentBlock,
    },
    CurrentModeUpdate {
        current_mode_id: SessionModeId,
        #[serde(skip_serializing_if = "Option::is_none", default)]
        available_modes: Option<Vec<SessionMode>>,
    },
    AvailableCommandsUpdate {
        commands: Vec<SlashCommand>,
    },
    SessionInfoUpdate {
        info: SessionInfo,
    },
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct PlanEntry {
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub priority: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub status: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct SlashCommand {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub description: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn agent_message_chunk_deserializes() {
        let json = r#"{
            "sessionUpdate": "agent_message_chunk",
            "content": { "type": "text", "text": "Hello" }
        }"#;
        let update: SessionUpdate = serde_json::from_str(json).unwrap();
        match update {
            SessionUpdate::AgentMessageChunk { content } => {
                assert_eq!(content, ContentBlock::Text { text: "Hello".into() });
            }
            _ => panic!("expected agent_message_chunk"),
        }
    }

    #[test]
    fn tool_call_update_completed_deserializes() {
        let json = r#"{
            "sessionUpdate": "tool_call_update",
            "tool_call_id": "tc_1",
            "status": "completed",
            "content": null
        }"#;
        let update: SessionUpdate = serde_json::from_str(json).unwrap();
        match update {
            SessionUpdate::ToolCallUpdate { tool_call_id, status, .. } => {
                assert_eq!(tool_call_id, "tc_1");
                assert_eq!(status, ToolCallStatus::Completed);
            }
            _ => panic!("expected tool_call_update"),
        }
    }

    #[test]
    fn plan_deserializes() {
        let json = r#"{
            "sessionUpdate": "plan",
            "entries": [
                { "content": "Step 1", "priority": "high", "status": "pending" }
            ]
        }"#;
        let update: SessionUpdate = serde_json::from_str(json).unwrap();
        match update {
            SessionUpdate::Plan { entries } => {
                assert_eq!(entries.len(), 1);
                assert_eq!(entries[0].content, "Step 1");
            }
            _ => panic!("expected plan"),
        }
    }
}
```

- [ ] **Step 3: Run tests**

Run: `cargo test -p minos-acp-protocol`
Expected: all tests pass

- [ ] **Step 4: Commit**

```bash
git add crates/minos-acp-protocol/
git commit -m "feat: add ACP server request/notification types (request_permission, session/update variants)"
```

---

## Task 5: Add MinosError variants for Gemini ACP

**Files:**
- Modify: `crates/minos-domain/src/error.rs`

- [ ] **Step 1: Add Gemini-specific error variants**

After the existing `CodexProtocolError` variant (around line 244), add:

```rust
    #[error("failed to spawn gemini: {message}")]
    GeminiSpawnFailed { message: String },

    #[error("gemini ACP protocol error on {method}: {message}")]
    AcpProtocolError { method: String, message: String },
```

- [ ] **Step 2: Verify compilation**

Run: `cargo check -p minos-domain`
Expected: compilation succeeds

- [ ] **Step 3: Commit**

```bash
git add crates/minos-domain/src/error.rs
git commit -m "feat: add GeminiSpawnFailed and AcpProtocolError error variants"
```

---

## Task 6: Implement AcpClient stdio pump

**Files:**
- Create: `crates/minos-agent-runtime/src/acp_client.rs`
- Modify: `crates/minos-agent-runtime/src/lib.rs`
- Modify: `crates/minos-agent-runtime/Cargo.toml`

- [ ] **Step 1: Add minos-acp-protocol dependency to Cargo.toml**

Add to `crates/minos-agent-runtime/Cargo.toml` under `[dependencies]`:

```toml
minos-acp-protocol = { path = "../minos-acp-protocol", version = "0.1.0" }
```

- [ ] **Step 2: Create acp_client.rs**

Create `crates/minos-agent-runtime/src/acp_client.rs`:

```rust
//! `AcpClient` — JSON-RPC 2.0 client over stdio for ACP agents.
//!
//! Architecture mirrors `CodexClient` (Option C — single-task writer):
//!
//! 1. `connect()` spawns the child process, wraps stdin/stdout in a pump task.
//! 2. Outbound writes flow over `mpsc` channel → pump serializes to stdin.
//! 3. Inbound frames from stdout are dispatched: responses to pending calls,
//!    notifications and server requests forwarded via `inbound_rx`.

use std::collections::HashMap;
use std::sync::Arc;

use minos_acp_protocol::AcpClientNotification;
use minos_acp_protocol::AcpClientRequest;
use minos_domain::MinosError;
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Child;
use tokio::sync::{mpsc, oneshot, Mutex};
use tokio::task::JoinHandle;
use tracing::{debug, warn};
use uuid::Uuid;

#[derive(Debug)]
pub(crate) enum Inbound {
    Notification {
        method: String,
        params: Value,
    },
    ServerRequest {
        id: Value,
        method: String,
        params: Value,
    },
    Closed,
}

enum Outbound {
    Request {
        method: String,
        params: Value,
        reply_to: oneshot::Sender<Result<Value, MinosError>>,
    },
    Notification {
        method: String,
        params: Value,
        ack: oneshot::Sender<Result<(), MinosError>>,
    },
    Reply {
        id: Value,
        result: Value,
        ack: oneshot::Sender<Result<(), MinosError>>,
    },
}

#[derive(Debug)]
pub(crate) struct AcpClient {
    outbound_tx: mpsc::Sender<Outbound>,
    inbound_rx: Arc<Mutex<mpsc::Receiver<Inbound>>>,
    pump_task: JoinHandle<()>,
    _child: Arc<tokio::sync::Mutex<Option<Child>>>,
}

impl Drop for AcpClient {
    fn drop(&mut self) {
        self.pump_task.abort();
    }
}

impl AcpClient {
    pub(crate) fn new(child: Child) -> Result<Self, MinosError> {
        let child = Arc::new(tokio::sync::Mutex::new(Some(child)));
        let (outbound_tx, outbound_rx) = mpsc::channel::<Outbound>(32);
        let (inbound_tx, inbound_rx) = mpsc::channel::<Inbound>(64);

        let stdin = child
            .try_lock()
            .ok()
            .and_then(|mut g| g.as_mut().and_then(|c| c.stdin.take()))
            .ok_or_else(|| MinosError::GeminiSpawnFailed {
                message: "could not take child stdin".into(),
            })?;

        let stdout = child
            .try_lock()
            .ok()
            .and_then(|mut g| g.as_mut().and_then(|c| c.stdout.take()))
            .ok_or_else(|| MinosError::GeminiSpawnFailed {
                message: "could not take child stdout".into(),
            })?;

        let pump_task = tokio::spawn(pump_loop(stdin, stdout, outbound_rx, inbound_tx));

        Ok(Self {
            outbound_tx,
            inbound_rx: Arc::new(Mutex::new(inbound_rx)),
            pump_task,
            _child: child,
        })
    }

    pub(crate) async fn call(&self, method: &str, params: Value) -> Result<Value, MinosError> {
        let (reply_to, rx) = oneshot::channel();
        self.outbound_tx
            .send(Outbound::Request {
                method: method.to_string(),
                params,
                reply_to,
            })
            .await
            .map_err(|_| MinosError::AcpProtocolError {
                method: method.to_string(),
                message: "acp client pump has shut down".into(),
            })?;
        rx.await.map_err(|_| MinosError::AcpProtocolError {
            method: method.to_string(),
            message: "acp client dropped the call response".into(),
        })?
    }

    pub(crate) async fn notify(&self, method: &str, params: Value) -> Result<(), MinosError> {
        let (ack, rx) = oneshot::channel();
        self.outbound_tx
            .send(Outbound::Notification {
                method: method.to_string(),
                params,
                ack,
            })
            .await
            .map_err(|_| MinosError::AcpProtocolError {
                method: method.to_string(),
                message: "acp client pump has shut down".into(),
            })?;
        rx.await.map_err(|_| MinosError::AcpProtocolError {
            method: method.to_string(),
            message: "acp client dropped the notification ack".into(),
        })?
    }

    pub(crate) async fn call_typed<R: AcpClientRequest>(
        &self,
        params: R,
    ) -> Result<R::Response, MinosError> {
        let value = serde_json::to_value(&params).map_err(|e| MinosError::AcpProtocolError {
            method: R::METHOD.into(),
            message: format!("encode params failed: {e}"),
        })?;
        let raw = self.call(R::METHOD, value).await?;
        serde_json::from_value::<R::Response>(raw).map_err(|e| MinosError::AcpProtocolError {
            method: R::METHOD.into(),
            message: format!("decode response failed: {e}"),
        })
    }

    pub(crate) async fn notify_typed<N: AcpClientNotification>(
        &self,
        params: N,
    ) -> Result<(), MinosError> {
        let value = serde_json::to_value(&params).map_err(|e| MinosError::AcpProtocolError {
            method: N::METHOD.into(),
            message: format!("encode notification params failed: {e}"),
        })?;
        self.notify(N::METHOD, value).await
    }

    pub(crate) async fn reply(&self, id: Value, result: Value) -> Result<(), MinosError> {
        let (ack, rx) = oneshot::channel();
        self.outbound_tx
            .send(Outbound::Reply { id, result, ack })
            .await
            .map_err(|_| MinosError::AcpProtocolError {
                method: "<reply>".into(),
                message: "acp client pump has shut down".into(),
            })?;
        rx.await.map_err(|_| MinosError::AcpProtocolError {
            method: "<reply>".into(),
            message: "acp client dropped the reply ack".into(),
        })?
    }

    pub(crate) async fn next_inbound(&self) -> Option<Inbound> {
        self.inbound_rx.lock().await.recv().await
    }
}

async fn pump_loop(
    stdin: tokio::process::ChildStdin,
    stdout: tokio::process::ChildStdout,
    mut outbound_rx: mpsc::Receiver<Outbound>,
    inbound_tx: mpsc::Sender<Inbound>,
) {
    let mut stdin_writer = stdin;
    let reader = BufReader::new(stdout);
    let mut lines = reader.lines();

    let mut pending: HashMap<String, oneshot::Sender<Result<Value, MinosError>>> = HashMap::new();

    loop {
        tokio::select! {
            biased;
            maybe_cmd = outbound_rx.recv() => {
                let Some(cmd) = maybe_cmd else {
                    break;
                };
                match cmd {
                    Outbound::Request { method, params, reply_to } => {
                        let id = Uuid::new_v4().to_string();
                        let frame = minos_acp_protocol::make_request(
                            serde_json::json!(id),
                            &method,
                            params,
                        );
                        let send_res = write_frame(&mut stdin_writer, &method, &frame).await;
                        if let Err(e) = send_res {
                            let _ = reply_to.send(Err(MinosError::AcpProtocolError {
                                method,
                                message: format!("stdin write failed: {e}"),
                            }));
                        } else {
                            pending.insert(id, reply_to);
                        }
                    }
                    Outbound::Notification { method, params, ack } => {
                        let frame = minos_acp_protocol::make_notification(&method, params);
                        let res = write_frame(&mut stdin_writer, &method, &frame).await;
                        let _ = ack.send(res);
                    }
                    Outbound::Reply { id, result, ack } => {
                        let frame = minos_acp_protocol::make_response(id, result);
                        let _ = ack.send(write_frame(&mut stdin_writer, "<reply>", &frame).await);
                    }
                }
            }
            maybe_line = lines.next_line() => {
                match maybe_line {
                    Ok(Some(line)) => {
                        handle_inbound_line(&line, &mut pending, &inbound_tx).await;
                    }
                    Ok(None) => {
                        debug!("gemini ACP stdout EOF");
                        for (_id, tx) in pending.drain() {
                            let _ = tx.send(Err(MinosError::AcpProtocolError {
                                method: "<pending>".into(),
                                message: "stdout closed before response".into(),
                            }));
                        }
                        let _ = inbound_tx.send(Inbound::Closed).await;
                        break;
                    }
                    Err(e) => {
                        warn!(error = %e, "gemini ACP stdout read error");
                        for (_id, tx) in pending.drain() {
                            let _ = tx.send(Err(MinosError::AcpProtocolError {
                                method: "<pending>".into(),
                                message: format!("stdout read error: {e}"),
                            }));
                        }
                        let _ = inbound_tx.send(Inbound::Closed).await;
                        break;
                    }
                }
            }
        }
    }
}

async fn write_frame(
    stdin: &mut tokio::process::ChildStdin,
    method: &str,
    frame: &Value,
) -> Result<(), MinosError> {
    let mut bytes = serde_json::to_string(frame).map_err(|e| MinosError::AcpProtocolError {
        method: method.to_string(),
        message: format!("serialize frame failed: {e}"),
    })?;
    bytes.push('\n');
    stdin.write_all(bytes.as_bytes()).await.map_err(|e| {
        MinosError::AcpProtocolError {
            method: method.to_string(),
            message: format!("stdin write failed: {e}"),
        }
    })?;
    stdin.flush().await.map_err(|e| MinosError::AcpProtocolError {
        method: method.to_string(),
        message: format!("stdin flush failed: {e}"),
    })?;
    Ok(())
}

async fn handle_inbound_line(
    line: &str,
    pending: &mut HashMap<String, oneshot::Sender<Result<Value, MinosError>>>,
    inbound_tx: &mpsc::Sender<Inbound>,
) {
    let Ok(value) = serde_json::from_str::<Value>(line) else {
        warn!(raw = %line, "gemini ACP sent malformed JSON; ignoring");
        return;
    };
    let id = value.get("id").cloned();
    let method = value
        .get("method")
        .and_then(Value::as_str)
        .map(str::to_string);
    let has_result_or_error = value.get("result").is_some() || value.get("error").is_some();

    match (id, method, has_result_or_error) {
        (Some(id_val), None, true) => {
            let key = match &id_val {
                Value::String(s) => s.clone(),
                Value::Number(n) => n.to_string(),
                _ => {
                    warn!(id = ?id_val, "response with non-string/non-number id; cannot dispatch");
                    return;
                }
            };
            let Some(tx) = pending.remove(&key) else {
                warn!(id = ?id_val, "response for unknown request id; dropping");
                return;
            };
            if let Some(err) = value.get("error") {
                let message = err
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or("ACP error without message")
                    .to_string();
                let _ = tx.send(Err(MinosError::AcpProtocolError {
                    method: "<response>".into(),
                    message,
                }));
            } else {
                let result = value.get("result").cloned().unwrap_or(Value::Null);
                let _ = tx.send(Ok(result));
            }
        }
        (Some(id_val), Some(method), false) => {
            let params = value.get("params").cloned().unwrap_or(Value::Null);
            let _ = inbound_tx
                .send(Inbound::ServerRequest {
                    id: id_val,
                    method,
                    params,
                })
                .await;
        }
        (None, Some(method), false) => {
            let params = value.get("params").cloned().unwrap_or(Value::Null);
            let _ = inbound_tx
                .send(Inbound::Notification { method, params })
                .await;
        }
        _ => {
            warn!(raw = %line, "gemini ACP sent ambiguous JSON-RPC frame; ignoring");
        }
    }
}
```

- [ ] **Step 3: Register module in lib.rs**

Add to `crates/minos-agent-runtime/src/lib.rs` after `pub(crate) mod codex_client;`:

```rust
pub(crate) mod acp_client;
pub mod gemini_driver;
```

- [ ] **Step 4: Verify compilation**

Run: `cargo check -p minos-agent-runtime`
Expected: compilation succeeds (gemini_driver doesn't exist yet, so we'll create a placeholder)

- [ ] **Step 5: Create placeholder gemini_driver.rs**

Create `crates/minos-agent-runtime/src/gemini_driver.rs`:

```rust
//! Gemini CLI ACP driver — manages `gemini --acp` lifecycle.
//!
//! Will be implemented in the next task.
```

- [ ] **Step 6: Verify compilation**

Run: `cargo check -p minos-agent-runtime`
Expected: compilation succeeds

- [ ] **Step 7: Commit**

```bash
git add crates/minos-agent-runtime/
git commit -m "feat: add AcpClient stdio pump and gemini_driver placeholder"
```

---

## Task 7: Implement gemini_driver.rs

**Files:**
- Modify: `crates/minos-agent-runtime/src/gemini_driver.rs`

- [ ] **Step 1: Implement GeminiAcpInstance**

Replace `crates/minos-agent-runtime/src/gemini_driver.rs`:

```rust
#![allow(unsafe_code)]

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use minos_acp_protocol::*;
use minos_domain::MinosError;
use serde_json::Value;
use tokio::process::{Child, Command};
use tokio::sync::{mpsc, Mutex};
use tokio::task::JoinHandle;
use tracing::{info, warn};

use crate::acp_client::AcpClient;
use crate::config::RawIngest;
use minos_domain::AgentName;

const KILL_ESCALATION: Duration = Duration::from_secs(3);

pub struct GeminiAcpInstance {
    pub workspace: PathBuf,
    pub child: Arc<tokio::sync::Mutex<Option<Child>>>,
    pub client: Arc<AcpClient>,
    pub session_id: Mutex<Option<String>>,
    pub spawned_at: std::time::Instant,
    pub last_activity_at: Mutex<std::time::Instant>,
    pub crash_signal: mpsc::Sender<()>,
}

impl GeminiAcpInstance {
    pub async fn spawn(
        cli_path: &Path,
        workspace: &Path,
        subprocess_env: &Arc<HashMap<String, String>>,
        crash_signal: mpsc::Sender<()>,
    ) -> Result<Self, MinosError> {
        let mut cmd = Command::new(cli_path);
        cmd.args(["--acp"])
            .current_dir(workspace)
            .env_clear()
            .envs(subprocess_env.iter())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);

        #[cfg(unix)]
        {
            unsafe {
                cmd.pre_exec(|| {
                    if libc::setpgid(0, 0) != 0 {
                        return Err(std::io::Error::last_os_error());
                    }
                    Ok(())
                });
            }
        }

        let child = cmd.spawn().map_err(|e| MinosError::GeminiSpawnFailed {
            message: format!("failed to spawn gemini --acp: {e}"),
        })?;

        let client = AcpClient::new(child)?;

        let now = std::time::Instant::now();
        Ok(Self {
            workspace: workspace.to_path_buf(),
            child: Arc::new(tokio::sync::Mutex::new(None)),
            client: Arc::new(client),
            session_id: Mutex::new(None),
            spawned_at: now,
            last_activity_at: Mutex::new(now),
            crash_signal,
        })
    }

    pub async fn initialize(&self) -> Result<InitializeResponse, MinosError> {
        self.client
            .call_typed(InitializeParams {
                protocol_version: 1,
                client_capabilities: Some(ClientCapabilities {
                    fs: FsCapabilities {
                        read_text_file: false,
                        write_text_file: false,
                    },
                    terminal: false,
                }),
                client_info: Some(Implementation {
                    name: "minos".into(),
                    title: Some("Minos Host".into()),
                    version: Some(env!("CARGO_PKG_VERSION").into()),
                }),
            })
            .await
    }

    pub async fn authenticate(&self, method_id: &str) -> Result<(), MinosError> {
        self.client
            .call_typed(AuthenticateParams {
                method_id: method_id.to_string(),
            })
            .await?;
        Ok(())
    }

    pub async fn new_session(&self, cwd: &Path) -> Result<NewSessionResponse, MinosError> {
        let resp = self
            .client
            .call_typed(NewSessionParams {
                cwd: cwd.to_string_lossy().to_string(),
                mcp_servers: vec![],
                additional_directories: None,
            })
            .await?;
        *self.session_id.lock().await = Some(resp.session_id.clone());
        Ok(resp)
    }

    pub async fn prompt(&self, text: &str) -> Result<PromptResponse, MinosError> {
        let session_id = self
            .session_id
            .lock()
            .await
            .clone()
            .ok_or_else(|| MinosError::AcpProtocolError {
                method: "session/prompt".into(),
                message: "no active session".into(),
            })?;
        self.client
            .call_typed(PromptParams {
                session_id,
                prompt: vec![ContentBlock::Text {
                    text: text.to_string(),
                }],
            })
            .await
    }

    pub async fn cancel(&self) -> Result<(), MinosError> {
        let session_id = self
            .session_id
            .lock()
            .await
            .clone()
            .ok_or_else(|| MinosError::AcpProtocolError {
                method: "session/cancel".into(),
                message: "no active session".into(),
            })?;
        self.client
            .notify_typed(CancelNotification { session_id })
            .await
    }

    pub async fn close_session(&self) -> Result<(), MinosError> {
        let session_id = self
            .session_id
            .lock()
            .await
            .clone()
            .ok_or_else(|| MinosError::AcpProtocolError {
                method: "session/close".into(),
                message: "no active session".into(),
            })?;
        self.client
            .call_typed(CloseSessionParams { session_id })
            .await?;
        *self.session_id.lock().await = None;
        Ok(())
    }

    pub async fn touch(&self) {
        *self.last_activity_at.lock().await = std::time::Instant::now();
    }

    pub async fn get_session_id(&self) -> Option<String> {
        self.session_id.lock().await.clone()
    }
}

pub fn spawn_acp_pump(
    client: Arc<AcpClient>,
    thread_id: String,
    events_tx: tokio::sync::broadcast::Sender<RawIngest>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            match client.next_inbound().await {
                Some(crate::acp_client::Inbound::Notification { method, params }) => {
                    let _ = events_tx.send(RawIngest {
                        agent: AgentName::Gemini,
                        thread_id: thread_id.clone(),
                        payload: serde_json::json!({
                            "kind": "acp_notification",
                            "method": method,
                            "params": params,
                        }),
                        ts_ms: chrono::Utc::now().timestamp_millis(),
                    });
                }
                Some(crate::acp_client::Inbound::ServerRequest { id, method, params }) => {
                    let _ = events_tx.send(RawIngest {
                        agent: AgentName::Gemini,
                        thread_id: thread_id.clone(),
                        payload: serde_json::json!({
                            "kind": "acp_server_request",
                            "id": id,
                            "method": method,
                            "params": params,
                        }),
                        ts_ms: chrono::Utc::now().timestamp_millis(),
                    });
                }
                Some(crate::acp_client::Inbound::Closed) => {
                    info!(
                        target: "minos_agent_runtime::gemini_driver",
                        thread_id = %thread_id,
                        "gemini ACP stream closed"
                    );
                    let _ = events_tx.send(RawIngest {
                        agent: AgentName::Gemini,
                        thread_id: thread_id.clone(),
                        payload: serde_json::json!({
                            "kind": "acp_closed",
                            "thread_id": thread_id,
                        }),
                        ts_ms: chrono::Utc::now().timestamp_millis(),
                    });
                    break;
                }
                None => break,
            }
        }
    })
}
```

- [ ] **Step 2: Verify compilation**

Run: `cargo check -p minos-agent-runtime`
Expected: compilation succeeds

- [ ] **Step 3: Commit**

```bash
git add crates/minos-agent-runtime/src/gemini_driver.rs
git commit -m "feat: implement GeminiAcpInstance with ACP lifecycle management"
```

---

## Task 8: Rewrite gemini.rs translator for full ACP v1

**Files:**
- Modify: `crates/minos-ui-protocol/src/gemini.rs`
- Modify: `crates/minos-ui-protocol/src/lib.rs`

- [ ] **Step 1: Rewrite gemini.rs translator**

Replace `crates/minos-ui-protocol/src/gemini.rs`:

```rust
use crate::error::TranslationError;
use crate::message::{MessageRole, ThreadEndReason, UiEventMessage};
use minos_domain::AgentName;
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use uuid::Uuid;

pub struct GeminiTranslatorState {
    thread_id: String,
    session_id: Option<String>,
    open_assistant_message_id: Option<String>,
    open_user_message_id: Option<String>,
    emitted_message_ids: HashSet<String>,
    tool_calls: HashMap<String, OpenGeminiToolCall>,
}

struct OpenGeminiToolCall {
    message_id: String,
    name: String,
}

impl GeminiTranslatorState {
    #[must_use]
    pub fn new(thread_id: String) -> Self {
        Self {
            thread_id,
            session_id: None,
            open_assistant_message_id: None,
            open_user_message_id: None,
            emitted_message_ids: HashSet::new(),
            tool_calls: HashMap::new(),
        }
    }
}

#[allow(clippy::too_many_lines)]
pub fn translate(
    state: &mut GeminiTranslatorState,
    raw: &Value,
) -> Result<Vec<UiEventMessage>, TranslationError> {
    let kind = raw
        .get("kind")
        .and_then(Value::as_str)
        .ok_or_else(|| TranslationError::Malformed {
            reason: "missing kind field".into(),
        })?;

    match kind {
        "acp_notification" => translate_acp_notification(state, raw),
        "acp_server_request" => translate_acp_server_request(state, raw),
        "acp_closed" => {
            let mut events = Vec::new();
            if let Some(mid) = state.open_assistant_message_id.take() {
                events.push(UiEventMessage::MessageCompleted {
                    message_id: mid,
                    finished_at_ms: chrono::Utc::now().timestamp_millis(),
                });
            }
            events.push(UiEventMessage::ThreadClosed {
                thread_id: state.thread_id.clone(),
                reason: ThreadEndReason::AgentDone,
                closed_at_ms: chrono::Utc::now().timestamp_millis(),
            });
            Ok(events)
        }
        other => Ok(vec![UiEventMessage::Raw {
            kind: format!("gemini/{other}"),
            payload_json: serde_json::to_string(raw).unwrap_or_default(),
        }]),
    }
}

fn translate_acp_notification(
    state: &mut GeminiTranslatorState,
    raw: &Value,
) -> Result<Vec<UiEventMessage>, TranslationError> {
    let params = raw.get("params").cloned().unwrap_or(Value::Null);
    let update = params.get("update").cloned().unwrap_or(Value::Null);
    let session_update = update
        .get("sessionUpdate")
        .and_then(Value::as_str)
        .unwrap_or("");

    match session_update {
        "agent_message_chunk" => {
            let content = update.get("content").cloned().unwrap_or(Value::Null);
            let content_type = content
                .get("type")
                .and_then(Value::as_str)
                .unwrap_or("text");

            let mid = state
                .open_assistant_message_id
                .clone()
                .unwrap_or_else(|| {
                    let id = format!("msg_{}", Uuid::new_v4());
                    state.open_assistant_message_id = Some(id.clone());
                    id
                });

            let mut events = Vec::new();

            if state.emitted_message_ids.insert(mid.clone()) {
                events.push(UiEventMessage::MessageStarted {
                    message_id: mid.clone(),
                    role: MessageRole::Assistant,
                    started_at_ms: 0,
                });
            }

            match content_type {
                "text" => {
                    let text = content
                        .get("text")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string();
                    if !text.is_empty() {
                        events.push(UiEventMessage::TextDelta {
                            message_id: mid,
                            text,
                        });
                    }
                }
                "thought" => {
                    let text = content
                        .get("text")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string();
                    if !text.is_empty() {
                        events.push(UiEventMessage::ReasoningDelta {
                            message_id: mid,
                            text,
                        });
                    }
                }
                _ => {
                    events.push(UiEventMessage::Raw {
                        kind: format!("gemini/agent_message_chunk/{content_type}"),
                        payload_json: serde_json::to_string(&update).unwrap_or_default(),
                    });
                }
            }

            Ok(events)
        }
        "tool_call" => {
            let tool_call_id = update
                .get("tool_call_id")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            let title = update
                .get("title")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            let kind = update
                .get("kind")
                .and_then(Value::as_str)
                .unwrap_or("other")
                .to_string();

            let mid = state
                .open_assistant_message_id
                .clone()
                .unwrap_or_default();

            if !tool_call_id.is_empty() {
                state.tool_calls.insert(
                    tool_call_id.clone(),
                    OpenGeminiToolCall {
                        message_id: mid.clone(),
                        name: title.clone(),
                    },
                );
                Ok(vec![UiEventMessage::ToolCallPlaced {
                    message_id: mid,
                    tool_call_id,
                    name: format!("{kind}: {title}"),
                    args_json: String::new(),
                }])
            } else {
                Ok(vec![])
            }
        }
        "tool_call_update" => {
            let tool_call_id = update
                .get("tool_call_id")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            let status = update
                .get("status")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();

            if status == "completed" {
                let content = update
                    .get("content")
                    .and_then(|c| {
                        if let Some(arr) = c.as_array() {
                            arr.iter()
                                .filter_map(|item| {
                                    item.get("content")
                                        .and_then(|inner| inner.get("text"))
                                        .and_then(Value::as_str)
                                })
                                .collect::<Vec<_>>()
                                .join("\n")
                                .into()
                        } else {
                            c.as_str().map(str::to_string)
                        }
                    })
                    .unwrap_or_default();

                state.tool_calls.remove(&tool_call_id);
                Ok(vec![UiEventMessage::ToolCallCompleted {
                    tool_call_id,
                    output: content,
                    is_error: false,
                }])
            } else {
                Ok(vec![])
            }
        }
        "plan" => Ok(vec![UiEventMessage::Raw {
            kind: "gemini/plan".into(),
            payload_json: serde_json::to_string(&update).unwrap_or_default(),
        }]),
        "thought" => {
            let content = update.get("content").cloned().unwrap_or(Value::Null);
            let text = content
                .get("text")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            let mid = state.open_assistant_message_id.clone().unwrap_or_default();
            if !text.is_empty() {
                Ok(vec![UiEventMessage::ReasoningDelta {
                    message_id: mid,
                    text,
                }])
            } else {
                Ok(vec![])
            }
        }
        "current_mode_update" => Ok(vec![UiEventMessage::Raw {
            kind: "gemini/mode_change".into(),
            payload_json: serde_json::to_string(&update).unwrap_or_default(),
        }]),
        "available_commands_update" => Ok(vec![UiEventMessage::Raw {
            kind: "gemini/commands_update".into(),
            payload_json: serde_json::to_string(&update).unwrap_or_default(),
        }]),
        "session_info_update" => Ok(vec![UiEventMessage::Raw {
            kind: "gemini/session_info".into(),
            payload_json: serde_json::to_string(&update).unwrap_or_default(),
        }]),
        "" => Ok(vec![]),
        other => Ok(vec![UiEventMessage::Raw {
            kind: format!("gemini/acp/{other}"),
            payload_json: serde_json::to_string(&update).unwrap_or_default(),
        }]),
    }
}

fn translate_acp_server_request(
    state: &mut GeminiTranslatorState,
    raw: &Value,
) -> Result<Vec<UiEventMessage>, TranslationError> {
    let method = raw
        .get("method")
        .and_then(Value::as_str)
        .unwrap_or("");

    match method {
        "session/request_permission" => {
            let params = raw.get("params").cloned().unwrap_or(Value::Null);
            let tool_call = params.get("tool_call").cloned().unwrap_or(Value::Null);
            let tool_call_id = tool_call
                .get("tool_call_id")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            let title = tool_call
                .get("title")
                .and_then(Value::as_str)
                .unwrap_or("Tool execution")
                .to_string();
            let kind = tool_call
                .get("kind")
                .and_then(Value::as_str)
                .unwrap_or("other")
                .to_string();

            let mid = state
                .open_assistant_message_id
                .clone()
                .unwrap_or_default();

            if !tool_call_id.is_empty() {
                state.tool_calls.insert(
                    tool_call_id.clone(),
                    OpenGeminiToolCall {
                        message_id: mid.clone(),
                        name: title.clone(),
                    },
                );
                Ok(vec![UiEventMessage::ToolCallPlaced {
                    message_id: mid,
                    tool_call_id,
                    name: format!("{kind}: {title}"),
                    args_json: String::new(),
                }])
            } else {
                Ok(vec![UiEventMessage::Raw {
                    kind: "gemini/permission_request".into(),
                    payload_json: serde_json::to_string(raw).unwrap_or_default(),
                }])
            }
        }
        "fs/read_text_file" | "fs/write_text_file" => {
            Ok(vec![UiEventMessage::Raw {
                kind: format!("gemini/{method}"),
                payload_json: serde_json::to_string(raw).unwrap_or_default(),
            }])
        }
        _ => Ok(vec![UiEventMessage::Raw {
            kind: format!("gemini/server_request/{method}"),
            payload_json: serde_json::to_string(raw).unwrap_or_default(),
        }]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::*;
    use pretty_assertions::assert_eq;

    fn val(s: &str) -> serde_json::Value {
        serde_json::from_str(s).unwrap()
    }

    #[test]
    fn acp_notification_agent_message_chunk_text() {
        let mut s = GeminiTranslatorState::new("thr_x".into());
        let raw = val(r#"{
            "kind": "acp_notification",
            "params": {
                "update": {
                    "sessionUpdate": "agent_message_chunk",
                    "content": { "type": "text", "text": "Hello" }
                }
            }
        }"#);
        let out = translate(&mut s, &raw).unwrap();
        assert!(out.iter().any(|e| matches!(
            e,
            UiEventMessage::MessageStarted { role: MessageRole::Assistant, .. }
        )));
        assert!(out.iter().any(|e| matches!(
            e,
            UiEventMessage::TextDelta { text, .. } if text == "Hello"
        )));
    }

    #[test]
    fn acp_notification_thought_emits_reasoning_delta() {
        let mut s = GeminiTranslatorState::new("thr".into());
        let _ = translate(&mut s, &val(r#"{
            "kind": "acp_notification",
            "params": { "update": { "sessionUpdate": "agent_message_chunk", "content": { "type": "text", "text": "" } } }
        }"#)).unwrap();
        let out = translate(&mut s, &val(r#"{
            "kind": "acp_notification",
            "params": { "update": { "sessionUpdate": "thought", "content": { "type": "text", "text": "thinking..." } } }
        }"#)).unwrap();
        assert!(out.iter().any(|e| matches!(
            e,
            UiEventMessage::ReasoningDelta { text, .. } if text == "thinking..."
        )));
    }

    #[test]
    fn acp_notification_tool_call_placed() {
        let mut s = GeminiTranslatorState::new("thr".into());
        let _ = translate(&mut s, &val(r#"{
            "kind": "acp_notification",
            "params": { "update": { "sessionUpdate": "agent_message_chunk", "content": { "type": "text", "text": "" } } }
        }"#)).unwrap();
        let out = translate(&mut s, &val(r#"{
            "kind": "acp_notification",
            "params": { "update": { "sessionUpdate": "tool_call", "tool_call_id": "tc_1", "title": "Edit file", "kind": "edit", "status": "pending" } }
        }"#)).unwrap();
        assert!(out.iter().any(|e| matches!(
            e,
            UiEventMessage::ToolCallPlaced { tool_call_id, name, .. }
                if tool_call_id == "tc_1" && name.contains("Edit file")
        )));
    }

    #[test]
    fn acp_notification_tool_call_update_completed() {
        let mut s = GeminiTranslatorState::new("thr".into());
        let _ = translate(&mut s, &val(r#"{
            "kind": "acp_notification",
            "params": { "update": { "sessionUpdate": "agent_message_chunk", "content": { "type": "text", "text": "" } } }
        }"#)).unwrap();
        let _ = translate(&mut s, &val(r#"{
            "kind": "acp_notification",
            "params": { "update": { "sessionUpdate": "tool_call", "tool_call_id": "tc_1", "title": "Edit", "kind": "edit", "status": "pending" } }
        }"#)).unwrap();
        let out = translate(&mut s, &val(r#"{
            "kind": "acp_notification",
            "params": { "update": { "sessionUpdate": "tool_call_update", "tool_call_id": "tc_1", "status": "completed", "content": null } }
        }"#)).unwrap();
        assert!(out.iter().any(|e| matches!(
            e,
            UiEventMessage::ToolCallCompleted { tool_call_id, is_error: false, .. }
                if tool_call_id == "tc_1"
        )));
    }

    #[test]
    fn acp_closed_emits_thread_closed() {
        let mut s = GeminiTranslatorState::new("thr".into());
        let _ = translate(&mut s, &val(r#"{
            "kind": "acp_notification",
            "params": { "update": { "sessionUpdate": "agent_message_chunk", "content": { "type": "text", "text": "hi" } } }
        }"#)).unwrap();
        let out = translate(&mut s, &val(r#"{ "kind": "acp_closed" }"#)).unwrap();
        assert!(out.iter().any(|e| matches!(
            e,
            UiEventMessage::MessageCompleted { .. }
        )));
        assert!(out.iter().any(|e| matches!(
            e,
            UiEventMessage::ThreadClosed { reason: ThreadEndReason::AgentDone, .. }
        )));
    }

    #[test]
    fn acp_notification_plan_emits_raw() {
        let mut s = GeminiTranslatorState::new("thr".into());
        let out = translate(&mut s, &val(r#"{
            "kind": "acp_notification",
            "params": { "update": { "sessionUpdate": "plan", "entries": [{ "content": "Step 1", "priority": "high", "status": "pending" }] } }
        }"#)).unwrap();
        assert!(out.iter().any(|e| matches!(
            e,
            UiEventMessage::Raw { kind, .. } if kind == "gemini/plan"
        )));
    }

    #[test]
    fn acp_server_request_permission_emits_tool_call_placed() {
        let mut s = GeminiTranslatorState::new("thr".into());
        let _ = translate(&mut s, &val(r#"{
            "kind": "acp_notification",
            "params": { "update": { "sessionUpdate": "agent_message_chunk", "content": { "type": "text", "text": "" } } }
        }"#)).unwrap();
        let out = translate(&mut s, &val(r#"{
            "kind": "acp_server_request",
            "method": "session/request_permission",
            "params": { "tool_call": { "tool_call_id": "tc_perm", "title": "Run shell", "kind": "terminal", "status": "pending" }, "options": [] }
        }"#)).unwrap();
        assert!(out.iter().any(|e| matches!(
            e,
            UiEventMessage::ToolCallPlaced { tool_call_id, name, .. }
                if tool_call_id == "tc_perm" && name.contains("Run shell")
        )));
    }

    #[test]
    fn unknown_session_update_falls_through_to_raw() {
        let mut s = GeminiTranslatorState::new("thr".into());
        let out = translate(&mut s, &val(r#"{
            "kind": "acp_notification",
            "params": { "update": { "sessionUpdate": "custom_event", "data": "something" } }
        }"#)).unwrap();
        assert!(out.iter().any(|e| matches!(
            e,
            UiEventMessage::Raw { kind, .. } if kind == "gemini/acp/custom_event"
        )));
    }
}
```

- [ ] **Step 2: Update lib.rs exports**

Replace the gemini line in `crates/minos-ui-protocol/src/lib.rs`:

```rust
pub use gemini::{translate as translate_gemini, GeminiTranslatorState};
```

- [ ] **Step 3: Update translate_stateless for Gemini**

In `crates/minos-ui-protocol/src/lib.rs`, update the `Gemini` branch in `translate_stateless`:

```rust
        AgentKind::Gemini => {
            let mut s = GeminiTranslatorState::new(String::new());
            translate_gemini(&mut s, raw_payload)
        }
```

- [ ] **Step 4: Run tests**

Run: `cargo test -p minos-ui-protocol`
Expected: all tests pass

- [ ] **Step 5: Commit**

```bash
git add crates/minos-ui-protocol/
git commit -m "feat: rewrite gemini.rs translator for full ACP v1 event coverage"
```

---

## Task 9: Integration verification

**Files:** None new — verification only

- [ ] **Step 1: Full workspace compilation check**

Run: `cargo check --workspace`
Expected: no errors

- [ ] **Step 2: Run all tests**

Run: `cargo test --workspace`
Expected: all existing + new tests pass

- [ ] **Step 3: Lint check**

Run: `cargo clippy --workspace`
Expected: no new warnings

- [ ] **Step 4: Commit any remaining fixes**

```bash
git add -A
git commit -m "chore: fix lint/test issues from Gemini ACP integration"
```

---

## Self-Review Checklist

**1. Spec coverage:** Each section of the design doc is covered:
- minos-acp-protocol crate → Tasks 1-4
- AcpClient stdio pump → Task 6
- gemini_driver.rs → Task 7
- gemini.rs translator → Task 8
- MinosError variants → Task 5
- Error handling → covered in each task
- File changes summary → all files listed in design doc are created/modified

**2. Placeholder scan:** No TBD/TODO found. All steps contain complete code.

**3. Type consistency:** All type names (`AcpClientRequest`, `InitializeParams`, `ContentBlock`, etc.) are defined in earlier tasks and referenced consistently in later tasks. Method names (`session/new`, `session/prompt`, etc.) match between protocol types and driver code.
