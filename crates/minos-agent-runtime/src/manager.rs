use crate::instance::AppServerInstance;
use crate::manager_event::ManagerEvent;
use crate::state_machine::ThreadState;
use crate::thread_handle::ThreadHandle;
use crate::{AgentRuntimeConfig, RawIngest};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::{Mutex, broadcast, watch};

#[derive(Clone, Debug)]
pub struct InstanceCaps {
    pub max_instances: usize,
    pub idle_timeout: std::time::Duration,
}

impl Default for InstanceCaps {
    fn default() -> Self {
        Self {
            max_instances: 8,
            idle_timeout: std::time::Duration::from_secs(30 * 60),
        }
    }
}

pub struct AgentManager {
    pub config: Arc<AgentRuntimeConfig>,
    pub caps: InstanceCaps,
    #[allow(dead_code)]
    pub(crate) instances: Arc<Mutex<HashMap<PathBuf, Arc<AppServerInstance>>>>,
    pub(crate) threads: Arc<Mutex<HashMap<String, ThreadHandle>>>,
    pub(crate) events_tx: broadcast::Sender<RawIngest>,
    pub(crate) manager_tx: broadcast::Sender<ManagerEvent>,
}

impl AgentManager {
    pub fn new(config: AgentRuntimeConfig, caps: InstanceCaps) -> Self {
        let (events_tx, _) = broadcast::channel(256);
        let (manager_tx, _) = broadcast::channel(64);
        Self {
            config: Arc::new(config),
            caps,
            instances: Arc::new(Mutex::new(HashMap::new())),
            threads: Arc::new(Mutex::new(HashMap::new())),
            events_tx,
            manager_tx,
        }
    }

    pub fn ingest_stream(&self) -> broadcast::Receiver<RawIngest> {
        self.events_tx.subscribe()
    }

    pub fn manager_event_stream(&self) -> broadcast::Receiver<ManagerEvent> {
        self.manager_tx.subscribe()
    }

    pub async fn thread_state_stream(
        &self,
        thread_id: &str,
    ) -> Option<watch::Receiver<ThreadState>> {
        self.threads
            .lock()
            .await
            .get(thread_id)
            .map(|h| h.state_rx.clone())
    }
}
