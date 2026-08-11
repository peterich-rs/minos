use std::sync::Arc;

use crate::approvals::ApprovalService;
use minos_domain::{AgentName, DeviceId};
use serde_json::Value;

use crate::error::BackendError;
use crate::ingest::{dispatch, translate::SessionTranslators};
use crate::realtime::RealtimeFanout;
use crate::store::StoreHandle;

#[derive(Debug, Clone)]
pub struct IngestCommand {
    pub agent: AgentName,
    pub session_id: String,
    pub seq: u64,
    pub payload: Value,
    pub ts_ms: i64,
    pub owner_device_id: DeviceId,
}

pub struct IngestUseCase {
    store: StoreHandle,
    translators: Arc<SessionTranslators>,
    approvals: Arc<dyn ApprovalService>,
    realtime: Arc<RealtimeFanout>,
}

impl IngestUseCase {
    #[must_use]
    pub fn new(
        store: impl Into<StoreHandle>,
        translators: Arc<SessionTranslators>,
        approvals: Arc<dyn ApprovalService>,
        realtime: Arc<RealtimeFanout>,
    ) -> Arc<Self> {
        let store = store.into();
        Arc::new(Self {
            store,
            translators,
            approvals,
            realtime,
        })
    }

    pub async fn execute(&self, command: IngestCommand) -> Result<(), BackendError> {
        dispatch(
            &self.store,
            self.translators.as_ref(),
            self.approvals.as_ref(),
            self.realtime.as_ref(),
            command.agent,
            &command.session_id,
            command.seq,
            &command.payload,
            command.ts_ms,
            command.owner_device_id,
        )
        .await
    }
}
