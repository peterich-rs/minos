//! Durable BotInboxDelivery ledger — restart-safe effective exactly-once inject.
//!
//! Transport is at-least-once; this table is the Host authority for whether a
//! `delivery_id` was already received / injected / rejected.

use sqlx::Row;

use super::LocalStore;

pub const STATUS_RECEIVED: &str = "received";
pub const STATUS_INJECTED: &str = "injected";
pub const STATUS_COMPLETED: &str = "completed";
pub const STATUS_REJECTED: &str = "rejected";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BotDeliveryLedgerRow {
    pub delivery_id: String,
    pub conversation_id: String,
    pub bot_id: String,
    pub session_id: Option<String>,
    pub status: String,
    pub accepted: Option<bool>,
    pub last_error: Option<String>,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

impl LocalStore {
    pub async fn get_bot_delivery(
        &self,
        delivery_id: &str,
    ) -> anyhow::Result<Option<BotDeliveryLedgerRow>> {
        let id = delivery_id.trim();
        if id.is_empty() {
            return Ok(None);
        }
        let row = sqlx::query(
            "SELECT delivery_id, conversation_id, bot_id, session_id, status,
                    accepted, last_error, created_at_ms, updated_at_ms
               FROM bot_delivery_ledger
              WHERE delivery_id = ?1",
        )
        .bind(id)
        .fetch_optional(self.pool())
        .await?;
        Ok(row.map(|r| BotDeliveryLedgerRow {
            delivery_id: r.get("delivery_id"),
            conversation_id: r.get("conversation_id"),
            bot_id: r.get("bot_id"),
            session_id: r.get("session_id"),
            status: r.get("status"),
            accepted: r.get::<Option<i64>, _>("accepted").map(|v| v != 0),
            last_error: r.get("last_error"),
            created_at_ms: r.get("created_at_ms"),
            updated_at_ms: r.get("updated_at_ms"),
        }))
    }

    /// Record first sight of a delivery. Returns true if this process should start inject.
    /// Existing terminal rows return false (caller should replay accept/reject).
    /// Existing `received`/`injected` without terminal means in-flight / already handled.
    pub async fn begin_bot_delivery(
        &self,
        delivery_id: &str,
        conversation_id: &str,
        bot_id: &str,
        session_id: Option<&str>,
        now_ms: i64,
    ) -> anyhow::Result<BotDeliveryBegin> {
        let id = delivery_id.trim();
        if id.is_empty() {
            return Ok(BotDeliveryBegin::Start);
        }
        if let Some(existing) = self.get_bot_delivery(id).await? {
            return Ok(match existing.status.as_str() {
                STATUS_REJECTED => BotDeliveryBegin::ReplayRejected,
                STATUS_INJECTED | STATUS_COMPLETED if existing.accepted == Some(true) => {
                    BotDeliveryBegin::ReplayAccepted
                }
                STATUS_INJECTED | STATUS_COMPLETED if existing.accepted == Some(false) => {
                    BotDeliveryBegin::ReplayRejected
                }
                STATUS_RECEIVED | STATUS_INJECTED | STATUS_COMPLETED => BotDeliveryBegin::InFlight,
                _ => BotDeliveryBegin::InFlight,
            });
        }
        let result = sqlx::query(
            "INSERT INTO bot_delivery_ledger (
                delivery_id, conversation_id, bot_id, session_id, status,
                accepted, last_error, created_at_ms, updated_at_ms
             ) VALUES (?1, ?2, ?3, ?4, ?5, NULL, NULL, ?6, ?6)
             ON CONFLICT(delivery_id) DO NOTHING",
        )
        .bind(id)
        .bind(conversation_id)
        .bind(bot_id)
        .bind(session_id)
        .bind(STATUS_RECEIVED)
        .bind(now_ms)
        .execute(self.pool())
        .await?;
        if result.rows_affected() > 0 {
            Ok(BotDeliveryBegin::Start)
        } else {
            // Race: another task inserted between get and insert.
            if let Some(existing) = self.get_bot_delivery(id).await? {
                return Ok(match existing.status.as_str() {
                    STATUS_REJECTED => BotDeliveryBegin::ReplayRejected,
                    STATUS_INJECTED | STATUS_COMPLETED if existing.accepted == Some(true) => {
                        BotDeliveryBegin::ReplayAccepted
                    }
                    STATUS_INJECTED | STATUS_COMPLETED if existing.accepted == Some(false) => {
                        BotDeliveryBegin::ReplayRejected
                    }
                    _ => BotDeliveryBegin::InFlight,
                });
            }
            Ok(BotDeliveryBegin::InFlight)
        }
    }

    pub async fn mark_bot_delivery_injected(
        &self,
        delivery_id: &str,
        session_id: Option<&str>,
        now_ms: i64,
    ) -> anyhow::Result<()> {
        let id = delivery_id.trim();
        if id.is_empty() {
            return Ok(());
        }
        sqlx::query(
            "UPDATE bot_delivery_ledger
                SET status = ?1,
                    accepted = 1,
                    session_id = COALESCE(?2, session_id),
                    updated_at_ms = ?3
              WHERE delivery_id = ?4",
        )
        .bind(STATUS_INJECTED)
        .bind(session_id)
        .bind(now_ms)
        .bind(id)
        .execute(self.pool())
        .await?;
        Ok(())
    }

    pub async fn mark_bot_delivery_rejected(
        &self,
        delivery_id: &str,
        last_error: &str,
        now_ms: i64,
    ) -> anyhow::Result<()> {
        let id = delivery_id.trim();
        if id.is_empty() {
            return Ok(());
        }
        sqlx::query(
            "UPDATE bot_delivery_ledger
                SET status = ?1,
                    accepted = 0,
                    last_error = ?2,
                    updated_at_ms = ?3
              WHERE delivery_id = ?4",
        )
        .bind(STATUS_REJECTED)
        .bind(last_error.chars().take(500).collect::<String>())
        .bind(now_ms)
        .bind(id)
        .execute(self.pool())
        .await?;
        Ok(())
    }

    /// Drop non-terminal received marker so Hub requeue can redeliver after retryable failure.
    pub async fn clear_bot_delivery_inflight(&self, delivery_id: &str) -> anyhow::Result<()> {
        let id = delivery_id.trim();
        if id.is_empty() {
            return Ok(());
        }
        sqlx::query(
            "DELETE FROM bot_delivery_ledger
              WHERE delivery_id = ?1 AND status = ?2",
        )
        .bind(id)
        .bind(STATUS_RECEIVED)
        .execute(self.pool())
        .await?;
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BotDeliveryBegin {
    Start,
    InFlight,
    ReplayAccepted,
    ReplayRejected,
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[tokio::test]
    async fn begin_inject_reject_round_trip() {
        let dir = tempdir().unwrap();
        let store = LocalStore::open(&dir.path().join("t.sqlite"))
            .await
            .unwrap();
        let now = 1_700_000_000_000i64;
        assert_eq!(
            store
                .begin_bot_delivery("d1", "c1", "bot-1", None, now)
                .await
                .unwrap(),
            BotDeliveryBegin::Start
        );
        assert_eq!(
            store
                .begin_bot_delivery("d1", "c1", "bot-1", None, now + 1)
                .await
                .unwrap(),
            BotDeliveryBegin::InFlight
        );
        store
            .mark_bot_delivery_injected("d1", Some("sess-1"), now + 2)
            .await
            .unwrap();
        assert_eq!(
            store
                .begin_bot_delivery("d1", "c1", "bot-1", None, now + 3)
                .await
                .unwrap(),
            BotDeliveryBegin::ReplayAccepted
        );
        store
            .mark_bot_delivery_rejected("d2", "no workspace", now)
            .await
            .unwrap();
        // Rejected path: insert received then reject — use begin then reject.
        assert_eq!(
            store
                .begin_bot_delivery("d3", "c1", "bot-1", None, now)
                .await
                .unwrap(),
            BotDeliveryBegin::Start
        );
        store
            .mark_bot_delivery_rejected("d3", "host_not_ready", now + 1)
            .await
            .unwrap();
        assert_eq!(
            store
                .begin_bot_delivery("d3", "c1", "bot-1", None, now + 2)
                .await
                .unwrap(),
            BotDeliveryBegin::ReplayRejected
        );
    }

    #[tokio::test]
    async fn clear_inflight_allows_redelivery() {
        let dir = tempdir().unwrap();
        let store = LocalStore::open(&dir.path().join("t.sqlite"))
            .await
            .unwrap();
        let now = 1_700_000_000_000i64;
        assert_eq!(
            store
                .begin_bot_delivery("d1", "c1", "bot-1", None, now)
                .await
                .unwrap(),
            BotDeliveryBegin::Start
        );
        store.clear_bot_delivery_inflight("d1").await.unwrap();
        assert_eq!(
            store
                .begin_bot_delivery("d1", "c1", "bot-1", None, now + 1)
                .await
                .unwrap(),
            BotDeliveryBegin::Start
        );
    }
}
