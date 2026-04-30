# Minos · Codex App-Server Typed Protocol — Design

| Field | Value |
|---|---|
| Status | Draft (in review) |
| Last updated | 2026-04-30 |
| Owner | fannnzhang |
| Parent spec | `docs/superpowers/specs/codex-app-server-integration-design.md` |
| Target plan | `docs/superpowers/plans/NN-codex-typed-protocol.md` (NN assigned at plan time) |
| Predecessors | Plan 04 (`docs/superpowers/plans/04-codex-app-server-integration.md`) — codex bridge in production |
| Supersedes | §10.1 (wire format), §6.4 (approval method names) of `codex-app-server-integration-design.md` |

---

## 1. Context

Plan 04 landed `minos-agent-runtime` with a hand-rolled JSON-RPC 2.0 client speaking codex's `app-server` stdio dialect. Every outbound call (`initialize`, `thread/start`, `turn/start`, `turn/interrupt`, `thread/archive`) is constructed via `serde_json::json!({ ... })`, every inbound notification is forwarded verbatim as `RawIngest { payload: Value }`, and approval `ServerRequest`s are matched against a hand-maintained string list (`APPROVAL_METHODS: &[&str]`).

OpenAI now publishes the codex app-server protocol as a JSON Schema set ([developers.openai.com/codex/app-server](https://developers.openai.com/codex/app-server)). The maintainer ran `codex app-server generate-json-schema --out ./schemas` and dropped the output into `schemas/` — about 200 schema files covering 71 client requests, 9 server requests, 58 server notifications, 1 client notification, plus the JSON-RPC envelope types. The current state of the repo (`git status` shows `?? schemas/`) is the trigger for this design.

Three concrete defects motivate the work:

1. **Approval method names have drifted.** `approvals.rs` lists `ApplyPatchApproval` / `ExecCommandApproval` / `FileChangeRequestApproval` / `PermissionsRequestApproval` / `CommandExecutionRequestApproval`. The v2 schema names are `item/commandExecution/requestApproval` / `item/fileChange/requestApproval` / `item/permissions/requestApproval` (namespaced). When codex emits the v2 names today, the runtime's `event_pump_loop` treats them as unknown server requests, fails to auto-reject, and merely warns. The approval-free contract the bridge depends on is silently broken.
2. **Auto-reject payload shape is non-conformant.** `build_auto_reject` always returns `{ "decision": "rejected" }`. The schemas define four distinct response shapes, and none accept the literal string `"rejected"`:
   - `ApplyPatchApprovalResponse` / `ExecCommandApprovalResponse` → `decision: ReviewDecision` (enum: `approved`, `approved_for_session`, `denied`, `timed_out`, `abort`).
   - `CommandExecutionRequestApprovalResponse` → `decision: CommandExecutionApprovalDecision` (enum: `accept`, `acceptForSession`, `decline`, `cancel`).
   - `FileChangeRequestApprovalResponse` → `decision: FileChangeApprovalDecision` (enum: `accept`, `acceptForSession`, `decline`, `cancel`).
   - `PermissionsRequestApprovalResponse` → `permissions: GrantedPermissionProfile` (no `decision` field at all; "reject" equates to granting empty permissions).
   Codex's behavior on the current malformed payload is undefined; the typed rewrite picks the right schema variant per response.
3. **`thread_id_from_response`** in `runtime.rs` (line 838) probes three different field shapes (`thread_id`, `threadId`, `thread.id`) because the original implementation predated the schema. The schema is now explicit: `ThreadStartResponse.thread.id: String` (required). The defensive heuristic is dead code that obscures the contract.

The fix is a typed protocol layer derived directly from the schemas, plus a runtime refactor that uses it.

---

## 2. Goals

### 2.1 In scope

1. New workspace crate **`minos-codex-protocol`** — vendored typify-generated Rust types for all 200 schema files, plus hand-written `ClientRequest` and `ClientNotification` traits, plus xtask-generated `ClientRequestMethod` / `ServerRequest` / `ServerNotification` enums and per-method trait impls covering every `oneOf` arm in `schemas/{ClientRequest,ClientNotification,ServerRequest,ServerNotification}.json`.
2. `xtask` subcommand **`gen-codex-protocol`** that runs typify over `schemas/`, post-processes the four union schemas to emit trait impls and tagged enums, runs `cargo fmt`, and writes the result to `crates/minos-codex-protocol/src/generated/`.
3. Refactor of `minos-agent-runtime`:
   - `codex_client.rs` gains a `call_typed<R: ClientRequest>(params: R) -> R::Response` helper, plus a `notify_typed<N: ClientNotification>(params: N) -> ()` helper. Existing `call`/`notify`/`reply` retained as escape hatches.
   - `runtime.rs` replaces all 5 hand-rolled `client.call("method", json!({...}))` sites with `client.call_typed(params)`, plus the 1 `client.notify("initialized", Value::Null)` site with `client.notify_typed(InitializedNotification {})`.
   - `runtime.rs` deletes `thread_id_from_response`; `start_response.thread.id` becomes the single source.
   - `approvals.rs` is rewritten: the `APPROVAL_METHODS: &[&str]` list is removed; `auto_reject` now takes a typed `&ServerRequest` and returns `Option<serde_json::Value>` carrying the typed rejection response.
   - `event_pump_loop` decodes inbound `Inbound::ServerRequest` into `ServerRequest`, dispatches via `auto_reject`, and falls back to a warn-log on deserialization failure.
4. ADR `docs/adr/0011-codex-protocol-typed-codegen.md` recording the codegen choice.
5. Tests:
   - In `minos-codex-protocol`: round-trip fixtures for representative schema instances, completeness check tying the three union enums to their schema `oneOf` lists, method-string check tying every `impl ClientRequest` constant to its schema entry.
   - In `minos-agent-runtime`: `test_support::FakeCodexServer` migrated to construct frames via typed structs; `approvals` unit tests rewritten against typed `ServerRequest` variants; new `event_pump_loop` tests for typed dispatch and unknown-method warn paths.

### 2.2 Out of scope

| Item | Deferred to |
|---|---|
| Migrating `RawIngest.payload` from `serde_json::Value` to typed `ServerNotification` | Backend-translator-typed-rewrite spec (chat UI follow-up) |
| Auto-approve / configurable approval policy (still always reject) | Approval-policy spec, future P2 |
| Wiring chat UI to typed `thread/resume` / `thread/fork` / `turn/steer` / `fuzzyFileSearch` / `mcp/*` methods | `streaming-chat-ui-design.md` (next P1 spec) |
| Automating `codex app-server generate-json-schema` from `cargo xtask` (refresh subcommand) | Captured here as future work; not landed this phase |
| Mobile / FFI surface changes | None planned; this is internal to the daemon |
| `minos-protocol` (daemon RPC surface) changes | None; orthogonal layer |

### 2.3 Testing philosophy (inherited)

Per project memory: unit tests cover logic only; UI/widget/functional tests are integration concerns and deferred. New test coverage stays in Rust `cargo test` only.

---

## 3. Feasibility Assessment

The change is **fully feasible**. Concrete evidence:

- The transport layer (`codex_client.rs`'s stdio pump + mpsc machinery) carries opaque `serde_json::Value`s end-to-end and is agnostic to the wire schema. Adding `call_typed` is a 20-line additive method; no behavior change to `call`/`notify`/`reply`/the pump loop.
- All 6 outbound JSON-RPC sites in `runtime.rs` are 1:1 with schema-defined methods (5 calls: `initialize`, `thread/start`, `turn/start`, `turn/interrupt`, `thread/archive`; 1 notification: `initialized`). No method is multi-shape or version-conditional.
- `approvals.rs` is a single-file module with 5 hardcoded method names and one builder function. Replacement is a focused rewrite, not a sweep.
- typify (Oxide Computer) supports JSON Schema draft-07, `oneOf` discriminator unions, `$ref` resolution, and `additionalProperties: true` (falling back to `serde_json::Value`). All constructs in `schemas/` are within typify's supported set.
- The four union schemas (`ClientRequest.json`, `ClientNotification.json`, `ServerRequest.json`, `ServerNotification.json`) follow a uniform shape per `oneOf` arm: `{ method: { enum: ["xxx"] }, params?: { $ref: "#/definitions/XxxParams" } }` (the `params` field is omitted only for the parameterless `initialized` notification). This is mechanically parseable by a small Rust post-processor using `serde_json` + `quote`.
- The existing test infrastructure (`FakeCodexServer` over a duplex pair) already mediates between the runtime and a scripted JSON-RPC peer. Migrating the fake's framing to typed types is local to `test_support.rs`.

---

## 4. Current Surface Inventory

Touched APIs and call sites (relative to repo root):

- `crates/minos-agent-runtime/src/codex_client.rs:185` — `CodexClient::call(method, params)` — opaque outbound request. Retained, plus typed wrapper added.
- `crates/minos-agent-runtime/src/codex_client.rs:205` — `CodexClient::notify(method, params)` — fire-and-forget notification. Retained as-is.
- `crates/minos-agent-runtime/src/codex_client.rs:226` — `CodexClient::reply(id, result)` — outbound reply to server request. Retained as-is.
- `crates/minos-agent-runtime/src/runtime.rs:418-451` — handshake: `client.call("initialize", ...)`, `client.notify("initialized", Value::Null)`, `client.call("thread/start", ...)`. All three sites hand-rolled with `serde_json::json!`. Migrating to `call_typed` × 2 and `notify_typed` × 1.
- `crates/minos-agent-runtime/src/runtime.rs:564-571` — `send_user_message`'s `turn/start` call hand-rolled. Migrating to typed.
- `crates/minos-agent-runtime/src/runtime.rs:721-730` — `stop`'s `turn/interrupt` + `thread/archive` polite goodbye calls hand-rolled. Migrating to typed.
- `crates/minos-agent-runtime/src/runtime.rs:825-849` — `initialize_params()`, `thread_id_from_response()`, `thread_id_request_params()` helpers. First two replaced; third becomes a typed param construction inline.
- `crates/minos-agent-runtime/src/runtime.rs:986-988` — `event_pump_loop` server-request dispatch keying off `APPROVAL_METHODS.contains(&method.as_str())`. Replaced with typed `ServerRequest` decode.
- `crates/minos-agent-runtime/src/approvals.rs` — entire module; `APPROVAL_METHODS` constant + `build_auto_reject(request_id, method)` function. Both removed; replaced with typed-enum-driven `auto_reject(&ServerRequest)`.
- `crates/minos-agent-runtime/src/test_support.rs` — `FakeCodexServer` framing (currently uses `serde_json::json!`). Migrating to typed structs from `minos-codex-protocol`.
- `crates/minos-agent-runtime/tests/runtime_e2e.rs` — references `FakeCodexServer`; behavior preserved, types upgraded transitively.
- `crates/minos-agent-runtime/Cargo.toml` — `[dependencies]` gains `minos-codex-protocol`.
- `crates/minos-agent-runtime/src/lib.rs` — re-export `build_auto_reject` (currently `pub use approvals::build_auto_reject`) is removed. Workspace grep at spec-authoring time: no other crate imports the symbol; safe to delete. The public surface of the runtime crate stays additive otherwise.

Schema-side authoritative inputs (read-only by codegen):

- `schemas/codex_app_server_protocol.v2.schemas.json` — master v2 root; resolved by typify via $ref into all `schemas/v2/*Params.json` / `*Response.json` / `*Notification.json` files.
- `schemas/codex_app_server_protocol.schemas.json` — root v1+v2 hybrid; supplies `InitializeParams` / `InitializeResponse` (v1) and the JSON-RPC envelope types.
- `schemas/ClientRequest.json` — `oneOf` listing all 71 client requests with `method` ↔ `params` ↔ response associations.
- `schemas/ServerRequest.json` — `oneOf` listing 9 server-initiated requests.
- `schemas/ServerNotification.json` — `oneOf` listing 58 server-side notifications.
- `schemas/ClientNotification.json` — `oneOf` listing client notifications (currently only `initialized`).

---

## 5. Design

### 5.1 Key design decisions

1. **typify with vendored output, not a build script.** Rejected: build-script codegen (output invisible in PR diffs, schema drift hidden from review). Rejected: hand-written types (200 schemas, drift inevitable). Chosen: `cargo xtask gen-codex-protocol` regenerates `crates/minos-codex-protocol/src/generated/` and the result is committed to git. Schema upgrades become a two-step PR: refresh `schemas/`, regenerate, commit both.
2. **Trait-based `ClientRequest` API, not per-method functions.** Rejected: 71 hand-written `client.thread_start(params).await` shims (boilerplate, schema additions force code edits). Rejected: a single match-style enum API (poor ergonomics for response extraction). Chosen: `pub trait ClientRequest { const METHOD: &'static str; type Response: DeserializeOwned; }` with auto-generated `impl` blocks per method. Adding a new client request becomes "regenerate codegen", zero hand edits.
3. **Three serde-tagged enums for inbound dispatch.** `ClientRequestMethod`, `ServerRequest`, `ServerNotification` are auto-generated as `#[serde(tag = "method", content = "params")]` enums mirroring the schema unions. Used for typed inbound dispatch at the `event_pump_loop` boundary; not used for outbound (outbound goes through the trait).
4. **`RawIngest` keeps `serde_json::Value`.** Rejected: replacing payload with typed `ServerNotification`. Reason: backend already runs a JSON-driven translator pipeline; converting that is its own scope. Consumers wanting typed access can `serde_json::from_value::<ServerNotification>(payload)` themselves — the crate publishes the enum.
5. **Approval auto-reject driven by typed enum match, not string list.** The `APPROVAL_METHODS` array is a known drift hazard (already wrong). The new `auto_reject(&ServerRequest) -> Option<Value>` is exhaustive over the typed `ServerRequest` enum: the match's missing arms become a compile error the next time a schema adds a server request method. Schema drift becomes a build break, not a silent runtime hole.
6. **`minos-codex-protocol` has zero dependencies on other minos crates.** Rejected: living inside `minos-protocol` (which is the daemon's RPC surface — orthogonal concern; would couple two unrelated wire formats). Chosen: standalone crate that depends only on `serde` / `serde_json`. This keeps the codex wire format isolated, lets future consumers (TUI smoke harness, CLI replay tool, etc.) depend on it without inheriting daemon surface.
7. **Schemas committed to git, not regenerated at build time.** Rejected: `cargo build` invokes `codex app-server generate-json-schema` (CI breaks without codex installed; build no longer reproducible). Chosen: `schemas/` is vendored. A future `cargo xtask refresh-codex-schemas` subcommand can wrap the codex CLI invocation, but it stays opt-in and is out of scope this phase.

### 5.2 Crate layout

```
crates/minos-codex-protocol/
├── Cargo.toml
├── src/
│   ├── lib.rs                    (hand-written) public re-exports + crate docs
│   ├── client_request.rs         (hand-written) ClientRequest trait + docs
│   ├── jsonrpc.rs                (hand-written) JsonRpcRequest/Response/Error/Message wire types
│   └── generated/
│       ├── mod.rs                (generated) #![allow(...)] + pub use submodules
│       ├── types.rs              (generated by typify) ~200 struct/enum
│       └── methods.rs            (generated by post-processor) trait impls + 3 union enums
└── tests/
    ├── round_trip.rs             schema fixtures ⇄ typed round-trip
    ├── unions_match_schemas.rs   enum variants exhaustively match schema oneOf lists
    └── fixtures/                 hand-curated JSON examples
        ├── params/
        ├── responses/
        ├── notifications/
        └── server_requests/
```

### 5.3 New crate-level types

```rust
// crates/minos-codex-protocol/src/client_request.rs
/// Marker trait implemented for every `*Params` type that corresponds to a
/// JSON-RPC method codex's app-server accepts. The associated constants and
/// types let `CodexClient::call_typed` build the request frame and decode the
/// response without runtime branching.
///
/// Implementations are auto-generated in `generated/methods.rs` from the
/// `oneOf` list in `schemas/ClientRequest.json`. Do not hand-write impls.
pub trait ClientRequest: serde::Serialize {
    const METHOD: &'static str;
    type Response: serde::de::DeserializeOwned;
}

/// Marker trait for outbound JSON-RPC notifications (no response expected).
/// Mirrors the shape of `ClientRequest` minus the response associated type.
/// Implementations are auto-generated from the `oneOf` list in
/// `schemas/ClientNotification.json` (currently a single variant: `initialized`).
pub trait ClientNotification: serde::Serialize {
    const METHOD: &'static str;
}
```

```rust
// crates/minos-codex-protocol/src/jsonrpc.rs
/// JSON-RPC 2.0 envelope as codex emits it. Codex omits the `"jsonrpc": "2.0"`
/// field on requests/responses (see test fixtures); we accept either shape on
/// inbound and emit without the field on outbound to match remote behavior.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct JsonRpcRequest<P> {
    pub id: serde_json::Value,
    pub method: String,
    pub params: P,
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct JsonRpcResponse<R> {
    pub id: serde_json::Value,
    pub result: R,
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct JsonRpcError {
    pub id: serde_json::Value,
    pub error: JsonRpcErrorPayload,
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct JsonRpcErrorPayload {
    pub code: i64,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
}
```

### 5.4 Generated method index (representative excerpt)

```rust
// crates/minos-codex-protocol/src/generated/methods.rs (excerpt; all generated)

impl crate::ClientRequest for InitializeParams {
    const METHOD: &'static str = "initialize";
    type Response = InitializeResponse;
}
impl crate::ClientRequest for ThreadStartParams {
    const METHOD: &'static str = "thread/start";
    type Response = ThreadStartResponse;
}
impl crate::ClientRequest for TurnStartParams {
    const METHOD: &'static str = "turn/start";
    type Response = TurnStartResponse;
}
impl crate::ClientRequest for TurnInterruptParams {
    const METHOD: &'static str = "turn/interrupt";
    type Response = TurnInterruptResponse;
}
impl crate::ClientRequest for ThreadArchiveParams {
    const METHOD: &'static str = "thread/archive";
    type Response = ThreadArchiveResponse;
}
// ... ~25 more

#[derive(Debug, serde::Deserialize)]
#[serde(tag = "method", content = "params")]
pub enum ServerNotification {
    #[serde(rename = "thread/started")]
    ThreadStarted(ThreadStartedNotification),
    #[serde(rename = "item/started")]
    ItemStarted(ItemStartedNotification),
    #[serde(rename = "item/agentMessage/delta")]
    AgentMessageDelta(AgentMessageDeltaNotification),
    #[serde(rename = "turn/completed")]
    TurnCompleted(TurnCompletedNotification),
    // ... ~50 more
}

#[derive(Debug, serde::Deserialize)]
#[serde(tag = "method", content = "params")]
pub enum ServerRequest {
    #[serde(rename = "item/commandExecution/requestApproval")]
    CommandExecutionRequestApproval(CommandExecutionRequestApprovalParams),
    #[serde(rename = "item/fileChange/requestApproval")]
    FileChangeRequestApproval(FileChangeRequestApprovalParams),
    #[serde(rename = "item/tool/requestUserInput")]
    ToolRequestUserInput(ToolRequestUserInputParams),
    #[serde(rename = "mcpServer/elicitation/request")]
    McpServerElicitationRequest(McpServerElicitationRequestParams),
    #[serde(rename = "item/permissions/requestApproval")]
    PermissionsRequestApproval(PermissionsRequestApprovalParams),
    #[serde(rename = "item/tool/call")]
    DynamicToolCall(DynamicToolCallParams),
    #[serde(rename = "account/chatgptAuthTokens/refresh")]
    ChatgptAuthTokensRefresh(ChatgptAuthTokensRefreshParams),
    #[serde(rename = "applyPatchApproval")]    // DEPRECATED legacy v1 path
    ApplyPatchApproval(ApplyPatchApprovalParams),
    #[serde(rename = "execCommandApproval")]   // DEPRECATED legacy v1 path
    ExecCommandApproval(ExecCommandApprovalParams),
}
```

### 5.5 Consumer-side example

```rust
// crates/minos-agent-runtime/src/runtime.rs (post-refactor excerpt)

use minos_codex_protocol::{
    InitializeParams, InitializeResponse, ClientInfo, InitializeCapabilities,
    InitializedNotification,
    ThreadStartParams, ThreadStartResponse,
    TurnStartParams, UserInput,
};

let init_response: InitializeResponse = client
    .call_typed(InitializeParams {
        client_info: ClientInfo {
            name: env!("CARGO_PKG_NAME").into(),
            title: Some("Minos".into()),
            version: env!("CARGO_PKG_VERSION").into(),
        },
        capabilities: Some(InitializeCapabilities {
            experimental_api: true,
            opt_out_notification_methods: None,
        }),
    })
    .await?;

client.notify_typed(InitializedNotification {}).await?;

let start_response: ThreadStartResponse = client
    .call_typed(ThreadStartParams {
        cwd: Some(cwd.clone()),
        ..Default::default()
    })
    .await?;
let thread_id = start_response.thread.id;
```

### 5.6 codegen pipeline

`cargo xtask gen-codex-protocol` performs:

1. **typify pass.** Load `schemas/codex_app_server_protocol.v2.schemas.json` plus `schemas/codex_app_server_protocol.schemas.json` into a single `typify::TypeSpace` (configured with `with_struct_builder(false)` and `PartialEq`/`Eq` derives). Render to TokenStream, format via `prettyplease`, write to `crates/minos-codex-protocol/src/generated/types.rs` with a fixed banner and `#![allow(clippy::too_many_lines, clippy::struct_field_names, ...)]`.
2. **Union post-process.** Parse the four union schema files (`ClientRequest.json`, `ClientNotification.json`, `ServerRequest.json`, `ServerNotification.json`). For each `oneOf` arm extract `(method_string, params_type_name)`. For `ClientRequest`, the matching response type is resolved by naming convention: `XxxParams` ↔ `XxxResponse`. (No `ClientResponse.json` exists in `schemas/`; the response shape is defined per-request in standalone `*Response.json` files.) The post-processor asserts the convention by checking that the inferred `*Response` type was emitted by typify in step 1; a missing response type aborts the xtask with a clear error. Emit `methods.rs` with:
   - One `impl ClientRequest for XxxParams { ... }` per client request method.
   - One `impl ClientNotification for XxxNotification { ... }` per client notification (currently just `InitializedNotification`).
   - `pub enum ClientRequestMethod { ... }`, `pub enum ServerRequest { ... }`, `pub enum ServerNotification { ... }` with `#[serde(tag = "method", content = "params")]`.
3. **Format + verify.** Run `cargo fmt -p minos-codex-protocol`, then `cargo build -p minos-codex-protocol`. A non-zero exit on either step aborts the xtask with the cargo output included.

### 5.7 Approval handling refactor

The five approval-shaped server requests each have their own response schema with non-overlapping reject vocabulary. The new `auto_reject` picks the correct typed reject per variant.

| Server request | Response type | Reject value |
|---|---|---|
| `applyPatchApproval` (v1, deprecated) | `ApplyPatchApprovalResponse` | `decision: ReviewDecision::Denied` |
| `execCommandApproval` (v1, deprecated) | `ExecCommandApprovalResponse` | `decision: ReviewDecision::Denied` |
| `item/commandExecution/requestApproval` | `CommandExecutionRequestApprovalResponse` | `decision: CommandExecutionApprovalDecision::Decline` |
| `item/fileChange/requestApproval` | `FileChangeRequestApprovalResponse` | `decision: FileChangeApprovalDecision::Decline` |
| `item/permissions/requestApproval` | `PermissionsRequestApprovalResponse` | `permissions: GrantedPermissionProfile::default()` (empty filesystem + network grants) |

`Denied`/`Decline` (rather than `Abort`/`Cancel`) is the chosen reject so the agent continues the turn instead of interrupting it — preserving the original "skip the prompt and let the model recover" intent of the auto-reject.

```rust
// crates/minos-agent-runtime/src/approvals.rs (new contents, abbreviated)

use minos_codex_protocol::{
    ServerRequest,
    ApplyPatchApprovalResponse, ExecCommandApprovalResponse, ReviewDecision,
    CommandExecutionRequestApprovalResponse, CommandExecutionApprovalDecision,
    FileChangeRequestApprovalResponse, FileChangeApprovalDecision,
    PermissionsRequestApprovalResponse, GrantedPermissionProfile,
};

/// Build the typed reply payload to auto-reject an approval `ServerRequest`.
/// Returns `None` for non-approval server requests (the runtime warns and
/// does not reply). Exhaustive over `ServerRequest`; new schema variants
/// trigger a non-exhaustive-match compile error on regeneration.
pub(crate) fn auto_reject(req: &ServerRequest) -> Option<serde_json::Value> {
    let value = match req {
        ServerRequest::ApplyPatchApproval(_) => serde_json::to_value(
            ApplyPatchApprovalResponse { decision: ReviewDecision::Denied },
        ),
        ServerRequest::ExecCommandApproval(_) => serde_json::to_value(
            ExecCommandApprovalResponse { decision: ReviewDecision::Denied },
        ),
        ServerRequest::CommandExecutionRequestApproval(_) => serde_json::to_value(
            CommandExecutionRequestApprovalResponse {
                decision: CommandExecutionApprovalDecision::Decline,
            },
        ),
        ServerRequest::FileChangeRequestApproval(_) => serde_json::to_value(
            FileChangeRequestApprovalResponse {
                decision: FileChangeApprovalDecision::Decline,
            },
        ),
        ServerRequest::PermissionsRequestApproval(_) => serde_json::to_value(
            PermissionsRequestApprovalResponse {
                permissions: GrantedPermissionProfile::default(),
                scope: None,
                strict_auto_review: None,
            },
        ),
        ServerRequest::ToolRequestUserInput(_)
        | ServerRequest::McpServerElicitationRequest(_)
        | ServerRequest::ChatgptAuthTokensRefresh(_)
        | ServerRequest::DynamicToolCall(_) => return None,
    };
    Some(value.expect("typed approval response serialization is infallible"))
}
```

The decision enums (`ReviewDecision`, `CommandExecutionApprovalDecision`, `FileChangeApprovalDecision`) and `GrantedPermissionProfile` are generated by typify from the schemas listed above. `GrantedPermissionProfile` carries optional `fileSystem` and `network` fields that both default to `None`; the empty default is the typed expression of "no permissions granted".

### 5.8 `event_pump_loop` dispatch refactor

```rust
// crates/minos-agent-runtime/src/runtime.rs (event_pump_loop excerpt)

Inbound::ServerRequest { id, method, params } => {
    let envelope = serde_json::json!({ "method": method, "params": params });
    match serde_json::from_value::<ServerRequest>(envelope) {
        Ok(req) => {
            if let Some(reply) = approvals::auto_reject(&req) {
                if let Err(e) = client.reply(id.clone(), reply).await {
                    warn!(error = %e, method = %method,
                          "auto-reject reply failed");
                } else {
                    info!(method = %method, "auto-rejected approval");
                }
            } else {
                warn!(method = %method,
                      "non-approval server request; not replying");
            }
        }
        Err(e) => {
            warn!(method = %method, error = %e,
                  "unknown server request method; not replying");
        }
    }
    // RawIngest keeps the original payload shape unchanged.
    let synthetic_method = format!("server_request/{method}");
    let payload = serde_json::json!({ "method": synthetic_method, "params": params });
    let _ = ingest_tx.send(RawIngest {
        agent, thread_id: thread_id.clone(),
        payload, ts_ms: current_unix_ms(),
    });
}
```

---

## 6. Phased Implementation (sketch)

Detailed phasing belongs in the implementation plan (`writing-plans` step). High-level sequencing:

### Phase A — `minos-codex-protocol` standalone

Create the crate, land the xtask command, generate and commit the types. The runtime is not touched. After this phase the workspace builds and `cargo test -p minos-codex-protocol` passes.

### Phase B — `minos-agent-runtime` typed migration

Add the dependency. Migrate `codex_client.rs` (add `call_typed` + `notify_typed`), `runtime.rs` (5 typed call sites + 1 typed notify site + delete `thread_id_from_response`), `approvals.rs` (full rewrite), `event_pump_loop` (typed dispatch). Migrate `test_support::FakeCodexServer` framing. After this phase the entire runtime e2e suite passes against typed wires on both ends.

### Phase C — Documentation + ADR

Land `docs/adr/0011-codex-protocol-typed-codegen.md`. Add the cross-reference notes in `codex-app-server-integration-design.md` §10.1 and §6.4. Update `crates/minos-codex-protocol/src/lib.rs` doc comment with the regen workflow.

### Phase D — Verification

`cargo test --workspace`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo deny check`, manual smoke against a real `codex app-server` (maintainer's workstation only).

---

## 7. Architectural Notes

- **Semver impact.** `minos-agent-runtime` currently re-exports `pub use approvals::build_auto_reject` from `lib.rs` (line 38). That symbol is removed. Workspace-wide grep at spec authoring time confirmed no other crate imports it (`build_auto_reject` only appears inside `minos-agent-runtime`). No external semver concern (workspace-internal crate).
- **Object safety.** `ClientRequest` and `ClientNotification` are not object-safe (associated constants on the former, `Self: Serialize` bound on both). Intentional — `call_typed` and `notify_typed` are generic; no `dyn` use is needed.
- **No new runtime allocations or task spawns.** `call_typed` adds one `serde_json::to_value` + one `serde_json::from_value` per call site, both already on the hot path (the existing `call(method, params)` does the same).
- **What is explicitly NOT changed:**
  - `RawIngest` payload type (`serde_json::Value`) — backend translator stability.
  - `CodexClient` transport machinery (stdio pump, mpsc channels, `Inbound`/`Outbound` enums) — orthogonal concern.
  - Approval policy (always reject) — separate spec.
  - `minos-protocol` — orthogonal RPC surface.
  - `minos-ui-protocol` — UI-side message format.
- **New cross-crate dependencies:** `minos-agent-runtime` → `minos-codex-protocol` (path dep). `minos-codex-protocol` → `serde`, `serde_json` only.
- **New dev-dependencies (xtask only):** `typify` (~0.4), `prettyplease`, `syn`, `quote`. None of these are exposed to runtime crates or shipped binaries.
- **Schema evolution policy.** typify defaults emit `#[serde(deny_unknown_fields)]` off — codex adding new optional fields will not break us. Adding a new enum variant or method requires a regen pass (and the union completeness test fails, prompting the regen).
- **`additionalProperties: true` handling.** Schemas like `McpToolCallResult._meta` and `DynamicToolCallSpec.inputSchema` are typed as `serde_json::Value` by typify. Acceptable — those fields are passthrough to MCP servers / dynamic tools, the runtime does not interpret them.

---

## 8. File Change Summary

- `Cargo.toml` -- (workspace) no edit; `crates/*` glob picks up the new crate automatically.
- `crates/minos-agent-runtime/Cargo.toml` -- add `minos-codex-protocol = { path = "../minos-codex-protocol", version = "0.1.0" }` to `[dependencies]`.
- `crates/minos-agent-runtime/src/approvals.rs` -- full rewrite; remove `APPROVAL_METHODS`, replace `build_auto_reject` with typed `auto_reject(&ServerRequest)`.
- `crates/minos-agent-runtime/src/codex_client.rs` -- add `call_typed<R: ClientRequest>` and `notify_typed<N: ClientNotification>` methods; existing `call`/`notify`/`reply` unchanged.
- `crates/minos-agent-runtime/src/lib.rs` -- remove `pub use approvals::build_auto_reject`; the symbol has no downstream consumers (verified: no other workspace crate imports it). Clarify the dependency-rule comment to reflect the new `minos-codex-protocol` dependency.
- `crates/minos-agent-runtime/src/runtime.rs` -- migrate 5 hand-rolled `client.call(...)` sites to `client.call_typed(...)` and 1 `client.notify("initialized", ...)` site to `client.notify_typed(InitializedNotification {})`; delete `thread_id_from_response` and the `initialize_params` / `thread_id_request_params` helpers; rewrite `event_pump_loop`'s server-request branch to use typed dispatch.
- `crates/minos-agent-runtime/src/test_support.rs` -- migrate `FakeCodexServer`'s frame construction to typed structs from `minos-codex-protocol`.
- `crates/minos-agent-runtime/tests/runtime_e2e.rs` -- adjust any direct construction of fake-server frames; behavior unchanged.
- `crates/minos-codex-protocol/Cargo.toml` -- new file; depends on `serde`, `serde_json`.
- `crates/minos-codex-protocol/src/lib.rs` -- new file; public re-exports + crate docs including regen workflow.
- `crates/minos-codex-protocol/src/client_request.rs` -- new file; `ClientRequest` trait definition + docs.
- `crates/minos-codex-protocol/src/jsonrpc.rs` -- new file; JSON-RPC envelope wire types.
- `crates/minos-codex-protocol/src/generated/mod.rs` -- generated; submodule re-exports + lint allows.
- `crates/minos-codex-protocol/src/generated/types.rs` -- generated; ~200 typify-rendered struct/enum.
- `crates/minos-codex-protocol/src/generated/methods.rs` -- generated; trait impls + 3 union enums.
- `crates/minos-codex-protocol/tests/round_trip.rs` -- new file; fixture-driven round-trip tests.
- `crates/minos-codex-protocol/tests/unions_match_schemas.rs` -- new file; enum completeness vs schema oneOf.
- `crates/minos-codex-protocol/tests/fixtures/**` -- new directory; hand-curated schema instance JSON files.
- `xtask/Cargo.toml` -- add `typify`, `prettyplease`, `syn`, `quote` dev-deps (or regular deps scoped to the gen subcommand).
- `xtask/src/main.rs` -- register the `gen-codex-protocol` subcommand.
- `xtask/src/gen_codex.rs` -- new file; typify driver + union post-processor.
- `docs/adr/0011-codex-protocol-typed-codegen.md` -- new ADR.
- `docs/superpowers/specs/codex-app-server-integration-design.md` -- add cross-reference notes in §10.1 and §6.4 pointing at this spec.
- `schemas/` -- already present (vendored); no edits as part of this spec.
