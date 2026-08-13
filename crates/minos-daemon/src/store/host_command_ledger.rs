//! Durable host-command ledger — restart-safe effective exactly-once execution.
//!
//! Transport is at-least-once; this table is the Host authority for whether a
//! `command_id` was already started / completed so Subscribe replay cannot
//! re-execute history after daemon restart.

use sqlx::Row;

use super::LocalStore;

pub const STATUS_INFLIGHT: &str = "inflight";
pub const STATUS_COMPLETED: &str = "completed";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostCommandLedgerRow {
    pub command_id: String,
    pub status: String,
    pub succeeded: Option<bool>,
    pub result_json: Option<String>,
    pub error_json: Option<String>,
    pub finished_at_ms: Option<i64>,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HostCommandBegin {
    Start,
    InFlight,
    Replay {
        succeeded: bool,
        result_json: Option<String>,
        error_json: Option<String>,
        finished_at_ms: i64,
    },
}

impl LocalStore {
    pub async fn get_host_command_ledger(
        &self,
        command_id: &str,
    ) -> anyhow::Result<Option<HostCommandLedgerRow>> {
        let id = command_id.trim();
        if id.is_empty() {
            return Ok(None);
        }
        let row = sqlx::query(
            "SELECT command_id, status, succeeded, result_json, error_json,
                    finished_at_ms, created_at_ms, updated_at_ms
               FROM host_command_ledger
              WHERE command_id = ?1",
        )
        .bind(id)
        .fetch_optional(self.pool())
        .await?;
        Ok(row.map(|r| HostCommandLedgerRow {
            command_id: r.get("command_id"),
            status: r.get("status"),
            succeeded: r.get::<Option<i64>, _>("succeeded").map(|v| v != 0),
            result_json: r.get("result_json"),
            error_json: r.get("error_json"),
            finished_at_ms: r.get("finished_at_ms"),
            created_at_ms: r.get("created_at_ms"),
            updated_at_ms: r.get("updated_at_ms"),
        }))
    }

    /// Record first sight of a host command. Returns whether this process should start it.
    pub async fn begin_host_command(
        &self,
        command_id: &str,
        now_ms: i64,
    ) -> anyhow::Result<HostCommandBegin> {
        let id = command_id.trim();
        if id.is_empty() {
            return Ok(HostCommandBegin::Start);
        }
        if let Some(existing) = self.get_host_command_ledger(id).await? {
            return Ok(classify_begin(&existing));
        }
        let result = sqlx::query(
            "INSERT INTO host_command_ledger (
                command_id, status, succeeded, result_json, error_json,
                finished_at_ms, created_at_ms, updated_at_ms
             ) VALUES (?1, ?2, NULL, NULL, NULL, NULL, ?3, ?3)
             ON CONFLICT(command_id) DO NOTHING",
        )
        .bind(id)
        .bind(STATUS_INFLIGHT)
        .bind(now_ms)
        .execute(self.pool())
        .await?;
        if result.rows_affected() > 0 {
            Ok(HostCommandBegin::Start)
        } else if let Some(existing) = self.get_host_command_ledger(id).await? {
            Ok(classify_begin(&existing))
        } else {
            Ok(HostCommandBegin::InFlight)
        }
    }

    pub async fn complete_host_command(
        &self,
        command_id: &str,
        succeeded: bool,
        result_json: Option<&str>,
        error_json: Option<&str>,
        finished_at_ms: i64,
    ) -> anyhow::Result<()> {
        let id = command_id.trim();
        if id.is_empty() {
            return Ok(());
        }
        sqlx::query(
            "INSERT INTO host_command_ledger (
                command_id, status, succeeded, result_json, error_json,
                finished_at_ms, created_at_ms, updated_at_ms
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6, ?6)
             ON CONFLICT(command_id) DO UPDATE SET
                status = excluded.status,
                succeeded = excluded.succeeded,
                result_json = excluded.result_json,
                error_json = excluded.error_json,
                finished_at_ms = excluded.finished_at_ms,
                updated_at_ms = excluded.updated_at_ms",
        )
        .bind(id)
        .bind(STATUS_COMPLETED)
        .bind(i64::from(succeeded))
        .bind(result_json)
        .bind(error_json)
        .bind(finished_at_ms)
        .execute(self.pool())
        .await?;
        Ok(())
    }

    pub async fn get_host_topic_cursor(&self, topic: &str) -> anyhow::Result<i64> {
        let topic = topic.trim();
        if topic.is_empty() {
            return Ok(0);
        }
        let seq = sqlx::query_scalar::<_, i64>(
            "SELECT topic_seq FROM host_topic_cursors WHERE topic = ?1",
        )
        .bind(topic)
        .fetch_optional(self.pool())
        .await?;
        Ok(seq.unwrap_or(0).max(0))
    }

    pub async fn set_host_topic_cursor(
        &self,
        topic: &str,
        topic_seq: i64,
        now_ms: i64,
    ) -> anyhow::Result<()> {
        let topic = topic.trim();
        if topic.is_empty() || topic_seq < 0 {
            return Ok(());
        }
        sqlx::query(
            "INSERT INTO host_topic_cursors (topic, topic_seq, updated_at_ms)
             VALUES (?1, ?2, ?3)
             ON CONFLICT(topic) DO UPDATE SET
                topic_seq = MAX(host_topic_cursors.topic_seq, excluded.topic_seq),
                updated_at_ms = excluded.updated_at_ms",
        )
        .bind(topic)
        .bind(topic_seq)
        .bind(now_ms)
        .execute(self.pool())
        .await?;
        Ok(())
    }

    /// Force-set cursor (SnapshotRequired resumes from retention floor).
    pub async fn replace_host_topic_cursor(
        &self,
        topic: &str,
        topic_seq: i64,
        now_ms: i64,
    ) -> anyhow::Result<()> {
        let topic = topic.trim();
        if topic.is_empty() || topic_seq < 0 {
            return Ok(());
        }
        sqlx::query(
            "INSERT INTO host_topic_cursors (topic, topic_seq, updated_at_ms)
             VALUES (?1, ?2, ?3)
             ON CONFLICT(topic) DO UPDATE SET
                topic_seq = excluded.topic_seq,
                updated_at_ms = excluded.updated_at_ms",
        )
        .bind(topic)
        .bind(topic_seq)
        .bind(now_ms)
        .execute(self.pool())
        .await?;
        Ok(())
    }
}

fn classify_begin(existing: &HostCommandLedgerRow) -> HostCommandBegin {
    match existing.status.as_str() {
        STATUS_COMPLETED => HostCommandBegin::Replay {
            succeeded: existing.succeeded.unwrap_or(false),
            result_json: existing.result_json.clone(),
            error_json: existing.error_json.clone(),
            finished_at_ms: existing.finished_at_ms.unwrap_or(existing.updated_at_ms),
        },
        _ => HostCommandBegin::InFlight,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[tokio::test]
    async fn begin_complete_replay_and_cursor() {
        let dir = tempdir().unwrap();
        let store = LocalStore::open(&dir.path().join("t.sqlite"))
            .await
            .unwrap();
        let now = 1_700_000_000_000i64;
        assert_eq!(
            store.begin_host_command("cmd-1", now).await.unwrap(),
            HostCommandBegin::Start
        );
        assert_eq!(
            store.begin_host_command("cmd-1", now + 1).await.unwrap(),
            HostCommandBegin::InFlight
        );
        store
            .complete_host_command("cmd-1", true, Some(r#"{"ok":true}"#), None, now + 2)
            .await
            .unwrap();
        match store.begin_host_command("cmd-1", now + 3).await.unwrap() {
            HostCommandBegin::Replay {
                succeeded,
                result_json,
                ..
            } => {
                assert!(succeeded);
                assert_eq!(result_json.as_deref(), Some(r#"{"ok":true}"#));
            }
            other => panic!("expected replay, got {other:?}"),
        }

        store
            .set_host_topic_cursor("host:abc", 10, now)
            .await
            .unwrap();
        store
            .set_host_topic_cursor("host:abc", 8, now + 1)
            .await
            .unwrap();
        assert_eq!(store.get_host_topic_cursor("host:abc").await.unwrap(), 10);
        store
            .replace_host_topic_cursor("host:abc", 5, now + 2)
            .await
            .unwrap();
        assert_eq!(store.get_host_topic_cursor("host:abc").await.unwrap(), 5);
    }
}
