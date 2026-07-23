//! Discover models available for a local CLI runtime.
//!
//! Preference order per runtime (best-effort, timeout-bounded):
//! 1. Native probe (Codex `model/list`, Grok/OpenCode CLI)
//! 2. Static alias / curated lists when probes fail

use std::process::Stdio;
use std::time::Duration;

use minos_domain::AgentName;
use minos_protocol::{ListModelsResponse, ModelInfo};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;
use tokio::time::timeout;

const PROBE_TIMEOUT: Duration = Duration::from_secs(8);

pub async fn list_models_for_runtime(runtime: AgentName) -> ListModelsResponse {
    match runtime {
        AgentName::Codex => probe_codex().await.unwrap_or_else(static_codex),
        AgentName::Grok => probe_cli_models("grok", &["models"], "cli")
            .await
            .unwrap_or_else(static_grok),
        AgentName::Opencode => probe_cli_models("opencode", &["models"], "cli")
            .await
            .unwrap_or_else(static_opencode),
        AgentName::Claude => static_claude(),
        AgentName::Gemini => static_gemini(),
    }
}

fn static_codex() -> ListModelsResponse {
    ListModelsResponse {
        runtime: AgentName::Codex,
        source: "static".into(),
        models: vec![
            model(
                "gpt-5.4",
                "GPT-5.4",
                true,
                &["low", "medium", "high", "xhigh"],
            ),
            model(
                "gpt-5.4-mini",
                "GPT-5.4 Mini",
                false,
                &["low", "medium", "high"],
            ),
            model("gpt-5.2", "GPT-5.2", false, &["low", "medium", "high"]),
        ],
    }
}

fn static_claude() -> ListModelsResponse {
    ListModelsResponse {
        runtime: AgentName::Claude,
        source: "static".into(),
        models: vec![
            model("sonnet", "Sonnet (alias)", true, &[]),
            model("opus", "Opus (alias)", false, &[]),
            model("fable", "Fable (alias)", false, &[]),
            model("haiku", "Haiku (alias)", false, &[]),
        ],
    }
}

fn static_gemini() -> ListModelsResponse {
    ListModelsResponse {
        runtime: AgentName::Gemini,
        source: "static".into(),
        models: vec![
            model("gemini-2.5-pro", "Gemini 2.5 Pro", true, &[]),
            model("gemini-2.5-flash", "Gemini 2.5 Flash", false, &[]),
            model("gemini-3-pro-preview", "Gemini 3 Pro Preview", false, &[]),
            model(
                "gemini-3-flash-preview",
                "Gemini 3 Flash Preview",
                false,
                &[],
            ),
        ],
    }
}

fn static_grok() -> ListModelsResponse {
    ListModelsResponse {
        runtime: AgentName::Grok,
        source: "static".into(),
        models: vec![model(
            "grok-4.5",
            "Grok 4.5",
            true,
            &["low", "medium", "high"],
        )],
    }
}

fn static_opencode() -> ListModelsResponse {
    ListModelsResponse {
        runtime: AgentName::Opencode,
        source: "static".into(),
        models: vec![
            model("openai/gpt-5.2", "OpenAI/GPT-5.2", true, &[]),
            model("openai/gpt-5.2-codex", "OpenAI/GPT-5.2 Codex", false, &[]),
        ],
    }
}

fn model(id: &str, name: &str, is_default: bool, efforts: &[&str]) -> ModelInfo {
    ModelInfo {
        id: id.into(),
        display_name: name.into(),
        description: None,
        is_default,
        supported_reasoning_efforts: efforts.iter().map(|s| (*s).to_string()).collect(),
        default_reasoning_effort: efforts.first().map(|s| (*s).to_string()),
    }
}

async fn probe_cli_models(bin: &str, args: &[&str], source: &str) -> Option<ListModelsResponse> {
    let runtime = match bin {
        "grok" => AgentName::Grok,
        "opencode" => AgentName::Opencode,
        _ => return None,
    };
    let mut child = Command::new(bin)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .ok()?;
    let stdout = child.stdout.take()?;
    let read = async {
        let mut lines = BufReader::new(stdout).lines();
        let mut models = Vec::new();
        while let Ok(Some(line)) = lines.next_line().await {
            let line = line.trim();
            if line.is_empty() || line.starts_with("You are") || line.starts_with("Default") {
                continue;
            }
            // `opencode models` → provider/model
            // `grok models` → `* id (default)` or plain id
            let cleaned = line
                .trim_start_matches(|c: char| c == '*' || c.is_whitespace())
                .split_whitespace()
                .next()
                .unwrap_or("")
                .trim_end_matches([',', ')']);
            if cleaned.is_empty() || cleaned == "Available" || cleaned == "models:" {
                continue;
            }
            let is_default = line.contains("(default)");
            // Only attach a static effort ladder when domain SSOT says the
            // runtime supports reasoning effort (CLI list text has no efforts).
            let (efforts, default_effort) = if runtime.supports_reasoning_effort() {
                (
                    vec!["low".into(), "medium".into(), "high".into()],
                    Some("high".into()),
                )
            } else {
                (vec![], None)
            };
            models.push(ModelInfo {
                id: cleaned.to_string(),
                display_name: cleaned.to_string(),
                description: None,
                is_default,
                supported_reasoning_efforts: efforts,
                default_reasoning_effort: default_effort,
            });
        }
        let _ = child.wait().await;
        models
    };
    let models = timeout(PROBE_TIMEOUT, read).await.ok()?;
    if models.is_empty() {
        return None;
    }
    // Ensure one default
    let mut models = models;
    if !models.iter().any(|m| m.is_default) {
        if let Some(first) = models.first_mut() {
            first.is_default = true;
        }
    }
    Some(ListModelsResponse {
        runtime,
        models,
        source: source.into(),
    })
}

async fn probe_codex() -> Option<ListModelsResponse> {
    let mut child = Command::new("codex")
        .args(["app-server", "--stdio"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .ok()?;
    let mut stdin = child.stdin.take()?;
    let stdout = child.stdout.take()?;
    let work = async {
        let init = serde_json::json!({
            "method": "initialize",
            "id": 1,
            "params": {
                "clientInfo": { "name": "minos", "version": "0" },
                "capabilities": {}
            }
        });
        stdin.write_all(format!("{init}\n").as_bytes()).await.ok()?;
        stdin
            .write_all(b"{\"method\":\"initialized\",\"params\":{}}\n")
            .await
            .ok()?;
        stdin
            .write_all(b"{\"method\":\"model/list\",\"id\":2,\"params\":{}}\n")
            .await
            .ok()?;
        stdin.flush().await.ok()?;

        let mut lines = BufReader::new(stdout).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            let Ok(v) = serde_json::from_str::<serde_json::Value>(&line) else {
                continue;
            };
            if v.get("id") != Some(&serde_json::json!(2)) {
                continue;
            }
            let data = v
                .pointer("/result/data")
                .and_then(|d| d.as_array())
                .cloned()
                .unwrap_or_default();
            let mut models = Vec::new();
            for m in data {
                let id = m
                    .get("id")
                    .or_else(|| m.get("model"))
                    .and_then(|x| x.as_str())
                    .unwrap_or("")
                    .to_string();
                if id.is_empty() {
                    continue;
                }
                let display = m
                    .get("displayName")
                    .and_then(|x| x.as_str())
                    .unwrap_or(&id)
                    .to_string();
                let is_default = m
                    .get("isDefault")
                    .and_then(|x| x.as_bool())
                    .unwrap_or(false);
                let efforts: Vec<String> = m
                    .get("supportedReasoningEfforts")
                    .and_then(|x| x.as_array())
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|e| {
                                e.get("reasoningEffort")
                                    .and_then(|x| x.as_str())
                                    .map(str::to_string)
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                let default_effort = m
                    .get("defaultReasoningEffort")
                    .and_then(|x| x.as_str())
                    .map(str::to_string);
                models.push(ModelInfo {
                    id,
                    display_name: display,
                    description: m
                        .get("description")
                        .and_then(|x| x.as_str())
                        .map(str::to_string),
                    is_default,
                    supported_reasoning_efforts: efforts,
                    default_reasoning_effort: default_effort,
                });
            }
            let _ = child.kill().await;
            if models.is_empty() {
                return None;
            }
            return Some(ListModelsResponse {
                runtime: AgentName::Codex,
                models,
                source: "app_server".into(),
            });
        }
        let _ = child.kill().await;
        None
    };
    timeout(PROBE_TIMEOUT, work).await.ok().flatten()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn static_lists_non_empty() {
        assert!(!static_claude().models.is_empty());
        assert!(static_claude().models.iter().any(|m| m.is_default));
        assert!(!static_gemini().models.is_empty());
    }

    #[test]
    fn unsupported_runtimes_do_not_advertise_efforts() {
        for m in static_claude().models {
            assert!(
                m.supported_reasoning_efforts.is_empty(),
                "claude model {} invented efforts",
                m.id
            );
            assert!(m.default_reasoning_effort.is_none());
        }
        for m in static_gemini().models {
            assert!(
                m.supported_reasoning_efforts.is_empty(),
                "gemini model {} invented efforts",
                m.id
            );
        }
        for m in static_opencode().models {
            assert!(
                m.supported_reasoning_efforts.is_empty(),
                "opencode model {} invented efforts",
                m.id
            );
        }
        assert!(!AgentName::Claude.supports_reasoning_effort());
        assert!(!AgentName::Gemini.supports_reasoning_effort());
        assert!(!AgentName::Opencode.supports_reasoning_effort());
    }

    #[test]
    fn effort_capable_runtimes_list_honest_efforts() {
        assert!(AgentName::Codex.supports_reasoning_effort());
        assert!(AgentName::Grok.supports_reasoning_effort());
        assert!(static_codex()
            .models
            .iter()
            .any(|m| !m.supported_reasoning_efforts.is_empty()));
        assert!(static_grok()
            .models
            .iter()
            .any(|m| !m.supported_reasoning_efforts.is_empty()));
    }
}
