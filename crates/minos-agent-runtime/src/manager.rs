use crate::instance::AppServerInstance;
use crate::manager_event::ManagerEvent;
use crate::state_machine::ThreadState;
use crate::thread_handle::ThreadHandle;
use crate::{AgentKind, AgentRuntimeConfig, RawIngest};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
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

    pub async fn start_agent(
        &self,
        agent: AgentKind,
        workspace: PathBuf,
    ) -> anyhow::Result<StartAgentOutcome> {
        let canon = std::fs::canonicalize(&workspace).unwrap_or_else(|_| workspace.clone());
        let instance = self.ensure_instance(&canon).await?;
        let thread_id = instance.start_thread().await?;
        instance.add_thread(thread_id.clone()).await;
        instance.touch().await;

        let handle = ThreadHandle::new(
            thread_id.clone(),
            canon.clone(),
            agent,
            ThreadState::Starting,
            0,
        );
        self.threads
            .lock()
            .await
            .insert(thread_id.clone(), handle.clone());
        let _ = self.manager_tx.send(ManagerEvent::ThreadAdded {
            thread_id: thread_id.clone(),
            workspace: canon.clone(),
            agent,
        });
        Ok(StartAgentOutcome {
            thread_id,
            cwd: canon,
        })
    }

    async fn ensure_instance(&self, workspace: &Path) -> anyhow::Result<Arc<AppServerInstance>> {
        let mut guard = self.instances.lock().await;
        if let Some(existing) = guard.get(workspace) {
            return Ok(existing.clone());
        }
        if guard.len() >= self.caps.max_instances {
            self.lru_evict(&mut guard).await?;
        }
        let inst = self.spawn_instance(workspace).await?;
        guard.insert(workspace.to_path_buf(), inst.clone());
        Ok(inst)
    }

    async fn spawn_instance(&self, _workspace: &Path) -> anyhow::Result<Arc<AppServerInstance>> {
        anyhow::bail!("spawn_instance unimplemented (C12)")
    }

    async fn lru_evict(
        &self,
        _map: &mut HashMap<PathBuf, Arc<AppServerInstance>>,
    ) -> anyhow::Result<()> {
        anyhow::bail!("evict unimplemented (C19)")
    }

    pub async fn list_threads(&self) -> Vec<crate::store_facing::ThreadSnapshot> {
        let g = self.threads.lock().await;
        g.values()
            .map(|h| crate::store_facing::ThreadSnapshot {
                thread_id: h.thread_id.clone(),
                workspace: h.workspace.clone(),
                state: h.current_state(),
            })
            .collect()
    }
}

#[derive(Clone, Debug)]
pub struct StartAgentOutcome {
    pub thread_id: String,
    pub cwd: PathBuf,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    #[ignore = "spawn_instance is stubbed until C12"]
    async fn start_agent_creates_instance_and_thread() {
        let tmp = tempfile::tempdir().unwrap();
        let cfg = AgentRuntimeConfig::new(tmp.path().to_path_buf());
        let mgr = AgentManager::new(cfg, InstanceCaps::default());
        let ws = std::path::PathBuf::from("/w-test");
        let resp = mgr.start_agent(AgentKind::Codex, ws.clone()).await.unwrap();
        assert_eq!(resp.cwd, ws);
        let snap = mgr.list_threads().await;
        assert_eq!(snap.len(), 1);
        assert_eq!(snap[0].workspace, ws);
    }
}
