//! Local-RPC dispatch for the seven backend-terminated methods.
//!
//! The WebSocket dispatcher (`envelope::mod`) decodes an incoming
//! [`Envelope::LocalRpc`] frame and hands the (id, method, params) triple
//! to [`handle`]. Each method maps to a single async fn on
//! [`LocalRpcContext`]; the master [`handle`] is a thin match over
//! [`LocalRpcMethod`] that returns a [`LocalRpcOutcome`] for the dispatcher
//! to wrap back up into an [`Envelope::LocalRpcResponse`].
//!
//! # Method menu (spec §6.1 / §6.2)
//!
//! | Method | Role gate | Pre-state | Notes |
//! |---|---|---|---|
//! | `Ping` | any | any | returns `{"ok": true}` verbatim |
//! | `RequestPairingQr` | `agent-host` | any | returns a full `PairingQrPayload` v2 |
//! | `Pair` | `ios-client` | unpaired | consumes token, emits `Event::Paired` to issuer |
//! | `ForgetPeer` | any | paired | emits `Event::Unpaired` to both sides |
//! | `ListThreads` | any | paired | scopes to caller's pairing partner |
//! | `ReadThread` | any | paired | fresh translator state per call (no bleed from live-ingest) |
//! | `GetThreadLastSeq` | any | paired | MAX(seq) for the given thread_id, 0 if empty |
//!
//! # Error code strings
//!
//! Snake-case, stable across releases; clients match on these:
//!
//! - `"unauthorized"` — role-gate violated OR pre-state violated (e.g.
//!   `list_threads` when the session is unpaired). Per spec §11.2 U2 the
//!   mobile UI maps this to "toast + route back to PairingPage".
//! - `"bad_request"` — params failed to deserialise against the method's
//!   schema.
//! - `"pairing_token_invalid"` — unknown/expired/consumed pairing token.
//! - `"pairing_state_mismatch"` — already paired / not paired for the method.
//! - `"thread_not_found"` — `ReadThread` called with an unknown thread id.
//! - `"internal"` — any underlying [`BackendError::StoreQuery`] etc. Caller
//!   should log and retry / escalate.
//!
//! # Peer-event delivery (§10.2 R4)
//!
//! After a successful `pair`, we push `Event::Paired` onto the issuer's
//! outbox via `try_send`. If the issuer is offline or its outbox rejects
//! the event, we compensate the already-committed pair by clearing the DB
//! pairing row, revoking both secret hashes, and restoring the in-memory
//! `paired_with` mirrors to the unpaired state.

use std::time::Duration;

use minos_domain::{DeviceRole, DeviceSecret};
use minos_protocol::{Envelope, EventKind, LocalRpcMethod, LocalRpcOutcome, RpcError};
use serde::Deserialize;
use sqlx::SqlitePool;

use crate::{
    error::BackendError,
    pairing::PairingService,
    session::{SessionHandle, SessionRegistry},
    store::devices,
};

/// Read-only context threaded into every per-method handler.
///
/// Reference-only so the dispatcher can build a fresh context per inbound
/// frame without cloning `Arc`s on the hot path.
pub struct LocalRpcContext<'a> {
    pub session: &'a SessionHandle,
    pub registry: &'a SessionRegistry,
    pub pairing: &'a PairingService,
    pub store: &'a SqlitePool,
    pub token_ttl: Duration,
    /// Public WebSocket origin the backend advertises to mobile via the
    /// pairing QR payload (spec §6.1 `PairingQrPayload.backend_url`).
    pub public_url: &'a str,
    /// Optional Cloudflare Access client id to embed in the QR payload for
    /// legacy deployments. Current clients normally get Access credentials
    /// from build-time / host env config instead.
    pub cf_access_client_id: Option<&'a str>,
    /// Optional Cloudflare Access client secret to embed in the QR payload.
    /// MUST be `Some` iff `cf_access_client_id` is `Some` (checked by
    /// [`crate::config::Config::validate`] at startup).
    pub cf_access_client_secret: Option<&'a str>,
}

/// Dispatch a decoded [`Envelope::LocalRpc`] to the right handler.
///
/// `id` is echoed back by the caller; this fn returns only the outcome.
pub async fn handle(
    ctx: &LocalRpcContext<'_>,
    method: &LocalRpcMethod,
    params: &serde_json::Value,
) -> LocalRpcOutcome {
    match method {
        LocalRpcMethod::Ping => handle_ping().await,
        LocalRpcMethod::RequestPairingQr => handle_request_pairing_token(ctx, params).await,
        LocalRpcMethod::Pair => handle_pair(ctx, params).await,
        LocalRpcMethod::ForgetPeer => handle_forget_peer(ctx).await,
        LocalRpcMethod::ListThreads => handle_list_threads(ctx, params).await,
        LocalRpcMethod::ReadThread => handle_read_thread(ctx, params).await,
        LocalRpcMethod::GetThreadLastSeq => handle_get_thread_last_seq(ctx, params).await,
    }
}

/// Backend → caller. Return a paginated list of thread summaries owned by
/// the caller's paired peer (`session.paired_with`). Capped at 500 rows.
///
/// Pre-state: paired. Unpaired callers get `unauthorized` so the mobile UI
/// can route back to `PairingPage` per spec §11.2 U2 — a silent empty list
/// would look like "no threads yet" and mask the real failure.
async fn handle_list_threads(
    ctx: &LocalRpcContext<'_>,
    params: &serde_json::Value,
) -> LocalRpcOutcome {
    let p: minos_protocol::ListThreadsParams = match serde_json::from_value(params.clone()) {
        Ok(v) => v,
        Err(e) => return err("bad_request", format!("invalid params: {e}")),
    };

    let Some(owner_id) = *ctx.session.paired_with.read().await else {
        return err("unauthorized", "list_threads requires a paired session");
    };
    let owner_s = Some(owner_id.to_string());

    let threads = match crate::store::threads::list(
        ctx.store,
        owner_s.as_deref(),
        p.agent,
        p.before_ts_ms,
        p.limit.min(500),
    )
    .await
    {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!(
                target: "minos_backend::local_rpc",
                error = ?e,
                "list_threads failed"
            );
            return err("internal", "list_threads failed");
        }
    };

    let next_before_ts_ms = threads.last().map(|t| t.last_ts_ms);
    let resp = minos_protocol::ListThreadsResponse {
        threads,
        next_before_ts_ms,
    };
    LocalRpcOutcome::Ok {
        result: serde_json::to_value(resp).unwrap_or_else(|_| serde_json::json!({})),
    }
}

/// Mobile → Backend. Read a window of translated UI events for one thread.
/// A fresh `CodexTranslatorState` is instantiated per call, so history
/// replays never share state with the live-ingest translator cache — this
/// guarantees deterministic output on repeated reads and protects the live
/// path from replay-induced mutation.
#[allow(clippy::too_many_lines)] // Single-site reader: ownership probe + read_range + translation + title/end-reason probes share a pagination cursor.
async fn handle_read_thread(
    ctx: &LocalRpcContext<'_>,
    params: &serde_json::Value,
) -> LocalRpcOutcome {
    let Some(owner_id) = *ctx.session.paired_with.read().await else {
        return err("unauthorized", "read_thread requires a paired session");
    };
    let p: minos_protocol::ReadThreadParams = match serde_json::from_value(params.clone()) {
        Ok(v) => v,
        Err(e) => return err("bad_request", format!("invalid params: {e}")),
    };

    let owner_device_id: Option<String> =
        match sqlx::query_scalar("SELECT owner_device_id FROM threads WHERE thread_id = ?1")
            .bind(&p.thread_id)
            .fetch_optional(ctx.store)
            .await
        {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!(
                    target: "minos_backend::local_rpc",
                    error = %e,
                    thread_id = %p.thread_id,
                    "read_thread.owner_probe failed"
                );
                return err("internal", "read_thread failed");
            }
        };
    let Some(owner_device_id) = owner_device_id else {
        return err(
            "thread_not_found",
            format!("thread not found: {}", p.thread_id),
        );
    };
    if owner_device_id != owner_id.to_string() {
        return err(
            "thread_not_found",
            format!("thread not found: {}", p.thread_id),
        );
    }

    let from_seq = p.from_seq.unwrap_or(0);
    let limit = p.limit.min(2000);
    let rows = match crate::store::raw_events::read_range(ctx.store, &p.thread_id, from_seq, limit)
        .await
    {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!(
                target: "minos_backend::local_rpc",
                error = ?e,
                thread_id = %p.thread_id,
                "read_thread.read_range failed"
            );
            return err("internal", "read_thread failed");
        }
    };

    // Fresh translator state per call so history stays deterministic.
    let mut state = minos_ui_protocol::CodexTranslatorState::new(p.thread_id.clone());
    let mut ui_events: Vec<minos_ui_protocol::UiEventMessage> = Vec::new();
    let mut last_seq_read = from_seq;
    for row in &rows {
        last_seq_read = u64::try_from(row.seq).unwrap_or(last_seq_read);
        match row.agent {
            minos_domain::AgentName::Codex => {
                match minos_ui_protocol::translate_codex(&mut state, &row.payload) {
                    Ok(v) => ui_events.extend(v),
                    Err(e) => ui_events.push(minos_ui_protocol::UiEventMessage::Error {
                        code: "translation_failed".into(),
                        message: format!("{e}"),
                        message_id: None,
                    }),
                }
            }
            minos_domain::AgentName::Claude => {
                match minos_ui_protocol::translate_claude(&row.payload) {
                    Ok(v) => ui_events.extend(v),
                    Err(e) => ui_events.push(minos_ui_protocol::UiEventMessage::Error {
                        code: "translation_failed".into(),
                        message: format!("{e}"),
                        message_id: None,
                    }),
                }
            }
            minos_domain::AgentName::Gemini => {
                match minos_ui_protocol::translate_gemini(&row.payload) {
                    Ok(v) => ui_events.extend(v),
                    Err(e) => ui_events.push(minos_ui_protocol::UiEventMessage::Error {
                        code: "translation_failed".into(),
                        message: format!("{e}"),
                        message_id: None,
                    }),
                }
            }
        }
    }

    if from_seq == 0
        && !ui_events.iter().any(|ui| {
            matches!(
                ui,
                minos_ui_protocol::UiEventMessage::ThreadTitleUpdated { .. }
            )
        })
    {
        let stored_title: Option<Option<String>> =
            match sqlx::query_scalar("SELECT title FROM threads WHERE thread_id = ?1")
                .bind(&p.thread_id)
                .fetch_optional(ctx.store)
                .await
            {
                Ok(v) => v,
                Err(e) => {
                    tracing::warn!(
                        target: "minos_backend::local_rpc",
                        error = %e,
                        thread_id = %p.thread_id,
                        "read_thread.title_probe failed"
                    );
                    return err("internal", "read_thread failed");
                }
            };
        if let Some(Some(title)) = stored_title {
            ui_events.insert(
                0,
                minos_ui_protocol::UiEventMessage::ThreadTitleUpdated {
                    thread_id: p.thread_id.clone(),
                    title,
                },
            );
        }
    }

    // Pagination cursor: if we filled the page, hand the caller a `next_seq`
    // to continue from. Otherwise the cursor is None (no more rows).
    let next_seq = if u32::try_from(rows.len()).unwrap_or(u32::MAX) == limit {
        Some(last_seq_read + 1)
    } else {
        None
    };

    // Look up end_reason (may be present even if rows are empty).
    let end_reason_json: Option<Option<String>> =
        match sqlx::query_scalar("SELECT end_reason FROM threads WHERE thread_id = ?1")
            .bind(&p.thread_id)
            .fetch_optional(ctx.store)
            .await
        {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!(
                    target: "minos_backend::local_rpc",
                    error = %e,
                    thread_id = %p.thread_id,
                    "read_thread.end_reason_probe failed"
                );
                return err("internal", "read_thread failed");
            }
        };
    let thread_end_reason = end_reason_json
        .flatten()
        .as_ref()
        .and_then(|s| serde_json::from_str::<minos_ui_protocol::ThreadEndReason>(s).ok());

    let resp = minos_protocol::ReadThreadResponse {
        ui_events,
        next_seq,
        thread_end_reason,
    };
    LocalRpcOutcome::Ok {
        result: serde_json::to_value(resp).unwrap_or_else(|_| serde_json::json!({})),
    }
}

/// Host helper. Returns the largest persisted `seq` for `thread_id`, or 0
/// if the thread has no ingested events yet.
async fn handle_get_thread_last_seq(
    ctx: &LocalRpcContext<'_>,
    params: &serde_json::Value,
) -> LocalRpcOutcome {
    if ctx.session.paired_with.read().await.is_none() {
        return err(
            "unauthorized",
            "get_thread_last_seq requires a paired session",
        );
    }
    let p: minos_protocol::GetThreadLastSeqParams = match serde_json::from_value(params.clone()) {
        Ok(v) => v,
        Err(e) => return err("bad_request", format!("invalid params: {e}")),
    };
    let last_seq = match crate::store::raw_events::last_seq(ctx.store, &p.thread_id).await {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!(
                target: "minos_backend::local_rpc",
                error = ?e,
                "get_thread_last_seq failed"
            );
            return err("internal", "get_thread_last_seq failed");
        }
    };
    LocalRpcOutcome::Ok {
        result: serde_json::to_value(minos_protocol::GetThreadLastSeqResponse { last_seq })
            .unwrap_or_else(|_| serde_json::json!({})),
    }
}

/// Build an error outcome with a machine-readable code + human message.
///
/// Pulled out so the set of valid codes is visible at a glance, and to
/// guarantee every `Err` goes through one funnel (easier to audit).
pub(crate) fn err(code: &str, message: impl Into<String>) -> LocalRpcOutcome {
    LocalRpcOutcome::Err {
        error: RpcError {
            code: code.to_string(),
            message: message.into(),
        },
    }
}

async fn compensate_pair_delivery_failure(
    ctx: &LocalRpcContext<'_>,
    issuer_handle: Option<&SessionHandle>,
) -> Result<(), BackendError> {
    if let Some(issuer_handle) = issuer_handle {
        *issuer_handle.paired_with.write().await = None;
    }
    *ctx.session.paired_with.write().await = None;

    match ctx.pairing.forget_pair(ctx.session.device_id).await? {
        Some(_) => Ok(()),
        None => Err(BackendError::StoreQuery {
            operation: "compensate_pair_delivery_failure".to_string(),
            message: "expected committed pair to exist during compensation".to_string(),
        }),
    }
}

async fn deliver_pair_to_current_issuer(
    ctx: &LocalRpcContext<'_>,
    issuer: minos_domain::DeviceId,
    issuer_handle: &SessionHandle,
    peer_name: &str,
    issuer_secret: &DeviceSecret,
) -> Result<(), LocalRpcOutcome> {
    let frame = Envelope::Event {
        version: 1,
        event: EventKind::Paired {
            peer_device_id: ctx.session.device_id,
            peer_name: peer_name.to_string(),
            your_device_secret: DeviceSecret(issuer_secret.as_str().to_string()),
        },
    };

    // ORDER MATTERS: mirror paired_with BEFORE pushing Event::Paired so the
    // issuer dispatcher cannot process a Forward with stale Unpaired state.
    // Delivery still only counts as success if the queue operation targets
    // the registry's current live session for this device.
    *issuer_handle.paired_with.write().await = Some(ctx.session.device_id);
    if let Err(e) = ctx.registry.try_send_current(issuer_handle, frame) {
        tracing::warn!(
            target: "minos_backend::envelope",
            error = %e,
            issuer = %issuer,
            consumer = %ctx.session.device_id,
            "failed to push Event::Paired to current issuer session; compensating committed pair"
        );
        if let Err(compensate_err) =
            compensate_pair_delivery_failure(ctx, Some(issuer_handle)).await
        {
            tracing::error!(
                target: "minos_backend::envelope",
                error = %compensate_err,
                issuer = %issuer,
                consumer = %ctx.session.device_id,
                "failed to compensate pair after issuer delivery failure"
            );
        }
        return Err(err(
            "internal",
            "failed to deliver pairing secret to issuer; pairing rolled back",
        ));
    }

    Ok(())
}

/// Always returns `{"ok": true}`; see spec §6.1.
///
/// `async` is kept for symmetry with the other handlers — the master
/// [`handle`] awaits them uniformly, and the trivial body lets this
/// compile to a zero-cost future.
#[allow(clippy::unused_async)]
async fn handle_ping() -> LocalRpcOutcome {
    LocalRpcOutcome::Ok {
        result: serde_json::json!({"ok": true}),
    }
}

/// `request_pairing_qr`: agent-host only; mints a fresh token and returns
/// a full `PairingQrPayload` the host renders into a QR code for the
/// mobile client to scan.
///
/// C4 replaces the earlier legacy `{token, expires_at}` body with the v2
/// QR payload that bundles:
///   - backend WebSocket URL (so mobile doesn't need DNS)
///   - host display name (echoed from the RPC params)
///   - one-shot pairing token + its expiry (unix epoch ms)
///   - optional legacy Cloudflare Access credentials
///
/// Spec §6.1 gates the caller's role. CF tokens are normally client-side
/// build/host env config; backend-held values are only kept for older QR
/// distribution flows.
async fn handle_request_pairing_token(
    ctx: &LocalRpcContext<'_>,
    params: &serde_json::Value,
) -> LocalRpcOutcome {
    if ctx.session.role != DeviceRole::AgentHost {
        return err("unauthorized", "only agent-host may request pairing tokens");
    }

    let p: minos_protocol::RequestPairingQrParams = match serde_json::from_value(params.clone()) {
        Ok(v) => v,
        Err(e) => return err("bad_request", format!("invalid params: {e}")),
    };

    let (token, expires_at) = match ctx
        .pairing
        .request_token(ctx.session.device_id, ctx.token_ttl)
        .await
    {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!(
                target: "minos_backend::envelope",
                error = %e,
                "request_pairing_qr token mint failed"
            );
            return err("internal", e.to_string());
        }
    };

    let qr_payload = minos_protocol::PairingQrPayload {
        v: 2,
        backend_url: ctx.public_url.to_string(),
        host_display_name: p.host_display_name,
        pairing_token: token.as_str().to_string(),
        expires_at_ms: expires_at.timestamp_millis(),
        cf_access_client_id: ctx.cf_access_client_id.map(str::to_owned),
        cf_access_client_secret: ctx.cf_access_client_secret.map(str::to_owned),
    };

    let resp = minos_protocol::RequestPairingQrResponse { qr_payload };
    LocalRpcOutcome::Ok {
        result: serde_json::to_value(resp).unwrap_or_else(|_| serde_json::json!({})),
    }
}

#[derive(Debug, Deserialize)]
struct PairParams {
    token: String,
    device_name: String,
}

/// `pair`: consume a token, mint two DeviceSecrets, notify the issuer.
///
/// Guards:
/// - reject if caller role is not `ios-client`.
/// - reject if this session already reports a peer (must go through
///   `forget_peer` first, spec §10.2 R4).
/// - reject invalid/expired/consumed tokens → `pairing_token_invalid`.
/// - reject already-paired peer → `pairing_state_mismatch`.
///
/// On success:
/// 1. update the consumer session's `paired_with` slot.
/// 2. look up the issuer's display name for the consumer's response.
/// 3. push `Event::Paired` to the issuer's outbox (if live) and update
///    its `paired_with` slot.
/// 4. return the consumer's `{peer_device_id, peer_name, your_device_secret}`
///    payload per spec §7.1 step 11.
#[allow(clippy::too_many_lines)] // Pair flow is a single spec-ordered sequence; splitting hides the rollback branches.
async fn handle_pair(ctx: &LocalRpcContext<'_>, params: &serde_json::Value) -> LocalRpcOutcome {
    if ctx.session.role != DeviceRole::IosClient {
        return err(
            "unauthorized",
            "only ios-client may pair with a pairing token",
        );
    }

    {
        // Read lock scoped so we don't hold it across the DB round-trip.
        if ctx.session.paired_with.read().await.is_some() {
            return err(
                "pairing_state_mismatch",
                "session is already paired; call forget_peer first",
            );
        }
    }

    let PairParams { token, device_name } =
        match serde_json::from_value::<PairParams>(params.clone()) {
            Ok(p) => p,
            Err(e) => {
                return err("bad_request", format!("pair params: {e}"));
            }
        };

    let candidate = minos_domain::PairingToken(token);

    let outcome = match ctx
        .pairing
        .consume_token(&candidate, ctx.session.device_id, device_name.clone())
        .await
    {
        Ok(o) => o,
        Err(BackendError::PairingTokenInvalid) => {
            return err(
                "pairing_token_invalid",
                "pairing token is unknown, expired, or already consumed",
            );
        }
        Err(BackendError::PairingStateMismatch { actual }) => {
            let message = if actual == "self" {
                "device cannot pair with itself".to_string()
            } else {
                format!("peer already paired (state: {actual})")
            };
            return err("pairing_state_mismatch", message);
        }
        Err(e) => {
            tracing::warn!(
                target: "minos_backend::envelope",
                error = %e,
                "pair consume_token failed"
            );
            return err("internal", e.to_string());
        }
    };

    let issuer = outcome.issuer_device_id;

    let Some(issuer_handle) = ctx.registry.get(issuer) else {
        tracing::warn!(
            target: "minos_backend::envelope",
            issuer = %issuer,
            consumer = %ctx.session.device_id,
            "pair committed but issuer is offline; compensating committed pair"
        );
        if let Err(compensate_err) = compensate_pair_delivery_failure(ctx, None).await {
            tracing::error!(
                target: "minos_backend::envelope",
                error = %compensate_err,
                issuer = %issuer,
                consumer = %ctx.session.device_id,
                "failed to compensate pair after issuer delivery failure"
            );
        }
        return err(
            "internal",
            "failed to deliver pairing secret to issuer; pairing rolled back",
        );
    };

    if let Err(outcome) = deliver_pair_to_current_issuer(
        ctx,
        issuer,
        &issuer_handle,
        &device_name,
        &outcome.issuer_secret,
    )
    .await
    {
        return outcome;
    }

    // Update this session's pairing state only after the issuer has accepted
    // the paired event, so compensation can keep the caller unpaired.
    *ctx.session.paired_with.write().await = Some(issuer);

    // Fetch issuer's display name for the consumer-side response payload.
    // Falls back to "Mac" if the row is somehow missing (shouldn't happen
    // post-consume_token, but defensive).
    let mac_name = match devices::get_device(ctx.store, issuer).await {
        Ok(Some(row)) => row.display_name,
        Ok(None) => {
            tracing::warn!(
                target: "minos_backend::envelope",
                issuer = %issuer,
                "pair succeeded but issuer device row missing"
            );
            "Mac".to_string()
        }
        Err(e) => {
            tracing::warn!(
                target: "minos_backend::envelope",
                error = %e,
                issuer = %issuer,
                "pair: lookup of issuer display_name failed"
            );
            "Mac".to_string()
        }
    };

    LocalRpcOutcome::Ok {
        result: serde_json::json!({
            "peer_device_id": issuer,
            "peer_name": mac_name,
            "your_device_secret": outcome.consumer_secret.as_str(),
        }),
    }
}

/// `forget_peer`: tear down the pairing, notify both sides.
///
/// Guards: reject if unpaired (nothing to forget). On success, emit
/// `Event::Unpaired` to this session's outbox and the peer's (if live),
/// and clear both `paired_with` slots.
async fn handle_forget_peer(ctx: &LocalRpcContext<'_>) -> LocalRpcOutcome {
    // Snapshot pairing state under a short read lock so we don't hold it
    // across the DB round-trip.
    let peer = {
        let guard = ctx.session.paired_with.read().await;
        *guard
    };
    let Some(peer) = peer else {
        return err(
            "pairing_state_mismatch",
            "session is not paired; nothing to forget",
        );
    };

    match ctx.pairing.forget_pair(ctx.session.device_id).await {
        Ok(Some(_)) => {}
        Ok(None) => {
            tracing::warn!(
                target: "minos_backend::envelope",
                device = %ctx.session.device_id,
                peer = %peer,
                "forget_peer cache/store mismatch: pair row missing during forget"
            );
            return err("internal", "pairing state missing in store");
        }
        Err(e) => {
            tracing::warn!(
                target: "minos_backend::envelope",
                error = %e,
                "forget_pair failed"
            );
            return err("internal", e.to_string());
        }
    }

    // Clear our side first so a concurrent route() sees no peer.
    *ctx.session.paired_with.write().await = None;

    // Push Event::Unpaired to both sides. Own side may already have its
    // UI wired via the LocalRpcResponse we're about to return, but the
    // wire contract (spec §7.4) says both sides receive the event — so
    // push it explicitly and let the client decide how to reconcile.
    //
    // Routing through `try_send_current` keeps a superseded socket from
    // eating the frame during a reconnect race: a stale outbox stays open
    // until the writer task tears down, so a raw `try_send` can succeed
    // against it while the live replacement misses the event.
    let unpaired = Envelope::Event {
        version: 1,
        event: EventKind::Unpaired,
    };
    if let Err(e) = ctx.registry.try_send_current(ctx.session, unpaired.clone()) {
        tracing::warn!(
            target: "minos_backend::envelope",
            error = ?e,
            device = %ctx.session.device_id,
            "failed to push Event::Unpaired to self"
        );
    }

    if let Some(peer_handle) = ctx.registry.get(peer) {
        // ORDER MATTERS: sever paired_with BEFORE pushing Event::Unpaired so
        // the peer dispatcher cannot route one last Forward off stale state.
        *peer_handle.paired_with.write().await = None;

        if let Err(e) = ctx.registry.try_send_current(&peer_handle, unpaired) {
            tracing::warn!(
                target: "minos_backend::envelope",
                error = ?e,
                peer = %peer,
                "failed to push Event::Unpaired to peer"
            );
        }
    }

    LocalRpcOutcome::Ok {
        result: serde_json::json!({"ok": true}),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::{devices, pairings, raw_events, test_support::memory_pool, threads};
    use minos_domain::{AgentName, DeviceId, DeviceRole};
    use pretty_assertions::assert_eq;

    // ── small helpers ─────────────────────────────────────────────────

    /// Insert a devices row and return its id; mimics the handshake path
    /// (step 9) that creates the row on WS upgrade.
    async fn insert_device_row(pool: &SqlitePool, name: &str, role: DeviceRole) -> DeviceId {
        let id = DeviceId::new();
        devices::insert_device(pool, id, name, role, 0)
            .await
            .unwrap();
        id
    }

    /// Build a [`SessionHandle`] + receiver for the given id/role. Mirrors
    /// what step 9's WS accept will do; scoped to tests.
    fn make_session(
        id: DeviceId,
        role: DeviceRole,
    ) -> (SessionHandle, tokio::sync::mpsc::Receiver<Envelope>) {
        SessionHandle::new(id, role)
    }

    fn make_ctx<'a>(
        session: &'a SessionHandle,
        registry: &'a SessionRegistry,
        pairing: &'a PairingService,
        pool: &'a SqlitePool,
    ) -> LocalRpcContext<'a> {
        LocalRpcContext {
            session,
            registry,
            pairing,
            store: pool,
            token_ttl: Duration::from_mins(5),
            public_url: "ws://127.0.0.1:8787/devices",
            cf_access_client_id: None,
            cf_access_client_secret: None,
        }
    }

    async fn seed_thread(
        pool: &SqlitePool,
        owner: DeviceId,
        thread_id: &str,
        agent: AgentName,
        payload: &serde_json::Value,
    ) {
        threads::upsert(pool, thread_id, agent, &owner.to_string(), 1)
            .await
            .unwrap();
        raw_events::insert_if_absent(pool, thread_id, 1, agent, payload, 1)
            .await
            .unwrap();
    }

    // ── ping ──────────────────────────────────────────────────────────

    #[tokio::test]
    async fn handle_ping_returns_ok_ok_true() {
        let pool = memory_pool().await;
        let registry = SessionRegistry::new();
        let pairing = PairingService::new(pool.clone());
        let mac = insert_device_row(&pool, "mac", DeviceRole::AgentHost).await;
        let (session, _rx) = make_session(mac, DeviceRole::AgentHost);
        let ctx = make_ctx(&session, &registry, &pairing, &pool);

        let out = handle(&ctx, &LocalRpcMethod::Ping, &serde_json::json!({})).await;
        match out {
            LocalRpcOutcome::Ok { result } => assert_eq!(result, serde_json::json!({"ok": true})),
            other => panic!("expected Ok, got {other:?}"),
        }
    }

    // ── request_pairing_token ─────────────────────────────────────────

    #[tokio::test]
    async fn request_pairing_token_rejects_ios_client() {
        let pool = memory_pool().await;
        let registry = SessionRegistry::new();
        let pairing = PairingService::new(pool.clone());
        let ios = insert_device_row(&pool, "iphone", DeviceRole::IosClient).await;
        let (session, _rx) = make_session(ios, DeviceRole::IosClient);
        let ctx = make_ctx(&session, &registry, &pairing, &pool);

        let out = handle(
            &ctx,
            &LocalRpcMethod::RequestPairingQr,
            &serde_json::json!({}),
        )
        .await;
        match out {
            LocalRpcOutcome::Err { error } => {
                assert_eq!(error.code, "unauthorized");
            }
            other => panic!("expected Err, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn request_pairing_qr_happy_path_returns_full_qr_payload() {
        let pool = memory_pool().await;
        let registry = SessionRegistry::new();
        let pairing = PairingService::new(pool.clone());
        let mac = insert_device_row(&pool, "mac", DeviceRole::AgentHost).await;
        let (session, _rx) = make_session(mac, DeviceRole::AgentHost);
        let ctx = make_ctx(&session, &registry, &pairing, &pool);

        let out = handle(
            &ctx,
            &LocalRpcMethod::RequestPairingQr,
            &serde_json::json!({"host_display_name": "Fan's Mac"}),
        )
        .await;
        match out {
            LocalRpcOutcome::Ok { result } => {
                let qr = &result["qr_payload"];
                assert_eq!(qr["v"], 2);
                assert_eq!(qr["backend_url"], "ws://127.0.0.1:8787/devices");
                assert_eq!(qr["host_display_name"], "Fan's Mac");
                let tok = qr["pairing_token"].as_str().expect("pairing_token string");
                assert!(tok.len() >= 32, "token too short: {tok:?}");
                assert!(qr["expires_at_ms"].is_i64());
                // CF tokens omitted in the default (local dev) context.
                assert!(qr.get("cf_access_client_id").is_none());
                assert!(qr.get("cf_access_client_secret").is_none());
            }
            other => panic!("expected Ok, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn request_pairing_qr_embeds_cf_tokens_when_configured() {
        let pool = memory_pool().await;
        let registry = SessionRegistry::new();
        let pairing = PairingService::new(pool.clone());
        let mac = insert_device_row(&pool, "mac", DeviceRole::AgentHost).await;
        let (session, _rx) = make_session(mac, DeviceRole::AgentHost);
        // Manual ctx override with CF tokens set.
        let ctx = LocalRpcContext {
            session: &session,
            registry: &registry,
            pairing: &pairing,
            store: &pool,
            token_ttl: Duration::from_mins(5),
            public_url: "wss://tunnel.example.com/devices",
            cf_access_client_id: Some("client-id.access"),
            cf_access_client_secret: Some("super-secret"),
        };

        let out = handle(
            &ctx,
            &LocalRpcMethod::RequestPairingQr,
            &serde_json::json!({"host_display_name": "prod"}),
        )
        .await;
        let LocalRpcOutcome::Ok { result } = out else {
            panic!("expected Ok, got {out:?}");
        };
        let qr = &result["qr_payload"];
        assert_eq!(qr["backend_url"], "wss://tunnel.example.com/devices");
        assert_eq!(qr["cf_access_client_id"], "client-id.access");
        assert_eq!(qr["cf_access_client_secret"], "super-secret");
    }

    #[tokio::test]
    async fn request_pairing_qr_rejects_missing_host_display_name() {
        let pool = memory_pool().await;
        let registry = SessionRegistry::new();
        let pairing = PairingService::new(pool.clone());
        let mac = insert_device_row(&pool, "mac", DeviceRole::AgentHost).await;
        let (session, _rx) = make_session(mac, DeviceRole::AgentHost);
        let ctx = make_ctx(&session, &registry, &pairing, &pool);

        let out = handle(
            &ctx,
            &LocalRpcMethod::RequestPairingQr,
            &serde_json::json!({}),
        )
        .await;
        match out {
            LocalRpcOutcome::Err { error } => {
                assert_eq!(error.code, "bad_request");
            }
            other => panic!("expected Err, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn read_thread_hides_threads_owned_by_a_different_paired_host() {
        let pool = memory_pool().await;
        let registry = SessionRegistry::new();
        let pairing = PairingService::new(pool.clone());

        let caller_peer = insert_device_row(&pool, "mac-a", DeviceRole::AgentHost).await;
        let other_peer = insert_device_row(&pool, "mac-b", DeviceRole::AgentHost).await;
        let ios = insert_device_row(&pool, "iphone", DeviceRole::IosClient).await;
        pairings::insert_pairing(&pool, caller_peer, ios, 0)
            .await
            .unwrap();

        let (session, _rx) = make_session(ios, DeviceRole::IosClient);
        *session.paired_with.write().await = Some(caller_peer);
        let ctx = make_ctx(&session, &registry, &pairing, &pool);

        seed_thread(
            &pool,
            other_peer,
            "thr-foreign",
            AgentName::Codex,
            &serde_json::json!({"method":"thread/started","params":{"threadId":"thr-foreign"}}),
        )
        .await;

        let out = handle(
            &ctx,
            &LocalRpcMethod::ReadThread,
            &serde_json::json!({"thread_id": "thr-foreign", "limit": 10}),
        )
        .await;
        match out {
            LocalRpcOutcome::Err { error } => {
                assert_eq!(error.code, "thread_not_found");
                assert_eq!(error.message, "thread not found: thr-foreign");
            }
            other => panic!("expected Err, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn read_thread_uses_translation_failed_for_unsupported_agent_history() {
        let pool = memory_pool().await;
        let registry = SessionRegistry::new();
        let pairing = PairingService::new(pool.clone());

        let host = insert_device_row(&pool, "mac", DeviceRole::AgentHost).await;
        let ios = insert_device_row(&pool, "iphone", DeviceRole::IosClient).await;
        pairings::insert_pairing(&pool, host, ios, 0).await.unwrap();

        let (session, _rx) = make_session(ios, DeviceRole::IosClient);
        *session.paired_with.write().await = Some(host);
        let ctx = make_ctx(&session, &registry, &pairing, &pool);

        seed_thread(
            &pool,
            host,
            "thr-claude",
            AgentName::Claude,
            &serde_json::json!({"method":"anything","params":{"value":1}}),
        )
        .await;

        let out = handle(
            &ctx,
            &LocalRpcMethod::ReadThread,
            &serde_json::json!({"thread_id": "thr-claude", "limit": 10}),
        )
        .await;
        let result = match out {
            LocalRpcOutcome::Ok { result } => result,
            other => panic!("expected Ok, got {other:?}"),
        };
        let read: minos_protocol::ReadThreadResponse = serde_json::from_value(result).unwrap();
        assert!(matches!(
            read.ui_events.as_slice(),
            [minos_ui_protocol::UiEventMessage::Error { code, message, message_id: None }]
                if code == "translation_failed"
                    && message == "translator not implemented for agent Claude"
        ));
    }

    #[tokio::test]
    async fn read_thread_surfaces_internal_error_when_title_decode_fails() {
        let pool = memory_pool().await;
        let registry = SessionRegistry::new();
        let pairing = PairingService::new(pool.clone());

        let host = insert_device_row(&pool, "mac", DeviceRole::AgentHost).await;
        let ios = insert_device_row(&pool, "iphone", DeviceRole::IosClient).await;
        pairings::insert_pairing(&pool, host, ios, 0).await.unwrap();

        let (session, _rx) = make_session(ios, DeviceRole::IosClient);
        *session.paired_with.write().await = Some(host);
        let ctx = make_ctx(&session, &registry, &pairing, &pool);

        seed_thread(
            &pool,
            host,
            "thr-bad-title",
            AgentName::Codex,
            &serde_json::json!({"method":"thread/started","params":{"threadId":"thr-bad-title"}}),
        )
        .await;

        // Force the title column to non-UTF-8 bytes via a CAST. SQLite stores
        // the bytes as TEXT but sqlx fails to decode them as a Rust String,
        // exercising the title-probe error branch in read_thread.
        sqlx::query("UPDATE threads SET title = CAST(X'C328' AS TEXT) WHERE thread_id = ?1")
            .bind("thr-bad-title")
            .execute(&pool)
            .await
            .unwrap();

        let out = handle(
            &ctx,
            &LocalRpcMethod::ReadThread,
            &serde_json::json!({"thread_id": "thr-bad-title", "limit": 10}),
        )
        .await;
        match out {
            LocalRpcOutcome::Err { error } => assert_eq!(error.code, "internal"),
            other => panic!("expected internal error from title decode failure, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn read_thread_first_page_includes_stored_title_when_history_has_no_title_event() {
        let pool = memory_pool().await;
        let registry = SessionRegistry::new();
        let pairing = PairingService::new(pool.clone());

        let host = insert_device_row(&pool, "mac", DeviceRole::AgentHost).await;
        let ios = insert_device_row(&pool, "iphone", DeviceRole::IosClient).await;
        pairings::insert_pairing(&pool, host, ios, 0).await.unwrap();

        let (session, _rx) = make_session(ios, DeviceRole::IosClient);
        *session.paired_with.write().await = Some(host);
        let ctx = make_ctx(&session, &registry, &pairing, &pool);

        seed_thread(
            &pool,
            host,
            "thr-title",
            AgentName::Codex,
            &serde_json::json!({
                "method": "item/started",
                "params": {
                    "itemId": "u1",
                    "role": "user",
                    "startedAtMs": 1,
                    "input": [{"type": "text", "text": "Explain the reconnect contract"}]
                }
            }),
        )
        .await;
        threads::update_title(&pool, "thr-title", "Explain the reconnect contract")
            .await
            .unwrap();

        let out = handle(
            &ctx,
            &LocalRpcMethod::ReadThread,
            &serde_json::json!({"thread_id": "thr-title", "limit": 10}),
        )
        .await;
        let result = match out {
            LocalRpcOutcome::Ok { result } => result,
            other => panic!("expected Ok, got {other:?}"),
        };
        let read: minos_protocol::ReadThreadResponse = serde_json::from_value(result).unwrap();

        assert!(matches!(
            read.ui_events.first(),
            Some(minos_ui_protocol::UiEventMessage::ThreadTitleUpdated { thread_id, title })
                if thread_id == "thr-title" && title == "Explain the reconnect contract"
        ));
    }

    // ── pair ──────────────────────────────────────────────────────────

    #[tokio::test]
    async fn pair_rejects_if_already_paired() {
        let pool = memory_pool().await;
        let registry = SessionRegistry::new();
        let pairing = PairingService::new(pool.clone());
        let ios = insert_device_row(&pool, "iphone", DeviceRole::IosClient).await;
        let (session, _rx) = make_session(ios, DeviceRole::IosClient);
        // Pre-seed pairing state: session already has a peer.
        *session.paired_with.write().await = Some(DeviceId::new());
        let ctx = make_ctx(&session, &registry, &pairing, &pool);

        let out = handle(
            &ctx,
            &LocalRpcMethod::Pair,
            &serde_json::json!({"token": "x", "device_name": "y"}),
        )
        .await;
        match out {
            LocalRpcOutcome::Err { error } => {
                assert_eq!(error.code, "pairing_state_mismatch");
            }
            other => panic!("expected Err, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn pair_rejects_agent_host_role_with_unauthorized() {
        let pool = memory_pool().await;
        let registry = SessionRegistry::new();
        let pairing = PairingService::new(pool.clone());
        let mac = insert_device_row(&pool, "mac", DeviceRole::AgentHost).await;
        let (session, _rx) = make_session(mac, DeviceRole::AgentHost);
        let ctx = make_ctx(&session, &registry, &pairing, &pool);

        let out = handle(
            &ctx,
            &LocalRpcMethod::Pair,
            &serde_json::json!({"token": "x", "device_name": "mac"}),
        )
        .await;
        match out {
            LocalRpcOutcome::Err { error } => {
                assert_eq!(error.code, "unauthorized");
                assert_eq!(
                    error.message,
                    "only ios-client may pair with a pairing token"
                );
            }
            other => panic!("expected Err, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn pair_rejects_on_invalid_token() {
        let pool = memory_pool().await;
        let registry = SessionRegistry::new();
        let pairing = PairingService::new(pool.clone());
        let ios = insert_device_row(&pool, "iphone", DeviceRole::IosClient).await;
        let (session, _rx) = make_session(ios, DeviceRole::IosClient);
        let ctx = make_ctx(&session, &registry, &pairing, &pool);

        let out = handle(
            &ctx,
            &LocalRpcMethod::Pair,
            &serde_json::json!({"token": "never-issued", "device_name": "iphone"}),
        )
        .await;
        match out {
            LocalRpcOutcome::Err { error } => {
                assert_eq!(error.code, "pairing_token_invalid");
            }
            other => panic!("expected Err, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn pair_rejects_self_pair_with_state_mismatch() {
        let pool = memory_pool().await;
        let registry = SessionRegistry::new();
        let pairing = PairingService::new(pool.clone());
        let ios = insert_device_row(&pool, "iphone", DeviceRole::IosClient).await;
        let (token, _) = pairing
            .request_token(ios, Duration::from_mins(5))
            .await
            .unwrap();
        let (session, _rx) = make_session(ios, DeviceRole::IosClient);
        let ctx = make_ctx(&session, &registry, &pairing, &pool);

        let out = handle(
            &ctx,
            &LocalRpcMethod::Pair,
            &serde_json::json!({"token": token.as_str(), "device_name": "iphone"}),
        )
        .await;
        match out {
            LocalRpcOutcome::Err { error } => {
                assert_eq!(error.code, "pairing_state_mismatch");
                assert_eq!(error.message, "device cannot pair with itself");
            }
            other => panic!("expected Err, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn pair_happy_path_returns_peer_info_and_pushes_event_to_issuer() {
        let pool = memory_pool().await;
        let registry = SessionRegistry::new();
        let pairing = PairingService::new(pool.clone());

        // Mac issuer: registered + live session in the registry.
        let mac = insert_device_row(&pool, "Fan's Mac", DeviceRole::AgentHost).await;
        let (mac_handle, mut mac_rx) = make_session(mac, DeviceRole::AgentHost);
        registry.insert(mac_handle.clone());

        // iOS consumer: separate session.
        let ios = DeviceId::new();
        let (ios_handle, _ios_rx) = make_session(ios, DeviceRole::IosClient);
        registry.insert(ios_handle.clone());

        // Mac issues a token (via the service directly; bypasses the
        // role-gated handler for test brevity).
        let (token, _exp) = pairing
            .request_token(mac, Duration::from_mins(5))
            .await
            .unwrap();

        let ctx = make_ctx(&ios_handle, &registry, &pairing, &pool);

        let out = handle(
            &ctx,
            &LocalRpcMethod::Pair,
            &serde_json::json!({"token": token.as_str(), "device_name": "Fan's iPhone"}),
        )
        .await;

        match out {
            LocalRpcOutcome::Ok { result } => {
                assert_eq!(result["peer_device_id"], serde_json::json!(mac));
                assert_eq!(result["peer_name"], "Fan's Mac");
                assert!(result["your_device_secret"].is_string());
            }
            other => panic!("expected Ok, got {other:?}"),
        }

        // iOS session now reflects the pairing.
        assert_eq!(*ios_handle.paired_with.read().await, Some(mac));
        // Mac session likewise. We assert this BEFORE draining the outbox
        // below: the mirror must be in place before Event::Paired becomes
        // observable, otherwise the issuer dispatcher could drain the event
        // and process a subsequent Forward while its own paired_with is
        // still None (spurious peer_offline error to the client).
        assert_eq!(
            *mac_handle.paired_with.read().await,
            Some(ios),
            "issuer paired_with must be mirrored before Event::Paired is observable"
        );

        // Mac's outbox received Event::Paired carrying issuer secret.
        let frame = mac_rx.recv().await.expect("Mac must receive Event::Paired");
        match frame {
            Envelope::Event { version, event } => {
                assert_eq!(version, 1);
                match event {
                    EventKind::Paired {
                        peer_device_id,
                        peer_name,
                        your_device_secret,
                    } => {
                        assert_eq!(peer_device_id, ios);
                        assert_eq!(peer_name, "Fan's iPhone");
                        assert!(!your_device_secret.as_str().is_empty());
                    }
                    other => panic!("expected Paired, got {other:?}"),
                }
            }
            other => panic!("expected Event envelope, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn pair_compensates_if_issuer_is_unavailable() {
        let pool = memory_pool().await;
        let registry = SessionRegistry::new();
        let pairing = PairingService::new(pool.clone());

        let mac = insert_device_row(&pool, "Fan's Mac", DeviceRole::AgentHost).await;
        let (token, _) = pairing
            .request_token(mac, Duration::from_mins(5))
            .await
            .unwrap();

        let ios = DeviceId::new();
        let (ios_handle, _ios_rx) = make_session(ios, DeviceRole::IosClient);
        let ctx = make_ctx(&ios_handle, &registry, &pairing, &pool);

        let out = handle(
            &ctx,
            &LocalRpcMethod::Pair,
            &serde_json::json!({"token": token.as_str(), "device_name": "Fan's iPhone"}),
        )
        .await;

        match out {
            LocalRpcOutcome::Err { error } => {
                assert_eq!(error.code, "internal");
                assert_eq!(
                    error.message,
                    "failed to deliver pairing secret to issuer; pairing rolled back"
                );
            }
            other => panic!("expected Err, got {other:?}"),
        }

        assert_eq!(*ios_handle.paired_with.read().await, None);
        assert_eq!(pairings::get_pair(&pool, mac).await.unwrap(), None);
        assert_eq!(pairings::get_pair(&pool, ios).await.unwrap(), None);
        assert_eq!(devices::get_secret_hash(&pool, mac).await.unwrap(), None);
        assert_eq!(devices::get_secret_hash(&pool, ios).await.unwrap(), None);
    }

    #[tokio::test]
    async fn pair_compensates_if_issuer_outbox_is_full() {
        let pool = memory_pool().await;
        let registry = SessionRegistry::new();
        let pairing = PairingService::new(pool.clone());

        let mac = insert_device_row(&pool, "Fan's Mac", DeviceRole::AgentHost).await;
        let (mac_handle, _mac_rx) = make_session(mac, DeviceRole::AgentHost);
        registry.insert(mac_handle.clone());

        for _ in 0..256 {
            mac_handle
                .outbox
                .try_send(Envelope::Event {
                    version: 1,
                    event: EventKind::Unpaired,
                })
                .expect("pre-filling the issuer outbox must succeed");
        }

        let (token, _) = pairing
            .request_token(mac, Duration::from_mins(5))
            .await
            .unwrap();

        let ios = DeviceId::new();
        let (ios_handle, _ios_rx) = make_session(ios, DeviceRole::IosClient);
        let ctx = make_ctx(&ios_handle, &registry, &pairing, &pool);

        let out = handle(
            &ctx,
            &LocalRpcMethod::Pair,
            &serde_json::json!({"token": token.as_str(), "device_name": "Fan's iPhone"}),
        )
        .await;

        match out {
            LocalRpcOutcome::Err { error } => {
                assert_eq!(error.code, "internal");
                assert_eq!(
                    error.message,
                    "failed to deliver pairing secret to issuer; pairing rolled back"
                );
            }
            other => panic!("expected Err, got {other:?}"),
        }

        assert_eq!(*mac_handle.paired_with.read().await, None);
        assert_eq!(*ios_handle.paired_with.read().await, None);
        assert_eq!(pairings::get_pair(&pool, mac).await.unwrap(), None);
        assert_eq!(pairings::get_pair(&pool, ios).await.unwrap(), None);
        assert_eq!(devices::get_secret_hash(&pool, mac).await.unwrap(), None);
        assert_eq!(devices::get_secret_hash(&pool, ios).await.unwrap(), None);
    }

    #[tokio::test]
    async fn pair_delivery_compensates_if_issuer_session_was_superseded() {
        let pool = memory_pool().await;
        let registry = SessionRegistry::new();
        let pairing = PairingService::new(pool.clone());

        let mac = insert_device_row(&pool, "Fan's Mac", DeviceRole::AgentHost).await;
        let (stale_handle, mut stale_rx) = make_session(mac, DeviceRole::AgentHost);
        registry.insert(stale_handle.clone());

        let token = pairing
            .request_token(mac, Duration::from_mins(5))
            .await
            .unwrap()
            .0;

        let ios = DeviceId::new();
        let (ios_handle, _ios_rx) = make_session(ios, DeviceRole::IosClient);
        let ctx = make_ctx(&ios_handle, &registry, &pairing, &pool);

        let outcome = pairing
            .consume_token(&token, ios, "Fan's iPhone".to_string())
            .await
            .unwrap();

        let (replacement_handle, mut replacement_rx) = make_session(mac, DeviceRole::AgentHost);
        registry.insert(replacement_handle.clone());

        let out = deliver_pair_to_current_issuer(
            &ctx,
            mac,
            &stale_handle,
            "Fan's iPhone",
            &outcome.issuer_secret,
        )
        .await;

        match out {
            Err(LocalRpcOutcome::Err { error }) => {
                assert_eq!(error.code, "internal");
                assert_eq!(
                    error.message,
                    "failed to deliver pairing secret to issuer; pairing rolled back"
                );
            }
            other => panic!("expected rollback Err, got {other:?}"),
        }

        assert!(matches!(
            stale_rx.try_recv(),
            Err(tokio::sync::mpsc::error::TryRecvError::Empty)
        ));
        assert!(matches!(
            replacement_rx.try_recv(),
            Err(tokio::sync::mpsc::error::TryRecvError::Empty)
        ));
        assert_eq!(*stale_handle.paired_with.read().await, None);
        assert_eq!(*replacement_handle.paired_with.read().await, None);
        assert_eq!(*ios_handle.paired_with.read().await, None);
        assert_eq!(pairings::get_pair(&pool, mac).await.unwrap(), None);
        assert_eq!(pairings::get_pair(&pool, ios).await.unwrap(), None);
        assert_eq!(devices::get_secret_hash(&pool, mac).await.unwrap(), None);
        assert_eq!(devices::get_secret_hash(&pool, ios).await.unwrap(), None);
    }

    // ── forget_peer ───────────────────────────────────────────────────

    #[tokio::test]
    async fn forget_peer_rejects_if_unpaired() {
        let pool = memory_pool().await;
        let registry = SessionRegistry::new();
        let pairing = PairingService::new(pool.clone());
        let ios = insert_device_row(&pool, "iphone", DeviceRole::IosClient).await;
        let (session, _rx) = make_session(ios, DeviceRole::IosClient);
        let ctx = make_ctx(&session, &registry, &pairing, &pool);

        let out = handle(&ctx, &LocalRpcMethod::ForgetPeer, &serde_json::json!({})).await;
        match out {
            LocalRpcOutcome::Err { error } => {
                assert_eq!(error.code, "pairing_state_mismatch");
            }
            other => panic!("expected Err, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn forget_peer_severs_peer_state_before_peer_event_is_observable() {
        let pool = memory_pool().await;
        let registry = SessionRegistry::new();
        let pairing = PairingService::new(pool.clone());

        let mac = insert_device_row(&pool, "mac", DeviceRole::AgentHost).await;
        let (mac_handle, mut mac_rx) = make_session(mac, DeviceRole::AgentHost);
        registry.insert(mac_handle.clone());

        let ios = DeviceId::new();
        let (ios_handle, mut ios_rx) = make_session(ios, DeviceRole::IosClient);
        registry.insert(ios_handle.clone());

        let (token, _) = pairing
            .request_token(mac, Duration::from_mins(5))
            .await
            .unwrap();

        {
            let ctx = make_ctx(&ios_handle, &registry, &pairing, &pool);
            let _ = handle(
                &ctx,
                &LocalRpcMethod::Pair,
                &serde_json::json!({"token": token.as_str(), "device_name": "iphone"}),
            )
            .await;
            let _paired = mac_rx.recv().await;
        }

        let ctx = make_ctx(&mac_handle, &registry, &pairing, &pool);
        let out = handle(&ctx, &LocalRpcMethod::ForgetPeer, &serde_json::json!({})).await;
        match out {
            LocalRpcOutcome::Ok { result } => {
                assert_eq!(result, serde_json::json!({"ok": true}));
            }
            other => panic!("expected Ok, got {other:?}"),
        }

        assert_eq!(
            *ios_handle.paired_with.read().await,
            None,
            "peer paired_with must be cleared before Event::Unpaired is observable"
        );

        let peer_event = ios_rx.recv().await.expect("iPhone must get Unpaired");
        match peer_event {
            Envelope::Event {
                event: EventKind::Unpaired,
                ..
            } => {}
            other => panic!("expected Unpaired, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn forget_peer_pushes_unpaired_event_to_both_sides() {
        let pool = memory_pool().await;
        let registry = SessionRegistry::new();
        let pairing = PairingService::new(pool.clone());

        // Fully wire a pair: Mac issues token, iPhone consumes it.
        let mac = insert_device_row(&pool, "mac", DeviceRole::AgentHost).await;
        let (mac_handle, mut mac_rx) = make_session(mac, DeviceRole::AgentHost);
        registry.insert(mac_handle.clone());

        let ios = DeviceId::new();
        let (ios_handle, mut ios_rx) = make_session(ios, DeviceRole::IosClient);
        registry.insert(ios_handle.clone());

        let (token, _) = pairing
            .request_token(mac, Duration::from_mins(5))
            .await
            .unwrap();

        // Run the pair handler so both `paired_with` slots and the DB are
        // in the post-pair state. Drain the Mac's Event::Paired from the
        // inbox so the subsequent Unpaired is the next event.
        {
            let ctx = make_ctx(&ios_handle, &registry, &pairing, &pool);
            let _ = handle(
                &ctx,
                &LocalRpcMethod::Pair,
                &serde_json::json!({"token": token.as_str(), "device_name": "iphone"}),
            )
            .await;
            // Drop the Event::Paired so the assertion below sees Unpaired
            // first.
            let _paired = mac_rx.recv().await;
        }

        // Now call forget_peer from the Mac's side.
        let ctx = make_ctx(&mac_handle, &registry, &pairing, &pool);
        let out = handle(&ctx, &LocalRpcMethod::ForgetPeer, &serde_json::json!({})).await;
        match out {
            LocalRpcOutcome::Ok { result } => {
                assert_eq!(result, serde_json::json!({"ok": true}));
            }
            other => panic!("expected Ok, got {other:?}"),
        }

        // Both paired_with slots cleared.
        assert_eq!(*mac_handle.paired_with.read().await, None);
        assert_eq!(*ios_handle.paired_with.read().await, None);

        // Mac (the caller) received Event::Unpaired.
        let self_event = mac_rx.recv().await.expect("Mac must get Unpaired");
        match self_event {
            Envelope::Event {
                event: EventKind::Unpaired,
                ..
            } => {}
            other => panic!("expected Unpaired, got {other:?}"),
        }
        // iOS peer also got Event::Unpaired.
        let peer_event = ios_rx.recv().await.expect("iPhone must get Unpaired");
        match peer_event {
            Envelope::Event {
                event: EventKind::Unpaired,
                ..
            } => {}
            other => panic!("expected Unpaired, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn forget_peer_does_not_push_unpaired_to_self_when_session_is_superseded() {
        let pool = memory_pool().await;
        let registry = SessionRegistry::new();
        let pairing = PairingService::new(pool.clone());

        let mac = insert_device_row(&pool, "mac", DeviceRole::AgentHost).await;
        let (mac_v1, mut mac_v1_rx) = make_session(mac, DeviceRole::AgentHost);
        registry.insert(mac_v1.clone());

        let ios = DeviceId::new();
        let (ios_handle, mut ios_rx) = make_session(ios, DeviceRole::IosClient);
        registry.insert(ios_handle.clone());

        let (token, _) = pairing
            .request_token(mac, Duration::from_mins(5))
            .await
            .unwrap();

        {
            let ctx = make_ctx(&ios_handle, &registry, &pairing, &pool);
            let _ = handle(
                &ctx,
                &LocalRpcMethod::Pair,
                &serde_json::json!({"token": token.as_str(), "device_name": "iphone"}),
            )
            .await;
            // Drain the issuer's Event::Paired so we observe forget_peer's
            // delivery decisions next.
            let _paired = mac_v1_rx.recv().await;
        }

        // A reconnect supersedes mac_v1 in the registry. mac_v1 stays "paired"
        // in memory because forget_peer is racing this replacement, but the
        // live session is now mac_v2.
        let (mac_v2, mut mac_v2_rx) = make_session(mac, DeviceRole::AgentHost);
        *mac_v2.paired_with.write().await = Some(ios);
        registry.insert(mac_v2.clone());

        // Run forget_peer from the stale mac_v1 session (the dispatcher
        // handed it off before the reconnect won the registry race).
        let ctx = make_ctx(&mac_v1, &registry, &pairing, &pool);
        let out = handle(&ctx, &LocalRpcMethod::ForgetPeer, &serde_json::json!({})).await;
        match out {
            LocalRpcOutcome::Ok { result } => assert_eq!(result, serde_json::json!({"ok": true})),
            other => panic!("expected Ok, got {other:?}"),
        }

        // The stale socket must not consume Event::Unpaired — it's no longer
        // the live device session, so the frame would be observed by the
        // dying writer task instead of the replacement.
        assert!(
            matches!(
                mac_v1_rx.try_recv(),
                Err(tokio::sync::mpsc::error::TryRecvError::Empty)
            ),
            "superseded self handle must not receive Event::Unpaired"
        );
        assert!(
            matches!(
                mac_v2_rx.try_recv(),
                Err(tokio::sync::mpsc::error::TryRecvError::Empty)
            ),
            "replacement self handle is not the forget_peer caller and must not receive Unpaired",
        );

        // Peer side still uses the registry's current ios entry, which was
        // never replaced; it must observe Event::Unpaired exactly once.
        let peer_event = ios_rx.recv().await.expect("iPhone must get Unpaired");
        match peer_event {
            Envelope::Event {
                event: EventKind::Unpaired,
                ..
            } => {}
            other => panic!("expected Unpaired, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn forget_peer_revokes_device_secret_hashes() {
        let pool = memory_pool().await;
        let registry = SessionRegistry::new();
        let pairing = PairingService::new(pool.clone());

        let mac = insert_device_row(&pool, "mac", DeviceRole::AgentHost).await;
        let (mac_handle, _mac_rx) = make_session(mac, DeviceRole::AgentHost);
        registry.insert(mac_handle.clone());

        let ios = DeviceId::new();
        let (ios_handle, _ios_rx) = make_session(ios, DeviceRole::IosClient);
        registry.insert(ios_handle.clone());

        let (token, _) = pairing
            .request_token(mac, Duration::from_mins(5))
            .await
            .unwrap();

        {
            let ctx = make_ctx(&ios_handle, &registry, &pairing, &pool);
            let out = handle(
                &ctx,
                &LocalRpcMethod::Pair,
                &serde_json::json!({"token": token.as_str(), "device_name": "iphone"}),
            )
            .await;
            match out {
                LocalRpcOutcome::Ok { .. } => {}
                other => panic!("expected Ok, got {other:?}"),
            }
        }

        assert!(devices::get_secret_hash(&pool, mac)
            .await
            .unwrap()
            .is_some());
        assert!(devices::get_secret_hash(&pool, ios)
            .await
            .unwrap()
            .is_some());

        let ctx = make_ctx(&mac_handle, &registry, &pairing, &pool);
        let out = handle(&ctx, &LocalRpcMethod::ForgetPeer, &serde_json::json!({})).await;
        match out {
            LocalRpcOutcome::Ok { result } => {
                assert_eq!(result, serde_json::json!({"ok": true}));
            }
            other => panic!("expected Ok, got {other:?}"),
        }

        assert_eq!(pairings::get_pair(&pool, mac).await.unwrap(), None);
        assert_eq!(pairings::get_pair(&pool, ios).await.unwrap(), None);
        assert_eq!(devices::get_secret_hash(&pool, mac).await.unwrap(), None);
        assert_eq!(devices::get_secret_hash(&pool, ios).await.unwrap(), None);
        assert_eq!(*mac_handle.paired_with.read().await, None);
        assert_eq!(*ios_handle.paired_with.read().await, None);
    }
}
