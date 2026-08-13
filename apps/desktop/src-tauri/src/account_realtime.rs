//! Account IM `/ws/client` owned by the Desktop Rust host.
//!
//! Native Bearer access JWT — no ticket. Frames are forwarded to the webview
//! as `account://frame` / `account://state` events.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use minos_protocol::realtime::{ClientFrame, ServerFrame};
use serde::Serialize;
use tauri::{AppHandle, Emitter};
use tokio::sync::{mpsc, Mutex};
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::header::AUTHORIZATION;
use tokio_tungstenite::tungstenite::Message;
use tracing::{info, warn};

use crate::account_cloud;

const EVENT_FRAME: &str = "account://frame";
const EVENT_STATE: &str = "account://state";

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountSyncState {
    pub state: String,
}

pub struct AccountRealtime {
    inner: Mutex<Inner>,
    epoch: AtomicU64,
}

struct Inner {
    stop_tx: Option<mpsc::Sender<()>>,
    out_tx: Option<mpsc::Sender<ClientFrame>>,
    access_token: String,
    account_id: String,
    conversation_ids: Vec<String>,
}

impl AccountRealtime {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(Inner {
                stop_tx: None,
                out_tx: None,
                access_token: String::new(),
                account_id: String::new(),
                conversation_ids: Vec::new(),
            }),
            epoch: AtomicU64::new(0),
        }
    }

    pub async fn start(
        &self,
        app: AppHandle,
        access_token: String,
        account_id: String,
    ) -> anyhow::Result<()> {
        self.stop().await;
        let (stop_tx, stop_rx) = mpsc::channel(1);
        let (out_tx, out_rx) = mpsc::channel::<ClientFrame>(64);
        {
            let mut inner = self.inner.lock().await;
            inner.stop_tx = Some(stop_tx);
            inner.out_tx = Some(out_tx);
            inner.access_token = access_token.clone();
            inner.account_id = account_id.clone();
        }
        let epoch = self.epoch.fetch_add(1, Ordering::SeqCst) + 1;
        let conversations = { self.inner.lock().await.conversation_ids.clone() };
        tokio::spawn(run_loop(
            app,
            epoch,
            Arc::new(AtomicU64::new(epoch)),
            access_token,
            account_id,
            conversations,
            out_rx,
            stop_rx,
        ));
        Ok(())
    }

    pub async fn update_auth(&self, access_token: String, account_id: String) {
        let mut inner = self.inner.lock().await;
        inner.access_token = access_token;
        inner.account_id = account_id;
    }

    pub async fn stop(&self) {
        self.epoch.fetch_add(1, Ordering::SeqCst);
        let mut inner = self.inner.lock().await;
        if let Some(tx) = inner.stop_tx.take() {
            let _ = tx.send(()).await;
        }
        inner.out_tx = None;
    }

    pub async fn subscribe_conversation(&self, conversation_id: String) {
        let mut inner = self.inner.lock().await;
        if !inner
            .conversation_ids
            .iter()
            .any(|id| id == &conversation_id)
        {
            inner.conversation_ids.push(conversation_id.clone());
        }
        if let Some(tx) = inner.out_tx.as_ref() {
            let mut resume = HashMap::new();
            if let Some(seq) = load_cursor(&format!("conversation:{conversation_id}")) {
                resume.insert(format!("conversation:{conversation_id}"), seq);
            }
            let _ = tx
                .send(ClientFrame::Subscribe {
                    topics: vec![format!("conversation:{conversation_id}")],
                    resume_after: if resume.is_empty() {
                        None
                    } else {
                        Some(resume)
                    },
                    client_request_id: None,
                })
                .await;
        }
    }

    pub async fn send(&self, frame: ClientFrame) -> anyhow::Result<()> {
        let inner = self.inner.lock().await;
        let tx = inner
            .out_tx
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("account ws not connected"))?;
        tx.send(frame)
            .await
            .map_err(|_| anyhow::anyhow!("account ws send failed"))?;
        Ok(())
    }
}

fn cursor_path() -> std::path::PathBuf {
    let home = dirs::home_dir().unwrap_or_else(|| std::path::PathBuf::from("."));
    home.join(".minos/desktop/topic-cursors.json")
}

fn load_cursors() -> HashMap<String, i64> {
    let path = cursor_path();
    std::fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn load_cursor(topic: &str) -> Option<i64> {
    load_cursors().get(topic).copied()
}

fn save_cursor(topic: &str, seq: i64) {
    let mut map = load_cursors();
    let prev = map.get(topic).copied().unwrap_or(0);
    if seq <= prev {
        return;
    }
    map.insert(topic.to_string(), seq);
    let path = cursor_path();
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Ok(buf) = serde_json::to_vec_pretty(&map) {
        let _ = std::fs::write(path, buf);
    }
}

pub fn clear_cursors() {
    let _ = std::fs::remove_file(cursor_path());
}

#[allow(clippy::too_many_arguments)]
async fn run_loop(
    app: AppHandle,
    epoch: u64,
    live_epoch: Arc<AtomicU64>,
    access_token: String,
    account_id: String,
    conversation_ids: Vec<String>,
    mut out_rx: mpsc::Receiver<ClientFrame>,
    mut stop_rx: mpsc::Receiver<()>,
) {
    let mut attempt: u32 = 0;
    loop {
        if live_epoch.load(Ordering::SeqCst) != epoch {
            return;
        }
        emit_state(&app, "connecting");
        match connect_once(
            &app,
            &access_token,
            &account_id,
            &conversation_ids,
            &mut out_rx,
            &mut stop_rx,
        )
        .await
        {
            LoopEnd::Stop => {
                emit_state(&app, "disconnected");
                return;
            }
            LoopEnd::Retry => {
                emit_state(&app, "disconnected");
                attempt = attempt.saturating_add(1);
                let delay = Duration::from_secs(u64::from(1u32 << attempt.min(5)).min(30));
                tokio::select! {
                    _ = stop_rx.recv() => {
                        emit_state(&app, "disconnected");
                        return;
                    }
                    _ = tokio::time::sleep(delay) => {}
                }
            }
        }
    }
}

enum LoopEnd {
    Stop,
    Retry,
}

async fn connect_once(
    app: &AppHandle,
    access_token: &str,
    account_id: &str,
    conversation_ids: &[String],
    out_rx: &mut mpsc::Receiver<ClientFrame>,
    stop_rx: &mut mpsc::Receiver<()>,
) -> LoopEnd {
    let url = account_cloud::client_ws_url();
    let mut req = match url.as_str().into_client_request() {
        Ok(req) => req,
        Err(error) => {
            warn!(target: "minos_desktop::account_ws", %error, "invalid client ws url");
            return LoopEnd::Retry;
        }
    };
    let bearer = format!("Bearer {access_token}");
    match bearer.parse() {
        Ok(value) => {
            req.headers_mut().insert(AUTHORIZATION, value);
        }
        Err(error) => {
            warn!(target: "minos_desktop::account_ws", %error, "invalid authorization header");
            return LoopEnd::Retry;
        }
    }

    let (stream, _) = match tokio_tungstenite::connect_async(req).await {
        Ok(pair) => pair,
        Err(error) => {
            warn!(target: "minos_desktop::account_ws", %error, "account ws connect failed");
            return LoopEnd::Retry;
        }
    };
    info!(target: "minos_desktop::account_ws", "account ws connected");
    let (mut sink, mut stream) = stream.split();

    loop {
        tokio::select! {
            _ = stop_rx.recv() => return LoopEnd::Stop,
            frame = out_rx.recv() => {
                let Some(frame) = frame else { return LoopEnd::Stop };
                let Ok(text) = serde_json::to_string(&frame) else { continue };
                if sink.send(Message::Text(text.into())).await.is_err() {
                    return LoopEnd::Retry;
                }
            }
            incoming = stream.next() => {
                match incoming {
                    Some(Ok(Message::Text(text))) => {
                        handle_server_text(app, account_id, conversation_ids, &mut sink, &text).await;
                    }
                    Some(Ok(Message::Ping(p))) => {
                        let _ = sink.send(Message::Pong(p)).await;
                    }
                    Some(Ok(Message::Close(_))) | None => return LoopEnd::Retry,
                    Some(Err(_)) => return LoopEnd::Retry,
                    _ => {}
                }
            }
        }
    }
}

async fn handle_server_text(
    app: &AppHandle,
    account_id: &str,
    conversation_ids: &[String],
    sink: &mut (impl SinkExt<Message> + Unpin),
    text: &str,
) {
    let _ = app.emit(EVENT_FRAME, text);
    let Ok(frame) = serde_json::from_str::<ServerFrame>(text) else {
        return;
    };
    match frame {
        ServerFrame::Hello { .. } => {
            emit_state(app, "syncing");
            let mut topics = vec![format!("account:{account_id}")];
            for id in conversation_ids {
                topics.push(format!("conversation:{id}"));
            }
            let mut resume = HashMap::new();
            for topic in &topics {
                if let Some(seq) = load_cursor(topic) {
                    resume.insert(topic.clone(), seq);
                }
            }
            let sub = ClientFrame::Subscribe {
                topics,
                resume_after: if resume.is_empty() {
                    None
                } else {
                    Some(resume)
                },
                client_request_id: None,
            };
            if let Ok(json) = serde_json::to_string(&sub) {
                let _ = sink.send(Message::Text(json.into())).await;
            }
        }
        ServerFrame::SubscribeAck { .. } => {
            emit_state(app, "live");
        }
        ServerFrame::DurableEvent {
            topic, topic_seq, ..
        } => {
            save_cursor(&topic, topic_seq);
        }
        _ => {}
    }
}

fn emit_state(app: &AppHandle, state: &str) {
    let _ = app.emit(
        EVENT_STATE,
        AccountSyncState {
            state: state.to_string(),
        },
    );
}
