#![allow(unsafe_code)]

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use base64::Engine;
use eventsource_stream::Eventsource;
use futures_util::StreamExt;
use minos_domain::AgentName;
use reqwest::Client;
use serde_json::Value;
use tokio::process::{Child, Command};
use tokio::sync::broadcast;
use tokio::task::JoinHandle;
use tracing::{info, warn};

use crate::config::RawIngest;

const KILL_ESCALATION: Duration = Duration::from_secs(3);

pub struct OpencodeServerConfig {
    pub opencode_bin: PathBuf,
    pub port: u16,
    pub password: String,
    pub subprocess_env: Arc<HashMap<String, String>>,
}

pub struct OpencodeServerInstance {
    pub workspace: PathBuf,
    pub config: OpencodeServerConfig,
    pub child: Option<Child>,
    pub http_client: Client,
    pub base_url: String,
    pub auth_header: String,
}

impl OpencodeServerInstance {
    pub async fn spawn(workspace: &Path, config: OpencodeServerConfig) -> anyhow::Result<Self> {
        let port = config.port;
        let mut cmd = Command::new(&config.opencode_bin);
        cmd.args(["serve", "--port", &port.to_string()])
            .current_dir(workspace)
            .env_clear()
            .envs(config.subprocess_env.iter())
            .env("OPENCODE_SERVER_PASSWORD", &config.password)
            .stdin(Stdio::null())
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

        let mut child = cmd.spawn()?;

        let stderr = child.stderr.take();
        if let Some(err_stream) = stderr {
            tokio::spawn(async move {
                use tokio::io::AsyncBufReadExt;
                let reader = tokio::io::BufReader::new(err_stream);
                let mut lines = reader.lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    warn!(
                        target: "minos_agent_runtime::opencode_driver",
                        line = %line,
                        "opencode stderr"
                    );
                }
            });
        }

        let base_url = format!("http://127.0.0.1:{port}");
        let auth_header = format!(
            "Basic {}",
            base64::engine::general_purpose::STANDARD.encode(format!("opencode:{}", config.password))
        );

        let http_client = Client::builder()
            .timeout(Duration::from_secs(30))
            .build()?;

        let instance = Self {
            workspace: workspace.to_path_buf(),
            config,
            child: Some(child),
            http_client,
            base_url: base_url.clone(),
            auth_header: auth_header.clone(),
        };

        let health_url = format!("{base_url}/global/health");
        for _ in 0..30 {
            if instance
                .http_client
                .get(&health_url)
                .header("Authorization", &auth_header)
                .send()
                .await
                .is_ok_and(|r| r.status().is_success())
            {
                info!(
                    target: "minos_agent_runtime::opencode_driver",
                    port,
                    workspace = %workspace.display(),
                    "opencode server ready"
                );
                return Ok(instance);
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }

        Err(anyhow::anyhow!("opencode server did not become healthy within timeout"))
    }

    pub async fn create_session(&mut self) -> anyhow::Result<String> {
        let resp = self
            .http_client
            .post(format!("{}/session", self.base_url))
            .header("Authorization", &self.auth_header)
            .header("Content-Type", "application/json")
            .body("{}")
            .send()
            .await?;
        let body: Value = resp.json().await?;
        body.get("id")
            .and_then(Value::as_str)
            .map(str::to_string)
            .ok_or_else(|| anyhow::anyhow!("opencode create_session: missing id in response"))
    }

    pub async fn send_prompt(&self, session_id: &str, text: &str) -> anyhow::Result<()> {
        let payload = serde_json::json!({
            "parts": [{ "type": "text", "text": text }]
        });
        self.http_client
            .post(format!("{}/session/{session_id}/prompt_async", self.base_url))
            .header("Authorization", &self.auth_header)
            .header("Content-Type", "application/json")
            .body(serde_json::to_string(&payload)?)
            .send()
            .await?;
        Ok(())
    }

    pub async fn abort_session(&self, session_id: &str) -> anyhow::Result<()> {
        self.http_client
            .post(format!("{}/session/{session_id}/abort", self.base_url))
            .header("Authorization", &self.auth_header)
            .send()
            .await?;
        Ok(())
    }

    pub async fn respond_permission(
        &self,
        session_id: &str,
        permission_id: &str,
        response: &str,
    ) -> anyhow::Result<()> {
        let payload = serde_json::json!({ "response": response });
        self.http_client
            .post(format!(
                "{}/session/{session_id}/permissions/{permission_id}",
                self.base_url
            ))
            .header("Authorization", &self.auth_header)
            .header("Content-Type", "application/json")
            .body(serde_json::to_string(&payload)?)
            .send()
            .await?;
        Ok(())
    }

    pub fn subscribe_sse_url(&self) -> String {
        format!("{}/event", self.base_url)
    }

    pub fn auth_header(&self) -> &str {
        &self.auth_header
    }

    pub fn workspace(&self) -> &Path {
        &self.workspace
    }

    pub async fn close(mut self) {
        if let Some(mut child) = self.child.take() {
            #[cfg(unix)]
            {
                let pid = child.id();
                if let Some(pid) = pid {
                    #[allow(clippy::cast_possible_wrap)]
                    unsafe {
                        libc::kill(pid as i32, libc::SIGTERM);
                    }
                }
            }

            let exited = tokio::time::timeout(KILL_ESCALATION, child.wait()).await;
            if exited.is_err() {
                warn!(
                    target: "minos_agent_runtime::opencode_driver",
                    "opencode did not exit after SIGTERM, sending SIGKILL"
                );
                let _ = child.kill().await;
            }
        }

        info!(
            target: "minos_agent_runtime::opencode_driver",
            workspace = %self.workspace.display(),
            "opencode server instance closed"
        );
    }
}

pub fn spawn_sse_pump(
    base_url: String,
    auth_header: String,
    thread_id: String,
    events_tx: broadcast::Sender<RawIngest>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            match try_sse_connect(&base_url, &auth_header, &thread_id, &events_tx).await {
                Ok(()) => {
                    info!(
                        target: "minos_agent_runtime::opencode_driver",
                        "SSE stream ended, reconnecting"
                    );
                }
                Err(e) => {
                    warn!(
                        target: "minos_agent_runtime::opencode_driver",
                        error = %e,
                        "SSE connection error, reconnecting"
                    );
                }
            }
            tokio::time::sleep(Duration::from_secs(2)).await;
        }
    })
}

async fn try_sse_connect(
    base_url: &str,
    auth_header: &str,
    thread_id: &str,
    events_tx: &broadcast::Sender<RawIngest>,
) -> anyhow::Result<()> {
    let client = reqwest::Client::new();
    let response = client
        .get(format!("{base_url}/event"))
        .header("Authorization", auth_header)
        .send()
        .await?;

    if !response.status().is_success() {
        anyhow::bail!("SSE endpoint returned {}", response.status());
    }

    let mut stream = response.bytes_stream().eventsource();
    while let Some(event) = stream.next().await {
        match event {
            Ok(sse_event) => {
                let payload: Value = match serde_json::from_str(&sse_event.data) {
                    Ok(v) => v,
                    Err(_) => serde_json::json!({
                        "kind": "raw",
                        "raw_kind": "sse",
                        "data": sse_event.data
                    }),
                };
                let _ = events_tx.send(RawIngest {
                    agent: AgentName::Opencode,
                    thread_id: thread_id.to_string(),
                    payload,
                    ts_ms: chrono::Utc::now().timestamp_millis(),
                });
            }
            Err(e) => {
                warn!(
                    target: "minos_agent_runtime::opencode_driver",
                    error = %e,
                    "SSE parse error"
                );
                break;
            }
        }
    }
    Ok(())
}
