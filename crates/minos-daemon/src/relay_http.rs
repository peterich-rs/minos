//! HTTP client for the backend's `/v1/*` control plane.
//!
//! Built on `openwire`. Used by `RelayClient` to issue formal host pairing
//! codes, fetch short-lived realtime tickets, run POST-first control-plane
//! queries, and forget pairings without going through the realtime socket.

use std::sync::Once;
use std::time::Duration;

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use ed25519_dalek::{Signer, SigningKey};
use http::{
    header::{AUTHORIZATION, CONTENT_TYPE},
    Method, Request, Response, StatusCode,
};
use minos_domain::{DeviceId, DeviceSecret, MinosError, PairingToken};
use minos_protocol::{
    HostPeerSummary, HostWsTicketResponse, MePeerResponse, MePeersResponse, PairingQrPayload,
    RequestPairingQrResponse,
};
use openwire::{Client, RequestBody, ResponseBody, WireError};
use serde::Deserialize;

use crate::openwire_trace::{logger_interceptor, OpenwireTraceFactory};

const HTTP_TIMEOUT: Duration = Duration::from_secs(10);
static INSTALL_RUSTLS_PROVIDER: Once = Once::new();
const BOOTSTRAP_NONCE_PATH: &str = "/v1/host/bootstrap/nonce";
const REQUEST_CODE_PATH: &str = "/v1/host/pairing/request-code";
const REDEEM_PATH: &str = "/v1/host/pairing/redeem";
/// Signed path segment for same-account Host Link (D02); no leading slash.
pub const HOST_LINK_PROOF_PATH: &str = "v1/hosts/link";
const HOST_WS_TICKET_PATH: &str = "/v1/host/realtime/ws-ticket";
const HOST_SELF_PATH: &str = "/v1/host/installations/self";

#[derive(Debug, serde::Serialize)]
struct BootstrapNonceRequest {
    installation_id: String,
}

#[derive(Debug, Deserialize)]
struct BootstrapNonceData {
    nonce: String,
}

#[derive(Debug, serde::Serialize)]
struct HostPairingRequestCodeRequest {
    installation_id: String,
    nonce: String,
    public_key: Option<String>,
    signature: String,
    host_display_name: Option<String>,
}

#[derive(Debug, serde::Serialize)]
struct HostPairingRedeemRequest {
    installation_id: String,
    nonce: String,
    public_key: Option<String>,
    signature: String,
    pairing_code: String,
    client_request_id: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ResponseEnvelope<T> {
    data: T,
}

#[derive(Debug, Deserialize)]
struct HostPairingRedeemData {
    host_installation_id: String,
    host_installation_token: String,
    issued_at_ms: i64,
}

#[derive(Debug, Deserialize)]
struct HostSelfData {
    links: Vec<HostSelfLinkSummary>,
}

#[derive(Debug, Deserialize)]
struct HostSelfLinkSummary {
    linked_via_installation_id: String,
    link_display_name: String,
    paired_at_ms: i64,
    #[serde(default)]
    last_active_at_ms: i64,
    #[serde(default)]
    online: bool,
}

#[derive(Debug, Deserialize)]
struct ErrorEnvelope {
    error: ErrorBody,
}

#[derive(Debug, Deserialize)]
struct ErrorBody {
    code: String,
    message: String,
}

pub enum PairingRedeemOutcome {
    Pending,
    Redeemed {
        host_installation_id: String,
        token: DeviceSecret,
        issued_at_ms: i64,
    },
}

pub struct RelayHttpClient {
    client: Client,
    base: String,
    device_id: DeviceId,
    device_role: &'static str,
    device_name: String,
    host_signing_key: SigningKey,
}

impl RelayHttpClient {
    pub fn new(
        backend_ws_url: &str,
        device_id: DeviceId,
        device_name: String,
    ) -> Result<Self, MinosError> {
        let host_signing_key = crate::host_bootstrap_key_store::load_or_generate()?;
        Self::new_with_signing_key(backend_ws_url, device_id, device_name, host_signing_key)
    }

    fn new_with_signing_key(
        backend_ws_url: &str,
        device_id: DeviceId,
        device_name: String,
        host_signing_key: SigningKey,
    ) -> Result<Self, MinosError> {
        INSTALL_RUSTLS_PROVIDER.call_once(|| {
            let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
        });

        let base = http_base(backend_ws_url).ok_or_else(|| MinosError::ConnectFailed {
            url: backend_ws_url.into(),
            message: "cannot derive HTTP base from backend URL".into(),
        })?;
        tracing::info!(
            target: "minos_daemon::relay_http",
            backend_ws_url,
            http_base = %base,
            "relay HTTP client configured"
        );
        let client = Client::builder()
            .call_timeout(HTTP_TIMEOUT)
            .application_interceptor(logger_interceptor("relay_http"))
            .event_listener_factory(OpenwireTraceFactory::new("relay_http"))
            .build()
            .map_err(|e| MinosError::BackendInternal {
                message: format!("openwire build: {e}"),
            })?;
        Ok(Self {
            client,
            base,
            device_id,
            device_role: "agent-host",
            device_name,
            host_signing_key,
        })
    }

    pub async fn request_pairing_qr(
        &self,
        host_display_name: String,
    ) -> Result<PairingQrPayload, MinosError> {
        let nonce = self.fetch_bootstrap_nonce().await?;
        let url = format!("{}{}", self.base, REQUEST_CODE_PATH);
        let request = self.request_with_json(
            Method::POST,
            &url,
            None,
            true,
            &HostPairingRequestCodeRequest {
                installation_id: self.device_id.to_string(),
                nonce: nonce.clone(),
                public_key: Some(self.host_public_key()),
                signature: self.host_signature(REQUEST_CODE_PATH, &nonce),
                host_display_name: Some(host_display_name),
            },
        )?;
        let resp = self.execute(&url, request).await?;
        let status = resp.status();
        if status.is_success() {
            let envelope: ResponseEnvelope<RequestPairingQrResponse> =
                decode_success_json(resp, "RequestPairingQrResponse").await?;
            Ok(envelope.data.qr_payload)
        } else {
            Err(decode_error(resp).await)
        }
    }

    pub async fn redeem_pairing_code(
        &self,
        pairing_code: &PairingToken,
    ) -> Result<PairingRedeemOutcome, MinosError> {
        let nonce = self.fetch_bootstrap_nonce().await?;
        let url = format!("{}{}", self.base, REDEEM_PATH);
        let request = self.request_with_json(
            Method::POST,
            &url,
            None,
            false,
            &HostPairingRedeemRequest {
                installation_id: self.device_id.to_string(),
                nonce: nonce.clone(),
                public_key: Some(self.host_public_key()),
                signature: self.host_signature(REDEEM_PATH, &nonce),
                pairing_code: pairing_code.as_str().to_string(),
                client_request_id: Some(format!("host-redeem-{}", uuid::Uuid::new_v4())),
            },
        )?;
        let resp = self.execute(&url, request).await?;
        let status = resp.status();
        if status.is_success() {
            let envelope: ResponseEnvelope<HostPairingRedeemData> =
                decode_success_json(resp, "HostPairingRedeemData").await?;
            return Ok(PairingRedeemOutcome::Redeemed {
                host_installation_id: envelope.data.host_installation_id,
                token: DeviceSecret(envelope.data.host_installation_token),
                issued_at_ms: envelope.data.issued_at_ms,
            });
        }

        let error = decode_error_envelope(resp).await;
        if status == StatusCode::CONFLICT {
            if let Some(env) = &error {
                if env.error.code == "pairing_not_confirmed"
                    || (env.error.code == "pairing_state_mismatch"
                        && env.error.message == "pending")
                {
                    return Ok(PairingRedeemOutcome::Pending);
                }
                if env.error.code == "pairing_code_invalid" {
                    return Err(MinosError::PairingTokenInvalid);
                }
            }
        }
        Err(error_to_minos(status, error))
    }

    pub async fn forget_pairing(&self, secret: &DeviceSecret) -> Result<(), MinosError> {
        let url = format!("{}/v1/pairing", self.base);
        let request = self.request_without_body(Method::DELETE, &url, Some(secret), false)?;
        let resp = self.execute(&url, request).await?;
        let status = resp.status();
        if status == StatusCode::NO_CONTENT || status == StatusCode::NOT_FOUND {
            // Idempotent: nothing to forget is fine.
            Ok(())
        } else {
            Err(decode_error(resp).await)
        }
    }

    pub async fn forget_peer_device(
        &self,
        secret: &DeviceSecret,
        mobile_device_id: DeviceId,
    ) -> Result<(), MinosError> {
        let url = format!("{}/v1/me/peers/{}", self.base, mobile_device_id);
        let request = self.request_without_body(Method::DELETE, &url, Some(secret), false)?;
        let resp = self.execute(&url, request).await?;
        let status = resp.status();
        if status == StatusCode::NO_CONTENT || status == StatusCode::NOT_FOUND {
            return Ok(());
        }
        Err(decode_error(resp).await)
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
        let url = format!("{}/v1/me/peer/query", self.base);
        let request = self.request_without_body(Method::POST, &url, Some(secret), false)?;
        let resp = self.execute(&url, request).await?;
        let status = resp.status();
        if status.is_success() {
            let body: MePeerResponse = decode_success_json(resp, "MePeerResponse").await?;
            return Ok(Some(body));
        }
        if status == StatusCode::NOT_FOUND {
            // 404 with `error.code == "not_paired"` is the structured
            // "no peer" signal. Anything else under 404 is unexpected
            // (e.g. an unknown route) — surface as an error so the
            // caller logs it.
            let body: Result<ErrorEnvelope, _> = resp.into_body().json().await;
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
        Err(decode_error(resp).await)
    }

    pub async fn get_me_peers(
        &self,
        secret: &DeviceSecret,
    ) -> Result<Vec<HostPeerSummary>, MinosError> {
        let url = format!("{}/v1/me/peers/query", self.base);
        let request = self.request_without_body(Method::POST, &url, Some(secret), false)?;
        let resp = self.execute(&url, request).await?;
        let status = resp.status();
        if status.is_success() {
            let body: MePeersResponse = decode_success_json(resp, "MePeersResponse").await?;
            return Ok(body.peers);
        }
        Err(decode_error(resp).await)
    }

    pub async fn get_host_peers(
        &self,
        token: &DeviceSecret,
    ) -> Result<Vec<HostPeerSummary>, MinosError> {
        let url = format!("{}{}", self.base, HOST_SELF_PATH);
        let request = self.request_host_bearer_without_body(Method::POST, &url, token, false)?;
        let resp = self.execute(&url, request).await?;
        let status = resp.status();
        if status.is_success() {
            let envelope: ResponseEnvelope<HostSelfData> =
                decode_success_json(resp, "HostSelfData").await?;
            return host_self_links_to_peer_summaries(envelope.data.links);
        }
        Err(decode_error(resp).await)
    }

    /// Fetch a short-lived ws-ticket for the host realtime gateway.
    ///
    /// Calls `POST /v1/host/realtime/ws-ticket` with the host installation
    /// bearer token. The backend returns a `ResponseEnvelope { data, meta }`
    /// where `data` matches [`HostWsTicketResponse`].
    pub async fn fetch_host_ws_ticket(
        &self,
        secret: &DeviceSecret,
    ) -> Result<HostWsTicketResponse, MinosError> {
        let url = format!("{}{}", self.base, HOST_WS_TICKET_PATH);
        let request = self.request_host_bearer_without_body(Method::POST, &url, secret, true)?;
        let resp = self.execute(&url, request).await?;
        let status = resp.status();
        if status.is_success() {
            // The backend wraps the ticket in ResponseEnvelope { data, meta }.
            // Deserialize the outer envelope and extract `data`.
            let envelope: ResponseEnvelope<HostWsTicketResponse> =
                decode_success_json(resp, "HostWsTicketResponse").await?;
            return Ok(envelope.data);
        }
        Err(decode_error(resp).await)
    }

    #[must_use]
    pub fn device_id(&self) -> DeviceId {
        self.device_id
    }

    pub async fn fetch_bootstrap_nonce(&self) -> Result<String, MinosError> {
        let url = format!("{}{}", self.base, BOOTSTRAP_NONCE_PATH);
        let request = self.request_with_json(
            Method::POST,
            &url,
            None,
            false,
            &BootstrapNonceRequest {
                installation_id: self.device_id.to_string(),
            },
        )?;
        let resp = self.execute(&url, request).await?;
        let status = resp.status();
        if status.is_success() {
            let envelope: ResponseEnvelope<BootstrapNonceData> =
                decode_success_json(resp, "BootstrapNonceData").await?;
            return Ok(envelope.data.nonce);
        }
        Err(decode_error(resp).await)
    }

    #[must_use]
    pub fn host_public_key(&self) -> String {
        format!(
            "ed25519:{}",
            URL_SAFE_NO_PAD.encode(self.host_signing_key.verifying_key().to_bytes())
        )
    }

    #[must_use]
    pub fn host_signature(&self, path: &str, nonce: &str) -> String {
        let payload = format!("{}:{nonce}:{path}", self.device_id);
        format!(
            "ed25519-sig:{}",
            URL_SAFE_NO_PAD.encode(self.host_signing_key.sign(payload.as_bytes()).to_bytes())
        )
    }

    /// Sign the same-account Host Link proof over `v1/hosts/link`.
    #[must_use]
    pub fn host_link_signature(&self, nonce: &str) -> String {
        self.host_signature(HOST_LINK_PROOF_PATH, nonce)
    }

    fn request_with_json<T>(
        &self,
        method: Method,
        url: &str,
        secret: Option<&DeviceSecret>,
        include_device_name: bool,
        body: &T,
    ) -> Result<Request<RequestBody>, MinosError>
    where
        T: serde::Serialize,
    {
        let payload = RequestBody::from_json(body).map_err(|e| MinosError::BackendInternal {
            message: format!("encode request body {url}: {e}"),
        })?;
        self.finish_request(
            self.request_builder(method, url, secret, include_device_name)
                .header(CONTENT_TYPE, "application/json"),
            payload,
            url,
        )
    }

    fn request_without_body(
        &self,
        method: Method,
        url: &str,
        secret: Option<&DeviceSecret>,
        include_device_name: bool,
    ) -> Result<Request<RequestBody>, MinosError> {
        self.finish_request(
            self.request_builder(method, url, secret, include_device_name),
            RequestBody::absent(),
            url,
        )
    }

    fn request_host_bearer_without_body(
        &self,
        method: Method,
        url: &str,
        token: &DeviceSecret,
        include_device_name: bool,
    ) -> Result<Request<RequestBody>, MinosError> {
        let mut req = Request::builder()
            .method(method)
            .uri(url)
            .header("x-device-id", self.device_id.to_string())
            .header("x-device-role", self.device_role)
            .header(AUTHORIZATION, format!("Bearer {}", token.as_str()));
        if include_device_name {
            req = req.header("x-device-name", &self.device_name);
        }
        self.finish_request(req, RequestBody::absent(), url)
    }

    fn request_builder(
        &self,
        method: Method,
        url: &str,
        secret: Option<&DeviceSecret>,
        include_device_name: bool,
    ) -> http::request::Builder {
        let mut req = Request::builder()
            .method(method)
            .uri(url)
            .header("x-device-id", self.device_id.to_string())
            .header("x-device-role", self.device_role);
        if include_device_name {
            req = req.header("x-device-name", &self.device_name);
        }
        if let Some(secret) = secret {
            req = req.header("x-device-secret", secret.as_str());
        }
        req
    }

    #[allow(clippy::unused_self)]
    fn finish_request(
        &self,
        req: http::request::Builder,
        body: RequestBody,
        url: &str,
    ) -> Result<Request<RequestBody>, MinosError> {
        req.body(body).map_err(|e| MinosError::BackendInternal {
            message: format!("build request {url}: {e}"),
        })
    }

    async fn execute(
        &self,
        url: &str,
        request: Request<RequestBody>,
    ) -> Result<Response<ResponseBody>, MinosError> {
        self.client
            .execute(request)
            .await
            .map_err(|e| connect_err(url, &e))
    }
}

fn connect_err(url: &str, e: &WireError) -> MinosError {
    if e.response_status() == Some(StatusCode::UNAUTHORIZED) {
        MinosError::Unauthorized {
            reason: format!("{url}: {e}"),
        }
    } else {
        MinosError::ConnectFailed {
            url: url.into(),
            message: e.to_string(),
        }
    }
}

fn host_self_links_to_peer_summaries(
    links: Vec<HostSelfLinkSummary>,
) -> Result<Vec<HostPeerSummary>, MinosError> {
    let mut peers = Vec::with_capacity(links.len());
    for link in links {
        let mobile_device_id = uuid::Uuid::parse_str(&link.linked_via_installation_id)
            .map(DeviceId)
            .map_err(|e| MinosError::BackendInternal {
                message: format!("decode host self linked device id: {e}"),
            })?;
        peers.push(HostPeerSummary {
            mobile_device_id,
            mobile_device_name: link.link_display_name,
            account_email: String::new(),
            paired_at_ms: link.paired_at_ms,
            last_active_at_ms: if link.last_active_at_ms > 0 {
                link.last_active_at_ms
            } else {
                link.paired_at_ms
            },
            online: link.online,
        });
    }
    Ok(peers)
}

async fn decode_success_json<T>(
    resp: Response<ResponseBody>,
    type_name: &str,
) -> Result<T, MinosError>
where
    T: serde::de::DeserializeOwned,
{
    resp.into_body()
        .json::<T>()
        .await
        .map_err(|e| MinosError::BackendInternal {
            message: format!("decode {type_name}: {e}"),
        })
}

async fn decode_error(resp: Response<ResponseBody>) -> MinosError {
    let status = resp.status();
    let body = decode_error_envelope(resp).await;
    error_to_minos(status, body)
}

async fn decode_error_envelope(resp: Response<ResponseBody>) -> Option<ErrorEnvelope> {
    resp.into_body().json().await.ok()
}

fn error_to_minos(status: StatusCode, body: Option<ErrorEnvelope>) -> MinosError {
    if status == StatusCode::UNAUTHORIZED {
        let reason = body
            .map(|env| format!("{}: {}", env.error.code, env.error.message))
            .unwrap_or_else(|| "backend unauthorized".to_string());
        return MinosError::Unauthorized { reason };
    }

    match body {
        Some(env) => MinosError::BackendInternal {
            message: format!(
                "backend {} ({}): {}",
                status, env.error.code, env.error.message
            ),
        },
        None => MinosError::BackendInternal {
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

    #[test]
    fn request_with_json_sets_application_json_content_type() {
        let client = RelayHttpClient::new_with_signing_key(
            "wss://example.com/devices",
            DeviceId::new(),
            "Minos Mac".into(),
            SigningKey::from_bytes(&[7_u8; 32]),
        )
        .unwrap();

        let request = client
            .request_with_json(
                Method::POST,
                "https://example.com/v1/host/pairing/request-code",
                None,
                true,
                &HostPairingRequestCodeRequest {
                    installation_id: client.device_id.to_string(),
                    nonce: "test-nonce".into(),
                    public_key: Some(client.host_public_key()),
                    signature: client.host_signature(REQUEST_CODE_PATH, "test-nonce"),
                    host_display_name: Some("Minos Mac".into()),
                },
            )
            .unwrap();

        assert_eq!(
            request.headers().get(CONTENT_TYPE).unwrap(),
            "application/json"
        );
    }

    #[test]
    fn host_self_links_preserve_presence_fields() {
        let mobile_device_id = DeviceId::new();
        let peers = host_self_links_to_peer_summaries(vec![HostSelfLinkSummary {
            linked_via_installation_id: mobile_device_id.to_string(),
            link_display_name: "Owner Phone".into(),
            paired_at_ms: 100,
            last_active_at_ms: 900,
            online: true,
        }])
        .unwrap();

        assert_eq!(peers.len(), 1);
        assert_eq!(peers[0].mobile_device_id, mobile_device_id);
        assert_eq!(peers[0].mobile_device_name, "Owner Phone");
        assert_eq!(peers[0].account_email, "");
        assert_eq!(peers[0].paired_at_ms, 100);
        assert_eq!(peers[0].last_active_at_ms, 900);
        assert!(peers[0].online);
    }

    #[test]
    fn host_link_signature_uses_d02_path_segment() {
        let device_id = DeviceId::new();
        let signing_key = SigningKey::from_bytes(&[11_u8; 32]);
        let client = RelayHttpClient::new_with_signing_key(
            "wss://example.com/devices",
            device_id,
            "Minos Mac".into(),
            signing_key.clone(),
        )
        .unwrap();
        let nonce = "nonce_test";
        let signature = client.host_link_signature(nonce);
        assert!(signature.starts_with("ed25519-sig:"));
        assert_eq!(
            signature,
            client.host_signature(HOST_LINK_PROOF_PATH, nonce)
        );
        let payload = format!("{device_id}:{nonce}:{HOST_LINK_PROOF_PATH}");
        let expected = format!(
            "ed25519-sig:{}",
            URL_SAFE_NO_PAD.encode(signing_key.sign(payload.as_bytes()).to_bytes())
        );
        assert_eq!(signature, expected);
    }
}
