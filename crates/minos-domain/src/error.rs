//! Single typed error for all Minos public APIs.
//!
//! Variants mirror the table in spec §7.4. `Lang` + `user_message` produce
//! short, user-facing copy (zh / en) so UI layers do not need to translate
//! by themselves. The `ErrorKind` companion enum mirrors `MinosError`'s
//! discriminants without payload and carries the single-source-of-truth
//! localization table — UniFFI consumers call `kind_message(kind, lang)`
//! because `#[uniffi::Error]` variants cannot be passed as arguments.

use crate::PairingState;

#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    Zh,
    En,
}

/// Payload-free discriminant of `MinosError`. Mirrored 1:1 with `MinosError`
/// variants (excluding carried data). UniFFI exposes this + `user_message`
/// as the cross-language localization bridge.
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorKind {
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
    CodexSpawnFailed,
    CodexConnectFailed,
    CodexProtocolError,
    AgentAlreadyRunning,
    AgentNotRunning,
    AgentNotSupported,
    AgentSessionIdMismatch,
}

impl ErrorKind {
    /// Single source of truth for user-facing zh/en strings. Adding a new
    /// `MinosError` variant requires adding:
    ///   1. the new `MinosError` variant itself
    ///   2. the matching `ErrorKind` variant
    ///   3. one arm in `MinosError::kind`
    ///   4. two arms (zh + en) here
    ///   5. one arm in Swift's `MinosError.kind` extension
    #[must_use]
    pub fn user_message(self, lang: Lang) -> &'static str {
        match (self, lang) {
            (Self::BindFailed, Lang::Zh) => "无法绑定本机端口；请检查 Tailscale 是否已启动并登录",
            (Self::BindFailed, Lang::En) => {
                "Cannot bind local port; please verify Tailscale is running and signed in"
            }
            (Self::ConnectFailed, Lang::Zh) => {
                "无法连接 Mac；请确认两端均已加入同一 Tailscale 网络"
            }
            (Self::ConnectFailed, Lang::En) => {
                "Cannot reach Mac; ensure both devices are on the same Tailscale network"
            }
            (Self::Disconnected, Lang::Zh) => "连接已断开，正在重试",
            (Self::Disconnected, Lang::En) => "Disconnected; reconnecting",
            (Self::PairingTokenInvalid, Lang::Zh) => "二维码已过期，请重新扫描",
            (Self::PairingTokenInvalid, Lang::En) => "QR code expired, please rescan",
            (Self::PairingStateMismatch, Lang::Zh) => "已存在配对设备，请确认替换",
            (Self::PairingStateMismatch, Lang::En) => {
                "A paired device already exists; confirm to replace"
            }
            (Self::DeviceNotTrusted, Lang::Zh) => "配对已失效，请重新扫码",
            (Self::DeviceNotTrusted, Lang::En) => "Pairing invalidated, please rescan",
            (Self::StoreIo, Lang::Zh) => "本地存储不可访问，请检查权限",
            (Self::StoreIo, Lang::En) => "Local storage inaccessible; check permissions",
            (Self::StoreCorrupt, Lang::Zh) => "本地配对状态损坏，已备份；请重新配对",
            (Self::StoreCorrupt, Lang::En) => {
                "Local pairing state corrupt; backed up. Please re-pair"
            }
            (Self::CliProbeTimeout, Lang::Zh) => "CLI 探测超时",
            (Self::CliProbeTimeout, Lang::En) => "CLI probe timed out",
            (Self::CliProbeFailed, Lang::Zh) => "CLI 探测失败",
            (Self::CliProbeFailed, Lang::En) => "CLI probe failed",
            (Self::RpcCallFailed, Lang::Zh) => "服务端错误，请稍后重试",
            (Self::RpcCallFailed, Lang::En) => "Server error, please retry",
            (Self::CodexSpawnFailed, Lang::Zh) => "无法启动 Codex CLI；请确认已安装 `codex`",
            (Self::CodexSpawnFailed, Lang::En) => "Failed to launch codex CLI; is codex installed?",
            (Self::CodexConnectFailed, Lang::Zh) => "无法连接 Codex 服务",
            (Self::CodexConnectFailed, Lang::En) => "Could not reach codex app-server",
            (Self::CodexProtocolError, Lang::Zh) => "Codex 返回错误，请查看日志",
            (Self::CodexProtocolError, Lang::En) => "Codex returned an error — see log",
            (Self::AgentAlreadyRunning, Lang::Zh) => "Agent 已在运行",
            (Self::AgentAlreadyRunning, Lang::En) => "An agent session is already running",
            (Self::AgentNotRunning, Lang::Zh) => "当前没有 Agent 会话",
            (Self::AgentNotRunning, Lang::En) => "No agent session is running",
            (Self::AgentNotSupported, Lang::Zh) => "这一期仅支持 Codex",
            (Self::AgentNotSupported, Lang::En) => "Only Codex is supported in this phase",
            (Self::AgentSessionIdMismatch, Lang::Zh) => "会话已失效，请重新启动",
            (Self::AgentSessionIdMismatch, Lang::En) => {
                "Session is no longer active; please restart"
            }
        }
    }
}

#[cfg_attr(feature = "uniffi", derive(uniffi::Error))]
#[derive(thiserror::Error, Debug)]
pub enum MinosError {
    // ── network / WS layer ──
    #[error("websocket bind failed on {addr}: {message}")]
    BindFailed { addr: String, message: String },

    #[error("websocket connect to {url} failed: {message}")]
    ConnectFailed { url: String, message: String },

    #[error("websocket disconnected: {reason}")]
    Disconnected { reason: String },

    // ── pairing layer ──
    #[error("pairing token invalid or expired")]
    PairingTokenInvalid,

    #[error("pairing not in expected state: {actual:?}")]
    PairingStateMismatch { actual: PairingState },

    #[error("device not trusted: {device_id}")]
    DeviceNotTrusted { device_id: String },

    // ── persistence layer ──
    #[error("store io failed at {path}: {message}")]
    StoreIo { path: String, message: String },

    #[error("store payload corrupt at {path}: {message}")]
    StoreCorrupt { path: String, message: String },

    // ── CLI probe layer ──
    #[error("cli probe timeout: {bin} after {timeout_ms}ms")]
    CliProbeTimeout { bin: String, timeout_ms: u64 },

    #[error("cli probe failed: {bin}: {message}")]
    CliProbeFailed { bin: String, message: String },

    // ── RPC layer ──
    #[error("rpc call failed: {method}: {message}")]
    RpcCallFailed { method: String, message: String },

    // ── agent runtime layer (spec §5.3) ──
    #[error("failed to spawn codex: {message}")]
    CodexSpawnFailed { message: String },

    #[error("failed to connect codex WS at {url}: {message}")]
    CodexConnectFailed { url: String, message: String },

    #[error("codex protocol error on {method}: {message}")]
    CodexProtocolError { method: String, message: String },

    #[error("agent is already running")]
    AgentAlreadyRunning,

    #[error("no agent session is running")]
    AgentNotRunning,

    #[error("agent {agent:?} not supported in this build")]
    AgentNotSupported { agent: crate::AgentName },

    #[error("session id does not match the active session")]
    AgentSessionIdMismatch,
}

impl MinosError {
    /// Payload-free discriminant — mirrors every variant 1:1.
    #[must_use]
    pub fn kind(&self) -> ErrorKind {
        match self {
            Self::BindFailed { .. } => ErrorKind::BindFailed,
            Self::ConnectFailed { .. } => ErrorKind::ConnectFailed,
            Self::Disconnected { .. } => ErrorKind::Disconnected,
            Self::PairingTokenInvalid => ErrorKind::PairingTokenInvalid,
            Self::PairingStateMismatch { .. } => ErrorKind::PairingStateMismatch,
            Self::DeviceNotTrusted { .. } => ErrorKind::DeviceNotTrusted,
            Self::StoreIo { .. } => ErrorKind::StoreIo,
            Self::StoreCorrupt { .. } => ErrorKind::StoreCorrupt,
            Self::CliProbeTimeout { .. } => ErrorKind::CliProbeTimeout,
            Self::CliProbeFailed { .. } => ErrorKind::CliProbeFailed,
            Self::RpcCallFailed { .. } => ErrorKind::RpcCallFailed,
            Self::CodexSpawnFailed { .. } => ErrorKind::CodexSpawnFailed,
            Self::CodexConnectFailed { .. } => ErrorKind::CodexConnectFailed,
            Self::CodexProtocolError { .. } => ErrorKind::CodexProtocolError,
            Self::AgentAlreadyRunning => ErrorKind::AgentAlreadyRunning,
            Self::AgentNotRunning => ErrorKind::AgentNotRunning,
            Self::AgentNotSupported { .. } => ErrorKind::AgentNotSupported,
            Self::AgentSessionIdMismatch => ErrorKind::AgentSessionIdMismatch,
        }
    }

    /// Short, user-facing string. Delegates to `ErrorKind::user_message` so
    /// the table lives in exactly one place.
    #[must_use]
    pub fn user_message(&self, lang: Lang) -> &'static str {
        self.kind().user_message(lang)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_includes_dynamic_context() {
        let e = MinosError::BindFailed {
            addr: "100.64.0.10:7878".into(),
            message: "address already in use".into(),
        };
        let s = format!("{e}");
        assert!(s.contains("100.64.0.10:7878"));
        assert!(s.contains("address already in use"));
    }

    #[test]
    fn user_message_is_static_per_variant_and_lang() {
        let e = MinosError::PairingTokenInvalid;
        assert_eq!(e.user_message(Lang::Zh), "二维码已过期，请重新扫描");
        assert_eq!(e.user_message(Lang::En), "QR code expired, please rescan");
    }

    #[test]
    #[allow(clippy::too_many_lines)] // fixture table grows with each new variant
    fn kind_exhaustively_matches_every_variant() {
        let cases: Vec<(MinosError, ErrorKind)> = vec![
            (
                MinosError::BindFailed {
                    addr: String::new(),
                    message: String::new(),
                },
                ErrorKind::BindFailed,
            ),
            (
                MinosError::ConnectFailed {
                    url: String::new(),
                    message: String::new(),
                },
                ErrorKind::ConnectFailed,
            ),
            (
                MinosError::Disconnected {
                    reason: String::new(),
                },
                ErrorKind::Disconnected,
            ),
            (
                MinosError::PairingTokenInvalid,
                ErrorKind::PairingTokenInvalid,
            ),
            (
                MinosError::PairingStateMismatch {
                    actual: PairingState::Paired,
                },
                ErrorKind::PairingStateMismatch,
            ),
            (
                MinosError::DeviceNotTrusted {
                    device_id: String::new(),
                },
                ErrorKind::DeviceNotTrusted,
            ),
            (
                MinosError::StoreIo {
                    path: String::new(),
                    message: String::new(),
                },
                ErrorKind::StoreIo,
            ),
            (
                MinosError::StoreCorrupt {
                    path: String::new(),
                    message: String::new(),
                },
                ErrorKind::StoreCorrupt,
            ),
            (
                MinosError::CliProbeTimeout {
                    bin: String::new(),
                    timeout_ms: 0,
                },
                ErrorKind::CliProbeTimeout,
            ),
            (
                MinosError::CliProbeFailed {
                    bin: String::new(),
                    message: String::new(),
                },
                ErrorKind::CliProbeFailed,
            ),
            (
                MinosError::RpcCallFailed {
                    method: String::new(),
                    message: String::new(),
                },
                ErrorKind::RpcCallFailed,
            ),
            (
                MinosError::CodexSpawnFailed {
                    message: String::new(),
                },
                ErrorKind::CodexSpawnFailed,
            ),
            (
                MinosError::CodexConnectFailed {
                    url: String::new(),
                    message: String::new(),
                },
                ErrorKind::CodexConnectFailed,
            ),
            (
                MinosError::CodexProtocolError {
                    method: String::new(),
                    message: String::new(),
                },
                ErrorKind::CodexProtocolError,
            ),
            (
                MinosError::AgentAlreadyRunning,
                ErrorKind::AgentAlreadyRunning,
            ),
            (MinosError::AgentNotRunning, ErrorKind::AgentNotRunning),
            (
                MinosError::AgentNotSupported {
                    agent: crate::AgentName::Codex,
                },
                ErrorKind::AgentNotSupported,
            ),
            (
                MinosError::AgentSessionIdMismatch,
                ErrorKind::AgentSessionIdMismatch,
            ),
        ];
        assert_eq!(
            cases.len(),
            18,
            "add a case when you add a MinosError variant"
        );
        for (err, expected_kind) in cases {
            assert_eq!(err.kind(), expected_kind, "{err:?} → wrong kind");
        }
    }

    #[test]
    fn every_error_kind_has_user_message_in_both_langs() {
        let kinds = [
            ErrorKind::BindFailed,
            ErrorKind::ConnectFailed,
            ErrorKind::Disconnected,
            ErrorKind::PairingTokenInvalid,
            ErrorKind::PairingStateMismatch,
            ErrorKind::DeviceNotTrusted,
            ErrorKind::StoreIo,
            ErrorKind::StoreCorrupt,
            ErrorKind::CliProbeTimeout,
            ErrorKind::CliProbeFailed,
            ErrorKind::RpcCallFailed,
            ErrorKind::CodexSpawnFailed,
            ErrorKind::CodexConnectFailed,
            ErrorKind::CodexProtocolError,
            ErrorKind::AgentAlreadyRunning,
            ErrorKind::AgentNotRunning,
            ErrorKind::AgentNotSupported,
            ErrorKind::AgentSessionIdMismatch,
        ];
        assert_eq!(
            kinds.len(),
            18,
            "add a kind when you add a MinosError variant"
        );
        for k in kinds {
            assert!(!k.user_message(Lang::Zh).is_empty(), "missing zh for {k:?}");
            assert!(!k.user_message(Lang::En).is_empty(), "missing en for {k:?}");
        }
    }
}
