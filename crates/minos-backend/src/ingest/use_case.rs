use std::sync::Arc;

use minos_domain::{AgentName, DeviceId};
use serde_json::Value;

use crate::approval_relay::ApprovalRelay;
use crate::error::BackendError;
use crate::ingest::{dispatch, translate::ThreadTranslators};
use crate::realtime::RealtimeFanout;
use crate::session::SessionRegistry;
use crate::store::StoreHandle;

#[derive(Debug, Clone)]
pub struct IngestCommand {
    pub agent: AgentName,
    pub thread_id: String,
    pub seq: u64,
    pub payload: Value,
    pub ts_ms: i64,
    pub owner_device_id: DeviceId,
}

pub struct IngestUseCase {
    store: StoreHandle,
    registry: Arc<SessionRegistry>,
    translators: Arc<ThreadTranslators>,
    approval_relay: Arc<ApprovalRelay>,
    realtime: Arc<RealtimeFanout>,
}

impl IngestUseCase {
    #[must_use]
    pub fn new(
        store: impl Into<StoreHandle>,
        registry: Arc<SessionRegistry>,
        translators: Arc<ThreadTranslators>,
        approval_relay: Arc<ApprovalRelay>,
        realtime: Arc<RealtimeFanout>,
    ) -> Arc<Self> {
        let store = store.into();
        Arc::new(Self {
            store,
            registry,
            translators,
            approval_relay,
            realtime,
        })
    }

    pub async fn execute(&self, command: IngestCommand) -> Result<(), BackendError> {
        dispatch(
            &self.store,
            self.registry.as_ref(),
            self.translators.as_ref(),
            self.approval_relay.as_ref(),
            self.realtime.as_ref(),
            command.agent,
            &command.thread_id,
            command.seq,
            &command.payload,
            command.ts_ms,
            command.owner_device_id,
        )
        .await
    }
}
