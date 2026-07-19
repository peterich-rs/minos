use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use serde::Serialize;

/// Per-job health state, updated by the supervisor after each tick.
#[derive(Debug, Clone, Serialize)]
pub struct JobHealth {
    pub name: String,
    pub last_ok_at_ms: Option<i64>,
    pub last_error: Option<String>,
    pub consecutive_failures: u32,
    pub total_ticks: u64,
    pub total_errors: u64,
}

/// Shared health registry that tracks the state of all running jobs.
#[derive(Debug, Clone, Default)]
pub struct JobHealthRegistry {
    inner: Arc<RwLock<HashMap<String, JobHealth>>>,
}

impl JobHealthRegistry {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a new job with initial health state.
    pub fn register(&self, name: &str) {
        let mut inner = self.inner.write().unwrap();
        inner.insert(
            name.to_string(),
            JobHealth {
                name: name.to_string(),
                last_ok_at_ms: None,
                last_error: None,
                consecutive_failures: 0,
                total_ticks: 0,
                total_errors: 0,
            },
        );
    }

    /// Record a successful tick.
    pub fn record_success(&self, name: &str, now_ms: i64) {
        let mut inner = self.inner.write().unwrap();
        if let Some(health) = inner.get_mut(name) {
            health.last_ok_at_ms = Some(now_ms);
            health.last_error = None;
            health.consecutive_failures = 0;
            health.total_ticks += 1;
        }
    }

    /// Record a failed tick.
    pub fn record_failure(&self, name: &str, error: &str) {
        let mut inner = self.inner.write().unwrap();
        if let Some(health) = inner.get_mut(name) {
            health.last_error = Some(error.to_string());
            health.consecutive_failures += 1;
            health.total_ticks += 1;
            health.total_errors += 1;
        }
    }

    /// Get the health state for all registered jobs.
    pub fn snapshot(&self) -> Vec<JobHealth> {
        let inner = self.inner.read().unwrap();
        inner.values().cloned().collect()
    }

    /// Get the consecutive failure count for a specific job.
    pub fn consecutive_failures(&self, name: &str) -> u32 {
        let inner = self.inner.read().unwrap();
        inner.get(name).map_or(0, |h| h.consecutive_failures)
    }
}
