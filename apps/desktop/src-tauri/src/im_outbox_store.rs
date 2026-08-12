//! Durable Desktop → Hub IM outbox (SQLite).
//!
//! Replaces localStorage `minos.im.outbox.v1`. Single-process app (single-instance
//! plugin) + SQLite transactions give fail-closed persistence without silent
//! truncation of unacked intents.

use std::path::PathBuf;
use std::sync::Mutex;

use anyhow::{anyhow, Context, Result};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

const MAX_ENTRIES: usize = 2_000;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ImOutboxEntryDto {
    pub id: String,
    pub kind: String,
    pub conversation_id: String,
    pub client_message_id: String,
    /// Immutable owner account. Empty = legacy quarantine (never claim).
    #[serde(default)]
    pub account_id: String,
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reply_to_message_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_runtimes: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_sent_at_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message_source: Option<String>,
    /// Structured AppendMessage mentions (wire shape kept as JSON values).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mentions: Option<Vec<serde_json::Value>>,
    pub status: String,
    pub attempts: i64,
    pub next_attempt_at: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

pub struct ImOutboxStore {
    conn: Mutex<Connection>,
}

impl ImOutboxStore {
    pub fn open_default() -> Result<Self> {
        let path = default_db_path()?;
        Self::open_path(path)
    }

    pub fn open_path(path: PathBuf) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("create im_outbox dir {}", parent.display()))?;
        }
        let conn = Connection::open(&path)
            .with_context(|| format!("open im_outbox db {}", path.display()))?;
        conn.execute_batch(
            "PRAGMA journal_mode=WAL;
             PRAGMA synchronous=NORMAL;
             PRAGMA foreign_keys=ON;
             CREATE TABLE IF NOT EXISTS im_outbox (
               client_message_id   TEXT PRIMARY KEY,
               id                  TEXT NOT NULL,
               kind                TEXT NOT NULL,
               conversation_id     TEXT NOT NULL,
               account_id          TEXT NOT NULL DEFAULT '',
               text                TEXT NOT NULL,
               title               TEXT,
               reply_to_message_id TEXT,
               agent_runtimes_json TEXT,
               agent_id            TEXT,
               agent_session_id    TEXT,
               client_sent_at_ms   INTEGER,
               message_source      TEXT,
               mentions_json       TEXT,
               status              TEXT NOT NULL
                 CHECK (status IN ('pending','inflight','acked','failed_terminal')),
               attempts            INTEGER NOT NULL DEFAULT 0,
               next_attempt_at     INTEGER NOT NULL,
               last_error          TEXT,
               created_at_ms       INTEGER NOT NULL,
               updated_at_ms       INTEGER NOT NULL
             );
             CREATE INDEX IF NOT EXISTS idx_im_outbox_status_due
               ON im_outbox(status, next_attempt_at);
             CREATE INDEX IF NOT EXISTS idx_im_outbox_conversation
               ON im_outbox(conversation_id, created_at_ms);
             CREATE INDEX IF NOT EXISTS idx_im_outbox_account_status_due
               ON im_outbox(account_id, status, next_attempt_at);",
        )?;
        // Optional structured mentions JSON (AppendMessage wire targets).
        // ALTER is idempotent: ignore "duplicate column" on existing DBs.
        let _ = conn.execute("ALTER TABLE im_outbox ADD COLUMN mentions_json TEXT", []);
        // Account ownership (empty = legacy quarantine). Idempotent ALTER.
        let _ = conn.execute(
            "ALTER TABLE im_outbox ADD COLUMN account_id TEXT NOT NULL DEFAULT ''",
            [],
        );
        let _ = conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_im_outbox_account_status_due
               ON im_outbox(account_id, status, next_attempt_at)",
            [],
        );
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    pub fn list_all(&self) -> Result<Vec<ImOutboxEntryDto>> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow!("im_outbox lock poisoned"))?;
        let mut stmt = conn.prepare(
            "SELECT client_message_id, id, kind, conversation_id, account_id, text, title,
                    reply_to_message_id, agent_runtimes_json, agent_id, agent_session_id,
                    client_sent_at_ms, message_source, mentions_json, status, attempts,
                    next_attempt_at, last_error, created_at_ms, updated_at_ms
               FROM im_outbox
           ORDER BY created_at_ms ASC, client_message_id ASC",
        )?;
        let rows = stmt.query_map([], map_row)?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }

    /// Replace full set (boot migrate / rare full sync). Fail-closed.
    /// Cap policy: drop oldest **acked** only. Never drops unacked intents —
    /// if unacked alone exceed MAX_ENTRIES, returns error.
    pub fn replace_all(&self, entries: &[ImOutboxEntryDto]) -> Result<()> {
        let mut conn = self
            .conn
            .lock()
            .map_err(|_| anyhow!("im_outbox lock poisoned"))?;
        let tx = conn.transaction()?;
        tx.execute("DELETE FROM im_outbox", [])?;
        let mut entries = entries.to_vec();
        if entries.len() > MAX_ENTRIES {
            entries.sort_by(|a, b| {
                a.created_at_ms
                    .cmp(&b.created_at_ms)
                    .then_with(|| a.client_message_id.cmp(&b.client_message_id))
            });
            let mut acked: Vec<_> = entries
                .iter()
                .filter(|e| e.status == "acked")
                .cloned()
                .collect();
            let rest: Vec<_> = entries
                .iter()
                .filter(|e| e.status != "acked")
                .cloned()
                .collect();
            acked.sort_by_key(|e| e.updated_at_ms);
            while acked.len() + rest.len() > MAX_ENTRIES && !acked.is_empty() {
                acked.remove(0);
            }
            if rest.len() > MAX_ENTRIES {
                return Err(anyhow!(
                    "im_outbox_capacity: {} unacked intents exceed max {}",
                    rest.len(),
                    MAX_ENTRIES
                ));
            }
            entries = rest;
            entries.extend(acked);
        }
        for e in &entries {
            insert_entry(&tx, e)?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn upsert(&self, entry: &ImOutboxEntryDto) -> Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow!("im_outbox lock poisoned"))?;
        // Capacity check + insert in one transaction so a rejected row never
        // remains durable (and later drainable) after the caller saw an error.
        let tx = conn
            .unchecked_transaction()
            .map_err(|e| anyhow!("im_outbox begin tx: {e}"))?;

        let existing: Option<String> = tx
            .query_row(
                "SELECT status FROM im_outbox WHERE client_message_id = ?1",
                params![entry.client_message_id],
                |r| r.get(0),
            )
            .optional()?;

        // New rows that would push unacked over the cap must fail before insert.
        // Updates of existing unacked rows are allowed (status/backoff churn).
        if existing.is_none() {
            let unacked: i64 = tx.query_row(
                "SELECT COUNT(*) FROM im_outbox WHERE status != 'acked'",
                [],
                |r| r.get(0),
            )?;
            let inserting_unacked = entry.status != "acked";
            if inserting_unacked && unacked >= MAX_ENTRIES as i64 {
                return Err(anyhow!(
                    "im_outbox_capacity: {} unacked intents already at max {}",
                    unacked,
                    MAX_ENTRIES
                ));
            }
        }

        tx.execute(
            "INSERT INTO im_outbox (
                client_message_id, id, kind, conversation_id, account_id, text, title,
                reply_to_message_id, agent_runtimes_json, agent_id, agent_session_id,
                client_sent_at_ms, message_source, mentions_json, status, attempts,
                next_attempt_at, last_error, created_at_ms, updated_at_ms
             ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20)
             ON CONFLICT(client_message_id) DO UPDATE SET
                id=excluded.id,
                kind=excluded.kind,
                conversation_id=excluded.conversation_id,
                account_id=excluded.account_id,
                text=excluded.text,
                title=excluded.title,
                reply_to_message_id=excluded.reply_to_message_id,
                agent_runtimes_json=excluded.agent_runtimes_json,
                agent_id=excluded.agent_id,
                agent_session_id=excluded.agent_session_id,
                client_sent_at_ms=excluded.client_sent_at_ms,
                message_source=excluded.message_source,
                mentions_json=excluded.mentions_json,
                status=excluded.status,
                attempts=excluded.attempts,
                next_attempt_at=excluded.next_attempt_at,
                last_error=excluded.last_error,
                updated_at_ms=excluded.updated_at_ms",
            params![
                entry.client_message_id,
                entry.id,
                entry.kind,
                entry.conversation_id,
                entry.account_id,
                entry.text,
                entry.title,
                entry.reply_to_message_id,
                runtimes_json(&entry.agent_runtimes),
                entry.agent_id,
                entry.agent_session_id,
                entry.client_sent_at_ms,
                entry.message_source,
                mentions_json(&entry.mentions),
                entry.status,
                entry.attempts,
                entry.next_attempt_at,
                entry.last_error,
                entry.created_at_ms,
                entry.updated_at_ms,
            ],
        )?;
        // Compact oldest acked rows when over cap; never delete unacked.
        Self::compact_acked_locked(&tx)?;
        let unacked: i64 = tx.query_row(
            "SELECT COUNT(*) FROM im_outbox WHERE status != 'acked'",
            [],
            |r| r.get(0),
        )?;
        if unacked > MAX_ENTRIES as i64 {
            // Defensive: should not happen after pre-check; abort without commit.
            return Err(anyhow!(
                "im_outbox_capacity: {} unacked intents exceed max {}",
                unacked,
                MAX_ENTRIES
            ));
        }
        tx.commit()?;
        Ok(())
    }

    fn compact_acked_locked(conn: &Connection) -> Result<()> {
        let total: i64 = conn.query_row("SELECT COUNT(*) FROM im_outbox", [], |r| r.get(0))?;
        if total <= MAX_ENTRIES as i64 {
            return Ok(());
        }
        let overflow = total - MAX_ENTRIES as i64;
        conn.execute(
            "DELETE FROM im_outbox WHERE client_message_id IN (
                SELECT client_message_id FROM im_outbox
                 WHERE status = 'acked'
              ORDER BY updated_at_ms ASC, client_message_id ASC
                 LIMIT ?1
            )",
            params![overflow],
        )?;
        Ok(())
    }

    pub fn clear(&self) -> Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow!("im_outbox lock poisoned"))?;
        conn.execute("DELETE FROM im_outbox", [])?;
        Ok(())
    }
}

fn insert_entry(tx: &rusqlite::Transaction<'_>, entry: &ImOutboxEntryDto) -> Result<()> {
    tx.execute(
        "INSERT INTO im_outbox (
            client_message_id, id, kind, conversation_id, account_id, text, title,
            reply_to_message_id, agent_runtimes_json, agent_id, agent_session_id,
            client_sent_at_ms, message_source, mentions_json, status, attempts,
            next_attempt_at, last_error, created_at_ms, updated_at_ms
         ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20)",
        params![
            entry.client_message_id,
            entry.id,
            entry.kind,
            entry.conversation_id,
            entry.account_id,
            entry.text,
            entry.title,
            entry.reply_to_message_id,
            runtimes_json(&entry.agent_runtimes),
            entry.agent_id,
            entry.agent_session_id,
            entry.client_sent_at_ms,
            entry.message_source,
            mentions_json(&entry.mentions),
            entry.status,
            entry.attempts,
            entry.next_attempt_at,
            entry.last_error,
            entry.created_at_ms,
            entry.updated_at_ms,
        ],
    )?;
    Ok(())
}

fn runtimes_json(v: &Option<Vec<String>>) -> Option<String> {
    v.as_ref()
        .map(|r| serde_json::to_string(r).unwrap_or_else(|_| "[]".into()))
}

fn mentions_json(v: &Option<Vec<serde_json::Value>>) -> Option<String> {
    v.as_ref().and_then(|m| {
        if m.is_empty() {
            None
        } else {
            serde_json::to_string(m).ok()
        }
    })
}

fn map_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ImOutboxEntryDto> {
    let runtimes_raw: Option<String> = row.get(8)?;
    let agent_runtimes = runtimes_raw.and_then(|s| serde_json::from_str(&s).ok());
    let mentions_raw: Option<String> = row.get(13)?;
    let mentions = mentions_raw.and_then(|s| serde_json::from_str(&s).ok());
    Ok(ImOutboxEntryDto {
        client_message_id: row.get(0)?,
        id: row.get(1)?,
        kind: row.get(2)?,
        conversation_id: row.get(3)?,
        account_id: row.get(4)?,
        text: row.get(5)?,
        title: row.get(6)?,
        reply_to_message_id: row.get(7)?,
        agent_runtimes,
        agent_id: row.get(9)?,
        agent_session_id: row.get(10)?,
        client_sent_at_ms: row.get(11)?,
        message_source: row.get(12)?,
        mentions,
        status: row.get(14)?,
        attempts: row.get(15)?,
        next_attempt_at: row.get(16)?,
        last_error: row.get(17)?,
        created_at_ms: row.get(18)?,
        updated_at_ms: row.get(19)?,
    })
}

fn default_db_path() -> Result<PathBuf> {
    let base = dirs::data_dir().ok_or_else(|| anyhow!("no data_dir for im_outbox"))?;
    Ok(base.join("minos").join("desktop").join("im_outbox.sqlite3"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn sample(id: &str, status: &str) -> ImOutboxEntryDto {
        ImOutboxEntryDto {
            id: format!("outbox:{id}"),
            kind: "user_message".into(),
            conversation_id: "c1".into(),
            client_message_id: id.into(),
            account_id: "acct-a".into(),
            text: "hi".into(),
            title: None,
            reply_to_message_id: None,
            agent_runtimes: None,
            agent_id: None,
            agent_session_id: None,
            client_sent_at_ms: Some(1),
            message_source: Some("client_live".into()),
            mentions: None,
            status: status.into(),
            attempts: 0,
            next_attempt_at: 1,
            last_error: None,
            created_at_ms: 1,
            updated_at_ms: 1,
        }
    }

    #[test]
    fn upsert_and_list_round_trip() {
        let dir = tempdir().unwrap();
        let store = ImOutboxStore::open_path(dir.path().join("t.db")).unwrap();
        store.upsert(&sample("m1", "pending")).unwrap();
        let all = store.list_all().unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].client_message_id, "m1");
        store.upsert(&sample("m1", "acked")).unwrap();
        let all = store.list_all().unwrap();
        assert_eq!(all[0].status, "acked");
    }

    #[test]
    fn replace_all_is_atomic() {
        let dir = tempdir().unwrap();
        let store = ImOutboxStore::open_path(dir.path().join("t.db")).unwrap();
        store.upsert(&sample("a", "pending")).unwrap();
        store
            .replace_all(&[sample("b", "pending"), sample("c", "inflight")])
            .unwrap();
        let ids: Vec<_> = store
            .list_all()
            .unwrap()
            .into_iter()
            .map(|e| e.client_message_id)
            .collect();
        assert_eq!(ids, vec!["b".to_string(), "c".to_string()]);
    }

    #[test]
    fn replace_all_never_drops_unacked() {
        let dir = tempdir().unwrap();
        let store = ImOutboxStore::open_path(dir.path().join("t.db")).unwrap();
        let mut rows = Vec::new();
        for i in 0..(MAX_ENTRIES + 5) {
            let mut s = sample(&format!("p{i}"), "pending");
            s.created_at_ms = i as i64;
            rows.push(s);
        }
        let err = store.replace_all(&rows).unwrap_err();
        assert!(
            err.to_string().contains("im_outbox_capacity"),
            "expected capacity error, got {err}"
        );
    }

    #[test]
    fn upsert_compacts_acked_not_pending() {
        let dir = tempdir().unwrap();
        let store = ImOutboxStore::open_path(dir.path().join("t.db")).unwrap();
        for i in 0..MAX_ENTRIES {
            let mut s = sample(&format!("a{i}"), "acked");
            s.updated_at_ms = i as i64;
            store.upsert(&s).unwrap();
        }
        // One more pending must fit by dropping oldest acked.
        store.upsert(&sample("pending-new", "pending")).unwrap();
        let all = store.list_all().unwrap();
        assert!(all.iter().any(|e| e.client_message_id == "pending-new"));
        assert!(all.len() <= MAX_ENTRIES);
        assert!(all
            .iter()
            .all(|e| e.status != "acked" || e.client_message_id != "a0"));
    }

    #[test]
    fn upsert_capacity_rejects_without_persisting() {
        let dir = tempdir().unwrap();
        let store = ImOutboxStore::open_path(dir.path().join("t.db")).unwrap();
        for i in 0..MAX_ENTRIES {
            store.upsert(&sample(&format!("p{i}"), "pending")).unwrap();
        }
        let err = store.upsert(&sample("overflow", "pending")).unwrap_err();
        assert!(
            err.to_string().contains("im_outbox_capacity"),
            "expected capacity error, got {err}"
        );
        let all = store.list_all().unwrap();
        assert_eq!(all.len(), MAX_ENTRIES);
        assert!(!all.iter().any(|e| e.client_message_id == "overflow"));
    }
}
