use std::path::PathBuf;
use std::sync::Arc;
use std::time::SystemTime;

use minos_agent_runtime::{AgentRuntime, AgentRuntimeConfig, AgentState as RuntimeAgentState};
use minos_domain::{AgentEvent, AgentName, MinosError};
use minos_protocol::{SendUserMessageRequest, StartAgentRequest, StartAgentResponse};
use tokio::sync::{broadcast, watch};

use crate::subscription::{AgentStateObserver, Subscription};

#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum AgentState {
    #[default]
    Idle,
    Starting {
        agent: AgentName,
    },
    Running {
        agent: AgentName,
        thread_id: String,
        started_at: SystemTime,
    },
    Stopping,
    Crashed {
        reason: String,
    },
}

impl From<RuntimeAgentState> for AgentState {
    fn from(state: RuntimeAgentState) -> Self {
        match state {
            RuntimeAgentState::Idle => Self::Idle,
            RuntimeAgentState::Starting { agent } => Self::Starting { agent },
            RuntimeAgentState::Running {
                agent,
                thread_id,
                started_at,
            } => Self::Running {
                agent,
                thread_id,
                started_at,
            },
            RuntimeAgentState::Stopping => Self::Stopping,
            RuntimeAgentState::Crashed { reason } => Self::Crashed { reason },
        }
    }
}

pub struct AgentGlue {
    runtime: Arc<AgentRuntime>,
    state_rx: watch::Receiver<RuntimeAgentState>,
}

impl AgentGlue {
    #[must_use]
    pub fn new(workspace_root: PathBuf) -> Self {
        Self::new_with_runtime(AgentRuntime::new(AgentRuntimeConfig::new(workspace_root)))
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
        let out = self.runtime.start(req.agent).await?;
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
        self.state_rx.borrow().clone().into()
    }

    #[must_use]
    pub fn state_stream(&self) -> watch::Receiver<RuntimeAgentState> {
        self.state_rx.clone()
    }

    #[must_use]
    pub fn event_stream(&self) -> broadcast::Receiver<AgentEvent> {
        self.runtime.event_stream()
    }

    pub async fn shutdown(&self) -> Result<(), MinosError> {
        self.runtime.stop().await
    }
}
