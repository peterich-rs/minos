use std::path::PathBuf;
use std::sync::Arc;

use minos_agent_runtime::{
    AgentLaunchMode, AgentRuntime, AgentRuntimeConfig, AgentState, RawIngest,
};
use minos_domain::MinosError;
use minos_protocol::{
    AgentLaunchMode as ProtoAgentLaunchMode, SendUserMessageRequest, StartAgentRequest,
    StartAgentResponse,
};
use tokio::sync::{broadcast, watch};

use crate::subscription::{AgentStateObserver, Subscription};

pub struct AgentGlue {
    runtime: Arc<AgentRuntime>,
    state_rx: watch::Receiver<AgentState>,
}

impl AgentGlue {
    #[must_use]
    pub fn new(
        workspace_root: PathBuf,
        subprocess_env: Arc<std::collections::HashMap<String, String>>,
    ) -> Self {
        let mut cfg = AgentRuntimeConfig::new(workspace_root);
        cfg.subprocess_env = subprocess_env;
        Self::new_with_runtime(AgentRuntime::new(cfg))
    }

    #[must_use]
    pub fn new_with_runtime(runtime: Arc<AgentRuntime>) -> Self {
        let state_rx = runtime.state_stream();
        Self { runtime, state_rx }
    }

    pub async fn start_agent(
        &self,
        req: StartAgentRequest,
    ) -> Result<StartAgentResponse, MinosError> {
        self.refresh_subprocess_env().await;
        let mode = req.mode.map_or(AgentLaunchMode::Jsonl, runtime_mode);
        let out = self.runtime.start_with_mode(req.agent, mode).await?;
        Ok(StartAgentResponse {
            session_id: out.session_id,
            cwd: out.cwd,
        })
    }

    pub async fn send_user_message(&self, req: SendUserMessageRequest) -> Result<(), MinosError> {
        self.runtime
            .send_user_message(&req.session_id, &req.text)
            .await
    }

    pub async fn stop_agent(&self) -> Result<(), MinosError> {
        self.runtime.stop().await
    }

    #[must_use]
    pub fn subscribe_state(&self, observer: Arc<dyn AgentStateObserver>) -> Arc<Subscription> {
        crate::subscription::spawn_agent_observer(self.state_stream(), observer)
    }

    #[must_use]
    pub fn current_state(&self) -> AgentState {
        self.state_rx.borrow().clone()
    }

    #[must_use]
    pub fn state_stream(&self) -> watch::Receiver<AgentState> {
        self.state_rx.clone()
    }

    #[must_use]
    pub fn ingest_stream(&self) -> broadcast::Receiver<RawIngest> {
        self.runtime.ingest_stream()
    }

    pub async fn shutdown(&self) -> Result<(), MinosError> {
        self.runtime.stop().await
    }

    async fn refresh_subprocess_env(&self) {
        let env = minos_cli_detect::capture_user_shell_env().await;
        self.runtime.replace_subprocess_env(env);
    }
}

fn runtime_mode(mode: ProtoAgentLaunchMode) -> AgentLaunchMode {
    match mode {
        ProtoAgentLaunchMode::Jsonl => AgentLaunchMode::Jsonl,
        ProtoAgentLaunchMode::Server => AgentLaunchMode::Server,
    }
}
