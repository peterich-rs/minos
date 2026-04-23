//! Dart-visible frb surface over `minos_mobile::MobileClient`.
//!
//! This file is the entire frb input: `flutter_rust_bridge_codegen` walks this
//! module (and its siblings under `crate::api`) to emit Dart bindings and the
//! matching `wire_*` handlers in `crate::frb_generated`. Anything added here
//! becomes visible from Dart; internal helpers live outside `crate::api`.
//!
//! The opaque wrapper [`MobileClient`] holds the real
//! `minos_mobile::MobileClient` behind a `RustOpaque` handle — Dart never
//! marshals its fields, only invokes methods on it. Domain enums/structs are
//! mirrored (see the `#[frb(mirror(...))]` blocks below) so pattern-matching
//! works on the Dart side without duplicating the localization table.

use std::path::Path;
use std::sync::OnceLock;

use flutter_rust_bridge::frb;
use tokio::runtime::{Builder, Runtime};
use tokio::sync::watch;

// `StreamSink` is defined by the `frb_generated_boilerplate!` macro expanded
// inside `crate::frb_generated`, not at the flutter_rust_bridge crate root.
// We re-route the name through the generated module so unqualified
// `StreamSink<T>` resolves both pre- and post-codegen.
use crate::frb_generated::StreamSink;

// Re-exported `pub use` so `crate::api::minos::TypeName` resolves for the
// generated wire code in `frb_generated.rs`. Mirror declarations below still
// provide the shape metadata the codegen needs.
pub use minos_domain::{ConnectionState, ErrorKind, Lang, MinosError, PairingState};
pub use minos_protocol::PairResponse;

// ───────────────────────────── opaque client ─────────────────────────────

/// Opaque Dart handle around `minos_mobile::MobileClient`.
///
/// The inner type is not exposed to Dart — all interactions go through the
/// `impl` below. This keeps `Arc<dyn PairingStore>` (and any other non-FFI-safe
/// internals) Rust-side.
#[frb(opaque)]
pub struct MobileClient(minos_mobile::MobileClient);

fn frb_runtime() -> &'static Runtime {
    static FRB_RUNTIME: OnceLock<Runtime> = OnceLock::new();
    FRB_RUNTIME.get_or_init(|| {
        Builder::new_multi_thread()
            .enable_all()
            .thread_name("minos-frb")
            .build()
            .expect("failed to build minos-ffi-frb tokio runtime")
    })
}

fn spawn_state_forwarder<F>(mut rx: watch::Receiver<ConnectionState>, mut emit: F)
where
    F: FnMut(ConnectionState) -> Result<(), ()> + Send + 'static,
{
    frb_runtime().spawn(async move {
        // Emit the snapshot visible at subscribe time so late subscribers
        // aren't stuck on whatever they last rendered.
        if emit(*rx.borrow_and_update()).is_err() {
            return;
        }
        while rx.changed().await.is_ok() {
            if emit(*rx.borrow()).is_err() {
                break;
            }
        }
    });
}

impl MobileClient {
    /// Construct a client backed by the built-in in-memory pairing store.
    /// Synchronous — no I/O happens until a pairing method is called.
    #[frb(sync)]
    #[must_use]
    pub fn new(self_name: String) -> Self {
        Self(minos_mobile::MobileClient::new_with_in_memory_store(
            self_name,
        ))
    }

    /// Pair using the raw JSON payload extracted from the scanned QR code.
    /// Delegates to `MobileClient::pair_with_json`; see that method for the
    /// full error surface.
    pub async fn pair_with_json(&self, qr_json: String) -> Result<PairResponse, MinosError> {
        self.0.pair_with_json(qr_json).await
    }

    /// Current connection state, read from the watch-channel cache. Cheap and
    /// synchronous.
    #[frb(sync)]
    #[must_use]
    pub fn current_state(&self) -> ConnectionState {
        self.0.current_state()
    }

    /// Subscribe to connection-state transitions. Emits the current value
    /// immediately, then every subsequent change. The spawned task exits once
    /// the Dart side drops the stream (detected via `sink.add(...).is_err()`).
    pub fn subscribe_state(&self, sink: StreamSink<ConnectionState>) {
        spawn_state_forwarder(self.0.events_stream(), move |state| {
            sink.add(state).map_err(|_| ())
        });
    }
}

// ────────────────────────────── free functions ──────────────────────────────

/// Initialize mobile-side Rust logging with the given directory (supplied by
/// Dart, typically `<Documents>/Minos/Logs`). Idempotent — safe to call once
/// per launch.
pub fn init_logging(log_dir: String) -> Result<(), MinosError> {
    minos_mobile::logging::init(Path::new(&log_dir))
}

/// Localize an `ErrorKind` into user-facing copy. Mirrors the UniFFI adapter's
/// `kind_message` so Dart can render localized error strings without hard-
/// coding them.
#[frb(sync)]
#[must_use]
pub fn kind_message(kind: ErrorKind, lang: Lang) -> String {
    kind.user_message(lang).to_string()
}

// ─────────────────────────── mirrored domain types ───────────────────────────
//
// frb requires us to re-declare any foreign type we want to expose to Dart.
// The `#[frb(mirror(T))]` attribute tells the codegen "this declaration is the
// shape of `T` from `crate::domain`; emit Dart bindings that encode/decode
// the real `T`". The mirror declarations themselves are never instantiated;
// they exist purely as codegen hints.

#[allow(dead_code)]
#[frb(mirror(ConnectionState))]
pub enum _ConnectionState {
    Disconnected,
    Pairing,
    Connected,
    Reconnecting { attempt: u32 },
}

#[allow(dead_code)]
#[frb(mirror(PairingState))]
pub enum _PairingState {
    Unpaired,
    AwaitingPeer,
    Paired,
}

#[allow(dead_code)]
#[frb(mirror(Lang))]
pub enum _Lang {
    Zh,
    En,
}

#[allow(dead_code)]
#[frb(mirror(ErrorKind))]
pub enum _ErrorKind {
    BindFailed,
    ConnectFailed,
    Disconnected,
    PairingTokenInvalid,
    PairingStateMismatch,
    DeviceNotTrusted,
    StoreIo,
    StoreCorrupt,
    CliProbeTimeout,
    CliProbeFailed,
    RpcCallFailed,
}

#[allow(dead_code)]
#[frb(mirror(PairResponse))]
pub struct _PairResponse {
    pub ok: bool,
    pub mac_name: String,
}

#[allow(dead_code)]
#[frb(mirror(MinosError))]
pub enum _MinosError {
    BindFailed { addr: String, message: String },
    ConnectFailed { url: String, message: String },
    Disconnected { reason: String },
    PairingTokenInvalid,
    PairingStateMismatch { actual: PairingState },
    DeviceNotTrusted { device_id: String },
    StoreIo { path: String, message: String },
    StoreCorrupt { path: String, message: String },
    CliProbeTimeout { bin: String, timeout_ms: u64 },
    CliProbeFailed { bin: String, message: String },
    RpcCallFailed { method: String, message: String },
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;
    use std::time::Duration;

    use super::*;

    #[test]
    fn state_forwarder_spawns_without_current_runtime() {
        let (tx, rx) = watch::channel(ConnectionState::Disconnected);

        assert!(
            tokio::runtime::Handle::try_current().is_err(),
            "test must start outside a tokio runtime"
        );

        let (state_tx, state_rx) = mpsc::channel();
        spawn_state_forwarder(rx, move |state| state_tx.send(state).map_err(|_| ()));

        assert_eq!(
            state_rx.recv_timeout(Duration::from_millis(200)).unwrap(),
            ConnectionState::Disconnected
        );

        tx.send(ConnectionState::Pairing).unwrap();
        assert_eq!(
            state_rx.recv_timeout(Duration::from_millis(200)).unwrap(),
            ConnectionState::Pairing
        );
    }
}
