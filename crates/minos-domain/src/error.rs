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
    Unauthorized,
    ConnectionStateMismatch,
    EnvelopeVersionUnsupported,
    PeerOffline,
    RelayInternal,
    CfAuthFailed,
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
    ///   6. the frb mirror enums `_MinosError` and `_ErrorKind` in
    ///      `crates/minos-ffi-frb/src/api/minos.rs` (re-run `cargo xtask gen-frb`)
    #[must_use]
    pub fn user_message(self, lang: Lang) -> &'static str {
        match (self, lang) {
            (Self::BindFailed, Lang::Zh) => "无法绑定中继监听地址；请检查 MINOS_RELAY_LISTEN 配置",
            (Self::BindFailed, Lang::En) => {
                "Cannot bind relay listen address; check MINOS_RELAY_LISTEN"
            }
            (Self::ConnectFailed, Lang::Zh) => {
                "无法连接中继服务；请检查网络与 Cloudflare Access 令牌"
            }
            (Self::ConnectFailed, Lang::En) => {
                "Cannot reach relay; check network and Cloudflare Access token"
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
            (Self::Unauthorized, Lang::Zh) => "操作未授权，请确认登录状态",
            (Self::Unauthorized, Lang::En) => "Unauthorized for this operation",
            (Self::ConnectionStateMismatch, Lang::Zh) => "连接状态不符合要求，请稍后重试",
            (Self::ConnectionStateMismatch, Lang::En) => {
                "Connection state mismatch; please retry later"
            }
            (Self::EnvelopeVersionUnsupported, Lang::Zh) => "协议版本不兼容，请升级应用",
            (Self::EnvelopeVersionUnsupported, Lang::En) => {
                "Protocol version unsupported; please update the app"
            }
            (Self::PeerOffline, Lang::Zh) => "对端设备离线，请检查配对设备",
            (Self::PeerOffline, Lang::En) => "Paired device offline; please check status",
            (Self::RelayInternal, Lang::Zh) => "中继服务异常，请稍后重试",
            (Self::RelayInternal, Lang::En) => "Relay service error; please retry later",
            (Self::CfAuthFailed, Lang::Zh) => "Cloudflare Access 认证失败，请检查 Service Token",
            (Self::CfAuthFailed, Lang::En) => {
                "Cloudflare Access authentication failed; please check the Service Token"
            }
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

    // ── relay layer (spec §10.1) ──
    #[error("unauthorized for this operation: {reason}")]
    Unauthorized { reason: String },

    #[error("relay connection state not suitable: expected {expected}, got {actual}")]
    ConnectionStateMismatch { expected: String, actual: String },

    #[error("envelope version unsupported: {version}")]
    EnvelopeVersionUnsupported { version: u8 },

    #[error("peer offline: {peer_device_id}")]
    PeerOffline { peer_device_id: String },

    #[error("relay internal error: {message}")]
    RelayInternal { message: String },

    #[error("cloudflare access authentication failed: {message}")]
    CfAuthFailed { message: String },

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
            Self::Unauthorized { .. } => ErrorKind::Unauthorized,
            Self::ConnectionStateMismatch { .. } => ErrorKind::ConnectionStateMismatch,
            Self::EnvelopeVersionUnsupported { .. } => ErrorKind::EnvelopeVersionUnsupported,
            Self::PeerOffline { .. } => ErrorKind::PeerOffline,
            Self::RelayInternal { .. } => ErrorKind::RelayInternal,
            Self::CfAuthFailed { .. } => ErrorKind::CfAuthFailed,
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
                MinosError::Unauthorized {
                    reason: String::new(),
                },
                ErrorKind::Unauthorized,
            ),
            (
                MinosError::ConnectionStateMismatch {
                    expected: String::new(),
                    actual: String::new(),
                },
                ErrorKind::ConnectionStateMismatch,
            ),
            (
                MinosError::EnvelopeVersionUnsupported { version: 0 },
                ErrorKind::EnvelopeVersionUnsupported,
            ),
            (
                MinosError::PeerOffline {
                    peer_device_id: String::new(),
                },
                ErrorKind::PeerOffline,
            ),
            (
                MinosError::RelayInternal {
                    message: String::new(),
                },
                ErrorKind::RelayInternal,
            ),
            (
                MinosError::CfAuthFailed {
                    message: String::new(),
                },
                ErrorKind::CfAuthFailed,
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
            24,
            "add a case when you add a MinosError variant"
        );
        for (err, expected_kind) in cases {
            assert_eq!(err.kind(), expected_kind, "{err:?} → wrong kind");
        }
    }

    /// Central list of every `ErrorKind`, used by the two exhaustive tests
    /// below. Adding a new kind requires adding it here as well.
    const ALL_KINDS: &[ErrorKind] = &[
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
        ErrorKind::Unauthorized,
        ErrorKind::ConnectionStateMismatch,
        ErrorKind::EnvelopeVersionUnsupported,
        ErrorKind::PeerOffline,
        ErrorKind::RelayInternal,
        ErrorKind::CfAuthFailed,
        ErrorKind::CodexSpawnFailed,
        ErrorKind::CodexConnectFailed,
        ErrorKind::CodexProtocolError,
        ErrorKind::AgentAlreadyRunning,
        ErrorKind::AgentNotRunning,
        ErrorKind::AgentNotSupported,
        ErrorKind::AgentSessionIdMismatch,
    ];

    #[test]
    fn every_error_kind_has_user_message_in_both_langs() {
        assert_eq!(
            ALL_KINDS.len(),
            24,
            "add a kind when you add an ErrorKind variant"
        );
        for k in ALL_KINDS {
            assert!(!k.user_message(Lang::Zh).is_empty(), "missing zh for {k:?}");
            assert!(!k.user_message(Lang::En).is_empty(), "missing en for {k:?}");
        }
    }

    #[test]
    fn no_tailscale_strings_remain_in_user_messages() {
        // Relay rollout (spec §10.1) removes Tailscale from the user-facing
        // copy; a regression would reintroduce it in a translation edit.
        // Guard both the relay-specific kinds called out in the spec AND
        // every other kind so future edits can't leak these words back in.
        let banned = ["tailscale", "tailnet"];
        for k in ALL_KINDS {
            for lang in [Lang::Zh, Lang::En] {
                let msg = k.user_message(lang);
                let lower = msg.to_lowercase();
                for word in banned {
                    assert!(
                        !lower.contains(word),
                        "banned word {word:?} leaked into user_message for {k:?} / {lang:?}: {msg}"
                    );
                }
            }
        }
    }

    #[test]
    fn cf_auth_failed_display_and_kind() {
        let err = MinosError::CfAuthFailed {
            message: "Cloudflare denied".into(),
        };
        assert_eq!(err.kind(), ErrorKind::CfAuthFailed);
        let s = err.to_string();
        assert!(s.contains("cloudflare"));
        assert!(s.contains("Cloudflare denied"));
    }

    #[test]
    fn cf_auth_failed_user_message_zh_no_tailscale_wording() {
        let m = ErrorKind::CfAuthFailed.user_message(Lang::Zh);
        assert!(m.contains("Cloudflare"));
        assert!(!m.to_lowercase().contains("tailscale"));
    }

    #[test]
    fn relay_error_variants_user_messages_match_spec() {
        // Spot-check the new relay copy so edits to the translation table
        // show up as failing asserts rather than silent drift.
        assert_eq!(
            ErrorKind::BindFailed.user_message(Lang::En),
            "Cannot bind relay listen address; check MINOS_RELAY_LISTEN"
        );
        assert_eq!(
            ErrorKind::ConnectFailed.user_message(Lang::En),
            "Cannot reach relay; check network and Cloudflare Access token"
        );
        assert_eq!(
            ErrorKind::Unauthorized.user_message(Lang::En),
            "Unauthorized for this operation"
        );
        assert_eq!(
            ErrorKind::EnvelopeVersionUnsupported.user_message(Lang::En),
            "Protocol version unsupported; please update the app"
        );
        assert_eq!(
            ErrorKind::PeerOffline.user_message(Lang::En),
            "Paired device offline; please check status"
        );
        assert_eq!(
            ErrorKind::RelayInternal.user_message(Lang::En),
            "Relay service error; please retry later"
        );
        assert_eq!(
            ErrorKind::ConnectionStateMismatch.user_message(Lang::En),
            "Connection state mismatch; please retry later"
        );
    }
}
