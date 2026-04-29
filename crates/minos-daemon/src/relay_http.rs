//! HTTP client for the backend's `/v1/*` control plane.
//!
//! Built on `reqwest`. Stamps the same `X-Device-*` and CF-Access
//! headers as the WS client. Used by `RelayClient` to issue pairing
//! tokens and to forget pairings without going through the multiplexed
//! envelope.

use std::time::Duration;

use minos_domain::{DeviceId, DeviceSecret, MinosError};
use minos_protocol::{
    MePeerResponse, PairingQrPayload, RequestPairingQrParams, RequestPairingQrResponse,
};
use reqwest::Client;
use serde::Deserialize;

use crate::config::RelayConfig;

const HTTP_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, Deserialize)]
struct ErrorEnvelope {
    error: ErrorBody,
}

#[derive(Debug, Deserialize)]
struct ErrorBody {
    code: String,
    message: String,
}

pub struct RelayHttpClient {
    client: Client,
    base: String,
    device_id: DeviceId,
    device_role: &'static str,
    device_name: String,
    config: RelayConfig,
}

impl RelayHttpClient {
    pub fn new(
        backend_ws_url: &str,
        device_id: DeviceId,
        device_name: String,
        config: RelayConfig,
    ) -> Result<Self, MinosError> {
        let base = http_base(backend_ws_url).ok_or_else(|| MinosError::ConnectFailed {
            url: backend_ws_url.into(),
            message: "cannot derive HTTP base from backend URL".into(),
        })?;
        let client = Client::builder()
            .timeout(HTTP_TIMEOUT)
            .build()
            .map_err(|e| MinosError::BackendInternal {
                message: format!("reqwest build: {e}"),
            })?;
        Ok(Self {
            client,
            base,
            device_id,
            device_role: "agent-host",
            device_name,
            config,
        })
    }

    pub async fn request_pairing_qr(
        &self,
        host_display_name: String,
    ) -> Result<PairingQrPayload, MinosError> {
        let url = format!("{}/v1/pairing/tokens", self.base);
        let req = self
            .client
            .post(&url)
            .header("x-device-id", self.device_id.to_string())
            .header("x-device-role", self.device_role)
            .header("x-device-name", &self.device_name);
        let req = stamp_cf(req, &self.config);
        let resp = req
            .json(&RequestPairingQrParams { host_display_name })
            .send()
            .await
            .map_err(|e| connect_err(&url, &e))?;
        let status = resp.status();
        if status.is_success() {
            let body: RequestPairingQrResponse =
                resp.json().await.map_err(|e| MinosError::BackendInternal {
                    message: format!("decode RequestPairingQrResponse: {e}"),
                })?;
            Ok(body.qr_payload)
        } else {
            Err(decode_error(status, resp).await)
        }
    }

    pub async fn forget_pairing(&self, secret: &DeviceSecret) -> Result<(), MinosError> {
        let url = format!("{}/v1/pairing", self.base);
        let req = self
            .client
            .delete(&url)
            .header("x-device-id", self.device_id.to_string())
            .header("x-device-role", self.device_role)
            .header("x-device-secret", secret.as_str());
        let req = stamp_cf(req, &self.config);
        let resp = req.send().await.map_err(|e| connect_err(&url, &e))?;
        let status = resp.status();
        if status == reqwest::StatusCode::NO_CONTENT || status == reqwest::StatusCode::NOT_FOUND {
            // Idempotent: nothing to forget is fine.
            Ok(())
        } else {
            Err(decode_error(status, resp).await)
        }
    }

    /// Fetch the backend's view of our currently paired peer.
    ///
    /// Returns `Ok(Some(_))` on `200`, `Ok(None)` on `404 not_paired`
    /// (no row, or row exists but isn't paired), and `Err` for any other
    /// failure path. Used by the daemon's relay-client right after WS
    /// handshake to repopulate its in-memory peer mirror without
    /// persisting anything to disk; pairing facts (who, name, paired_at)
    /// live on the backend.
    pub async fn get_me_peer(
        &self,
        secret: &DeviceSecret,
    ) -> Result<Option<MePeerResponse>, MinosError> {
        let url = format!("{}/v1/me/peer", self.base);
        let req = self
            .client
            .get(&url)
            .header("x-device-id", self.device_id.to_string())
            .header("x-device-role", self.device_role)
            .header("x-device-secret", secret.as_str());
        let req = stamp_cf(req, &self.config);
        let resp = req.send().await.map_err(|e| connect_err(&url, &e))?;
        let status = resp.status();
        if status.is_success() {
            let body: MePeerResponse =
                resp.json().await.map_err(|e| MinosError::BackendInternal {
                    message: format!("decode MePeerResponse: {e}"),
                })?;
            return Ok(Some(body));
        }
        if status == reqwest::StatusCode::NOT_FOUND {
            // 404 with `error.code == "not_paired"` is the structured
            // "no peer" signal. Anything else under 404 is unexpected
            // (e.g. an unknown route) — surface as an error so the
            // caller logs it.
            let body: Result<ErrorEnvelope, _> = resp.json().await;
            if let Ok(env) = body {
                if env.error.code == "not_paired" {
                    return Ok(None);
                }
                return Err(MinosError::BackendInternal {
                    message: format!("backend 404 ({}): {}", env.error.code, env.error.message),
                });
            }
            return Err(MinosError::BackendInternal {
                message: format!("backend {status}"),
            });
        }
        Err(decode_error(status, resp).await)
    }
}

fn stamp_cf(req: reqwest::RequestBuilder, cfg: &RelayConfig) -> reqwest::RequestBuilder {
    let mut req = req;
    if !cfg.cf_client_id.is_empty() {
        req = req.header("cf-access-client-id", cfg.cf_client_id.as_str());
    }
    if !cfg.cf_client_secret.is_empty() {
        req = req.header("cf-access-client-secret", cfg.cf_client_secret.as_str());
    }
    req
}

fn connect_err(url: &str, e: &reqwest::Error) -> MinosError {
    if e.status() == Some(reqwest::StatusCode::UNAUTHORIZED) {
        MinosError::CfAuthFailed {
            message: format!("{url}: {e}"),
        }
    } else {
        MinosError::ConnectFailed {
            url: url.into(),
            message: e.to_string(),
        }
    }
}

async fn decode_error(status: reqwest::StatusCode, resp: reqwest::Response) -> MinosError {
    let body: Result<ErrorEnvelope, _> = resp.json().await;
    match body {
        Ok(env) => MinosError::BackendInternal {
            message: format!(
                "backend {} ({}): {}",
                status, env.error.code, env.error.message
            ),
        },
        Err(_) => MinosError::BackendInternal {
            message: format!("backend {status}"),
        },
    }
}

pub(crate) fn http_base(ws_url: &str) -> Option<String> {
    let url = url::Url::parse(ws_url).ok()?;
    let scheme = match url.scheme() {
        "ws" => "http",
        "wss" => "https",
        other => other,
    };
    let host = url.host_str()?;
    let port = url.port().map(|p| format!(":{p}")).unwrap_or_default();
    Some(format!("{scheme}://{host}{port}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derive_http_base_from_ws_url() {
        assert_eq!(
            http_base("ws://127.0.0.1:8787/devices").unwrap(),
            "http://127.0.0.1:8787"
        );
        // `url` crate strips the default port (443 for wss) on parse, so
        // both bare and explicit-default-port forms collapse to the same
        // base — this is fine because reqwest will reapply the default.
        assert_eq!(
            http_base("wss://example.com/devices").unwrap(),
            "https://example.com"
        );
        assert_eq!(
            http_base("wss://example.com:8443/devices").unwrap(),
            "https://example.com:8443"
        );
    }
}
