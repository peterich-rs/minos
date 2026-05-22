use std::fs;
use std::path::Path;

use anyhow::{bail, Context, Result};
use serde_json::{json, Map, Value};

pub const BACKEND_PLATFORM_CONTRACT_PATH: &str = "schemas/minos_backend_platform_contract.json";
pub const BACKEND_OPENAPI_PATH: &str = "schemas/minos_backend_openapi.json";
pub const BACKEND_WS_SCHEMA_PATH: &str = "schemas/minos_backend_ws_schema.json";

struct GeneratedArtifact {
    path: &'static str,
    rendered: String,
}

pub fn generate(workspace_root: &Path, check: bool) -> Result<()> {
    for artifact in rendered_artifacts(workspace_root)? {
        write_or_check_artifact(workspace_root, &artifact, check)?;
    }
    Ok(())
}

fn rendered_artifacts(workspace_root: &Path) -> Result<Vec<GeneratedArtifact>> {
    Ok(vec![
        GeneratedArtifact {
            path: BACKEND_PLATFORM_CONTRACT_PATH,
            rendered: render_backend_platform_contract()?,
        },
        GeneratedArtifact {
            path: BACKEND_OPENAPI_PATH,
            rendered: render_backend_openapi()?,
        },
        GeneratedArtifact {
            path: BACKEND_WS_SCHEMA_PATH,
            rendered: render_backend_ws_schema(workspace_root)?,
        },
    ])
}

fn write_or_check_artifact(
    workspace_root: &Path,
    artifact: &GeneratedArtifact,
    check: bool,
) -> Result<()> {
    let output_path = workspace_root.join(artifact.path);
    if check {
        let existing = fs::read_to_string(&output_path)
            .with_context(|| format!("read {} for backend platform schema drift check", output_path.display()))?;
        if existing != artifact.rendered {
            bail!(
                "backend platform schema drift detected at {}. Run `cargo xtask gen-backend-platform-contract` and commit the updated artifacts.",
                output_path.display()
            );
        }
        return Ok(());
    }

    if let Some(parent) = output_path.parent() {
        fs::create_dir_all(parent).with_context(|| format!("mkdir -p {}", parent.display()))?;
    }
    fs::write(&output_path, &artifact.rendered)
        .with_context(|| format!("write {}", output_path.display()))?;
    eprintln!("wrote {}", output_path.display());
    Ok(())
}

fn render_backend_platform_contract() -> Result<String> {
    let contract = minos_backend::runtime::platform_contract_snapshot();
    render_pretty_json(&contract).context("serialize backend platform contract")
}

fn render_backend_openapi() -> Result<String> {
    let platform = minos_backend::runtime::platform_contract_snapshot();
    let mut paths = Map::new();

    for route in minos_backend::http::formal_route_inventory() {
        let openapi_path = openapi_path(route.path);
        let path_item = paths
            .entry(openapi_path.clone())
            .or_insert_with(|| Value::Object(Map::new()));
        let path_item = path_item
            .as_object_mut()
            .expect("path entries are always objects");

        let parameters = path_parameters(route.path);
        if !parameters.is_empty() {
            path_item
                .entry("parameters".to_string())
                .or_insert_with(|| Value::Array(parameters));
        }

        path_item.insert(
            route.method.to_ascii_lowercase(),
            render_openapi_operation(route),
        );
    }

    let doc = json!({
        "openapi": "3.1.0",
        "info": {
            "title": "Minos Backend HTTP API",
            "version": platform.version,
            "description": "Generated from minos-backend's formal route inventory. Operation bodies stay intentionally schematic in this pass; the document captures mounted paths, methods, auth surfaces, and websocket upgrade points."
        },
        "paths": Value::Object(paths),
        "components": {
            "securitySchemes": {
                "AccountBearer": {
                    "type": "http",
                    "scheme": "bearer",
                    "bearerFormat": "JWT",
                    "description": "Bearer token used by account_api endpoints."
                }
            }
        },
        "x-minos-generated-from": "minos_backend::http::formal_route_inventory",
        "x-minos-websocket-schema": BACKEND_WS_SCHEMA_PATH,
    });

    render_pretty_json(&doc)
}

fn render_openapi_operation(route: &minos_backend::http::RouteContract) -> Value {
    let mut operation = Map::new();
    operation.insert(
        "operationId".to_string(),
        Value::String(operation_id(route)),
    );
    operation.insert(
        "tags".to_string(),
        Value::Array(vec![Value::String(route.surface.to_string())]),
    );
    operation.insert(
        "summary".to_string(),
        Value::String(format!("{} {}", route.method, route.path)),
    );
    operation.insert(
        "description".to_string(),
        Value::String(operation_description(route)),
    );
    operation.insert("responses".to_string(), responses_for_route(route));
    operation.insert(
        "x-minos-surface".to_string(),
        Value::String(route.surface.to_string()),
    );
    operation.insert(
        "x-minos-auth".to_string(),
        Value::String(route.auth.to_string()),
    );
    operation.insert(
        "x-minos-probe-path".to_string(),
        Value::String(route.probe_path.to_string()),
    );

    if route.auth == "account_bearer" {
        operation.insert("security".to_string(), json!([{ "AccountBearer": [] }]));
    }
    if route.method == "POST" && !is_websocket_route(route) {
        operation.insert(
            "requestBody".to_string(),
            json!({
                "required": true,
                "content": {
                    "application/json": {
                        "schema": true,
                    }
                }
            }),
        );
    }
    if is_websocket_route(route) {
        operation.insert("x-minos-websocket".to_string(), Value::Bool(true));
        operation.insert(
            "x-minos-websocket-schema-ref".to_string(),
            Value::String("#/schemas/Envelope".to_string()),
        );
    }

    Value::Object(operation)
}

fn responses_for_route(route: &minos_backend::http::RouteContract) -> Value {
    let mut responses = Map::new();
    if is_websocket_route(route) {
        responses.insert(
            "101".to_string(),
            json!({ "description": "WebSocket upgrade accepted" }),
        );
        responses.insert(
            "401".to_string(),
            json!({ "description": "Upgrade rejected before websocket activation" }),
        );
        return Value::Object(responses);
    }

    let success_content = if route.path == "/metrics" {
        json!({
            "text/plain": {
                "schema": { "type": "string" }
            }
        })
    } else {
        json!({
            "application/json": {
                "schema": true,
            }
        })
    };
    responses.insert(
        "200".to_string(),
        json!({
            "description": "Success",
            "content": success_content,
        }),
    );

    if route.auth != "public" {
        responses.insert(
            "401".to_string(),
            json!({ "description": "Authentication required or rejected" }),
        );
    }

    Value::Object(responses)
}

fn operation_id(route: &minos_backend::http::RouteContract) -> String {
    let mut parts = vec![route.method.to_ascii_lowercase()];
    parts.extend(
        route
            .path
            .trim_start_matches('/')
            .split('/')
            .filter(|segment| !segment.is_empty())
            .map(|segment| segment.trim_start_matches(':').replace('-', "_")),
    );
    parts.join("_")
}

fn operation_description(route: &minos_backend::http::RouteContract) -> String {
    if is_websocket_route(route) {
        format!(
            "WebSocket upgrade endpoint for the `{}` surface. The frame contract is emitted separately in {}.",
            route.surface, BACKEND_WS_SCHEMA_PATH
        )
    } else {
        format!(
            "Generated from the mounted route inventory for the `{}` surface with `{}` auth semantics.",
            route.surface, route.auth
        )
    }
}

fn openapi_path(path: &str) -> String {
    path.split('/')
        .map(|segment| {
            segment
                .strip_prefix(':')
                .map_or_else(|| segment.to_string(), |name| format!("{{{name}}}"))
        })
        .collect::<Vec<_>>()
        .join("/")
}

fn path_parameters(path: &str) -> Vec<Value> {
    path.split('/')
        .filter_map(|segment| segment.strip_prefix(':'))
        .map(|name| {
            json!({
                "name": name,
                "in": "path",
                "required": true,
                "schema": { "type": "string" }
            })
        })
        .collect()
}

fn is_websocket_route(route: &minos_backend::http::RouteContract) -> bool {
    route.path.starts_with("/ws/")
}

fn render_backend_ws_schema(workspace_root: &Path) -> Result<String> {
    let platform = minos_backend::runtime::platform_contract_snapshot();
    let examples = load_envelope_examples(workspace_root)?;
    let example_names = examples.keys().cloned().map(Value::String).collect::<Vec<_>>();
    let gateways = minos_backend::http::formal_route_inventory()
        .iter()
        .filter(|route| is_websocket_route(route))
        .map(|route| {
            json!({
                "surface": route.surface,
                "path": route.path,
                "auth": route.auth,
                "probe_path": route.probe_path,
                "frame_schema_ref": "#/schemas/Envelope",
                "example_names": example_names,
            })
        })
        .collect::<Vec<_>>();

    let doc = json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "service": platform.service,
        "version": platform.version,
        "transport": "websocket",
        "description": "Generated websocket contract for the Minos backend relay envelope.",
        "gateways": gateways,
        "schemas": {
            "Envelope": envelope_schema(),
            "EventKind": event_kind_schema(),
        },
        "examples": examples,
    });

    render_pretty_json(&doc)
}

fn load_envelope_examples(workspace_root: &Path) -> Result<Map<String, Value>> {
    let examples_dir = workspace_root.join("crates/minos-protocol/tests/golden/envelope");
    let mut entries = fs::read_dir(&examples_dir)
        .with_context(|| format!("read_dir {}", examples_dir.display()))?
        .collect::<std::result::Result<Vec<_>, _>>()
        .with_context(|| format!("enumerate {}", examples_dir.display()))?;
    entries.sort_by_key(|entry| entry.file_name());

    let mut examples = Map::new();
    for entry in entries {
        let path = entry.path();
        if path.extension().and_then(|value| value.to_str()) != Some("json") {
            continue;
        }
        let stem = path
            .file_stem()
            .and_then(|value| value.to_str())
            .context("envelope example missing file stem")?;
        let raw = fs::read_to_string(&path)
            .with_context(|| format!("read envelope example {}", path.display()))?;
        let parsed = serde_json::from_str::<Value>(&raw)
            .with_context(|| format!("parse envelope example {}", path.display()))?;
        examples.insert(stem.to_string(), parsed);
    }
    Ok(examples)
}

fn envelope_schema() -> Value {
    json!({
        "oneOf": [
            {
                "title": "ForwardEnvelope",
                "type": "object",
                "required": ["kind", "v", "target_device_id", "payload"],
                "properties": {
                    "kind": { "const": "forward" },
                    "v": { "const": 1 },
                    "target_device_id": uuid_schema(),
                    "payload": true,
                }
            },
            {
                "title": "ForwardedEnvelope",
                "type": "object",
                "required": ["kind", "v", "from", "payload"],
                "properties": {
                    "kind": { "const": "forwarded" },
                    "v": { "const": 1 },
                    "from": uuid_schema(),
                    "payload": true,
                }
            },
            {
                "title": "EventEnvelope",
                "allOf": [
                    {
                        "type": "object",
                        "required": ["kind", "v"],
                        "properties": {
                            "kind": { "const": "event" },
                            "v": { "const": 1 }
                        }
                    },
                    { "$ref": "#/schemas/EventKind" }
                ]
            },
            {
                "title": "IngestEnvelope",
                "type": "object",
                "required": ["kind", "v", "agent", "thread_id", "seq", "payload", "ts_ms"],
                "properties": {
                    "kind": { "const": "ingest" },
                    "v": { "const": 1 },
                    "agent": { "type": "string" },
                    "thread_id": { "type": "string" },
                    "seq": integer_schema(),
                    "payload": true,
                    "ts_ms": { "type": "integer" }
                }
            }
        ]
    })
}

fn event_kind_schema() -> Value {
    json!({
        "oneOf": [
            {
                "title": "PairedEvent",
                "type": "object",
                "required": ["type", "peer_device_id", "peer_name"],
                "properties": {
                    "type": { "const": "paired" },
                    "peer_device_id": uuid_schema(),
                    "peer_name": { "type": "string" },
                    "your_device_secret": { "type": "string" }
                }
            },
            {
                "title": "PeerOnlineEvent",
                "type": "object",
                "required": ["type", "peer_device_id"],
                "properties": {
                    "type": { "const": "peer_online" },
                    "peer_device_id": uuid_schema()
                }
            },
            {
                "title": "PeerOfflineEvent",
                "type": "object",
                "required": ["type", "peer_device_id"],
                "properties": {
                    "type": { "const": "peer_offline" },
                    "peer_device_id": uuid_schema()
                }
            },
            {
                "title": "UnpairedEvent",
                "type": "object",
                "required": ["type"],
                "properties": {
                    "type": { "const": "unpaired" }
                }
            },
            {
                "title": "ServerShutdownEvent",
                "type": "object",
                "required": ["type"],
                "properties": {
                    "type": { "const": "server_shutdown" }
                }
            },
            {
                "title": "UiEventMessageEvent",
                "type": "object",
                "required": ["type", "thread_id", "seq", "ui", "ts_ms"],
                "properties": {
                    "type": { "const": "ui_event_message" },
                    "thread_id": { "type": "string" },
                    "seq": integer_schema(),
                    "ui": true,
                    "ts_ms": { "type": "integer" }
                }
            },
            {
                "title": "ApprovalRequestEvent",
                "type": "object",
                "required": ["type", "thread_id", "turn_id", "request_id", "method", "params", "timeout_ms"],
                "properties": {
                    "type": { "const": "approval_request" },
                    "thread_id": { "type": "string" },
                    "turn_id": { "type": "string" },
                    "request_id": { "type": "string" },
                    "method": { "type": "string" },
                    "params": true,
                    "timeout_ms": integer_schema()
                }
            },
            {
                "title": "ApprovalTimeoutEvent",
                "type": "object",
                "required": ["type", "thread_id", "request_id", "reason"],
                "properties": {
                    "type": { "const": "approval_timeout" },
                    "thread_id": { "type": "string" },
                    "request_id": { "type": "string" },
                    "reason": { "type": "string" }
                }
            },
            {
                "title": "AgentErrorEvent",
                "type": "object",
                "required": ["type", "code", "message"],
                "properties": {
                    "type": { "const": "agent_error" },
                    "session_id": { "type": "string" },
                    "code": { "type": "string" },
                    "message": { "type": "string" }
                }
            },
            {
                "title": "SocialMessageEvent",
                "type": "object",
                "required": ["type", "conversation_id", "message"],
                "properties": {
                    "type": { "const": "social_message" },
                    "conversation_id": { "type": "string" },
                    "message": true
                }
            },
            {
                "title": "IngestCheckpointEvent",
                "type": "object",
                "required": ["type", "last_seq_per_thread"],
                "properties": {
                    "type": { "const": "ingest_checkpoint" },
                    "last_seq_per_thread": {
                        "type": "object",
                        "additionalProperties": integer_schema()
                    }
                }
            }
        ]
    })
}

fn uuid_schema() -> Value {
    json!({
        "type": "string",
        "format": "uuid"
    })
}

fn integer_schema() -> Value {
    json!({
        "type": "integer",
        "minimum": 0
    })
}

fn render_pretty_json(value: &impl serde::Serialize) -> Result<String> {
    let mut rendered = serde_json::to_string_pretty(value)?;
    rendered.push('\n');
    Ok(rendered)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn count_openapi_operations(doc: &Value) -> usize {
        doc["paths"]
            .as_object()
            .expect("paths object")
            .values()
            .map(|path_item| {
                path_item
                    .as_object()
                    .expect("path item")
                    .iter()
                    .filter(|(key, _)| matches!(key.as_str(), "get" | "post" | "delete"))
                    .count()
            })
            .sum()
    }

    #[test]
    fn render_backend_platform_contract_includes_external_sql_metadata() {
        let rendered = render_backend_platform_contract().unwrap();
        let parsed: Value = serde_json::from_str(&rendered).unwrap();
        let drivers = parsed["external_sql"]["supported_drivers"]
            .as_array()
            .expect("supported_drivers array");
        assert!(drivers.iter().any(|value| value == "postgres"));
    }

    #[test]
    fn render_backend_openapi_covers_every_formal_route() {
        let rendered = render_backend_openapi().unwrap();
        let parsed: Value = serde_json::from_str(&rendered).unwrap();
        assert_eq!(parsed["openapi"], "3.1.0");
        assert_eq!(
            count_openapi_operations(&parsed),
            minos_backend::http::formal_route_inventory().len()
        );
    }

    #[test]
    fn render_backend_ws_schema_embeds_gateways_and_examples() {
        let workspace_root = crate::workspace_root().unwrap();
        let rendered = render_backend_ws_schema(&workspace_root).unwrap();
        let parsed: Value = serde_json::from_str(&rendered).unwrap();
        assert_eq!(parsed["transport"], "websocket");
        assert_eq!(parsed["gateways"].as_array().unwrap().len(), 2);
        assert!(parsed["examples"].get("event_paired").is_some());
        assert!(parsed["schemas"]["Envelope"]["oneOf"].as_array().unwrap().len() >= 4);
    }
}