//! Single typed error for all Minos public APIs.
//!
//! Variants mirror the table in spec §7.4. `Lang` + `user_message` produce
//! short, user-facing copy (zh / en) so UI layers do not need to translate
//! by themselves.

use crate::PairingState;

#[derive(Debug, Clone, Copy)]
pub enum Lang {
    Zh,
    En,
}

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
}

impl MinosError {
    /// Short, user-facing string. Stable for UI binding; do not include
    /// dynamic field values here — that is what `Display` is for.
    #[must_use]
    pub fn user_message(&self, lang: Lang) -> &'static str {
        match (self, lang) {
            (Self::BindFailed { .. }, Lang::Zh) => {
                "无法绑定本机端口；请检查 Tailscale 是否已启动并登录"
            }
            (Self::BindFailed { .. }, Lang::En) => {
                "Cannot bind local port; please verify Tailscale is running and signed in"
            }
            (Self::ConnectFailed { .. }, Lang::Zh) => {
                "无法连接 Mac；请确认两端均已加入同一 Tailscale 网络"
            }
            (Self::ConnectFailed { .. }, Lang::En) => {
                "Cannot reach Mac; ensure both devices are on the same Tailscale network"
            }
            (Self::Disconnected { .. }, Lang::Zh) => "连接已断开，正在重试",
            (Self::Disconnected { .. }, Lang::En) => "Disconnected; reconnecting",
            (Self::PairingTokenInvalid, Lang::Zh) => "二维码已过期，请重新扫描",
            (Self::PairingTokenInvalid, Lang::En) => "QR code expired, please rescan",
            (Self::PairingStateMismatch { .. }, Lang::Zh) => "已存在配对设备，请确认替换",
            (Self::PairingStateMismatch { .. }, Lang::En) => {
                "A paired device already exists; confirm to replace"
            }
            (Self::DeviceNotTrusted { .. }, Lang::Zh) => "配对已失效，请重新扫码",
            (Self::DeviceNotTrusted { .. }, Lang::En) => "Pairing invalidated, please rescan",
            (Self::StoreIo { .. }, Lang::Zh) => "本地存储不可访问，请检查权限",
            (Self::StoreIo { .. }, Lang::En) => "Local storage inaccessible; check permissions",
            (Self::StoreCorrupt { .. }, Lang::Zh) => "本地配对状态损坏，已备份；请重新配对",
            (Self::StoreCorrupt { .. }, Lang::En) => {
                "Local pairing state corrupt; backed up. Please re-pair"
            }
            (Self::CliProbeTimeout { .. }, Lang::Zh) => "CLI 探测超时",
            (Self::CliProbeTimeout { .. }, Lang::En) => "CLI probe timed out",
            (Self::CliProbeFailed { .. }, Lang::Zh) => "CLI 探测失败",
            (Self::CliProbeFailed { .. }, Lang::En) => "CLI probe failed",
            (Self::RpcCallFailed { .. }, Lang::Zh) => "服务端错误，请稍后重试",
            (Self::RpcCallFailed { .. }, Lang::En) => "Server error, please retry",
        }
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
    fn every_variant_has_user_message_in_both_langs() {
        // Construct one of every variant; user_message must not panic for any.
        let variants = vec![
            MinosError::BindFailed {
                addr: String::new(),
                message: String::new(),
            },
            MinosError::ConnectFailed {
                url: String::new(),
                message: String::new(),
            },
            MinosError::Disconnected {
                reason: String::new(),
            },
            MinosError::PairingTokenInvalid,
            MinosError::PairingStateMismatch {
                actual: PairingState::Paired,
            },
            MinosError::DeviceNotTrusted {
                device_id: String::new(),
            },
            MinosError::StoreIo {
                path: String::new(),
                message: String::new(),
            },
            MinosError::StoreCorrupt {
                path: String::new(),
                message: String::new(),
            },
            MinosError::CliProbeTimeout {
                bin: String::new(),
                timeout_ms: 0,
            },
            MinosError::CliProbeFailed {
                bin: String::new(),
                message: String::new(),
            },
            MinosError::RpcCallFailed {
                method: String::new(),
                message: String::new(),
            },
        ];
        for v in variants {
            assert!(!v.user_message(Lang::Zh).is_empty(), "missing zh for {v:?}");
            assert!(!v.user_message(Lang::En).is_empty(), "missing en for {v:?}");
        }
    }
}
