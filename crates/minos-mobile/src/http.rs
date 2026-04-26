//! HTTP client for the backend's `/v1/*` control plane.
//!
//! The mobile client uses this for the pre-WS pairing handshake (POST
//! `/v1/pairing/consume`) and for tearing the pair down (DELETE
//! `/v1/pairing`). The post-pair `Forward`/`Forwarded` and event push
//! traffic still flows over the WebSocket.

use std::fmt::Write as _;
use std::time::Duration;

use minos_domain::{DeviceId, DeviceSecret, MinosError};
use minos_protocol::{
    AuthRequest, AuthResponse, GetThreadLastSeqResponse, ListThreadsParams, ListThreadsResponse,
    LogoutRequest, PairConsumeRequest, PairResponse, ReadThreadParams, ReadThreadResponse,
    RefreshRequest, RefreshResponse,
};
use reqwest::Client;
use serde::Deserialize;

const HTTP_TIMEOUT: Duration = Duration::from_secs(15);

#[derive(Debug, Deserialize)]
struct ErrorEnvelope {
    error: ErrorBody,
}

#[derive(Debug, Deserialize)]
struct ErrorBody {
    code: String,
    message: String,
}

pub struct MobileHttpClient {
    client: Client,
    base: String,
    device_id: DeviceId,
    device_role: &'static str,
    cf_access: Option<(String, String)>,
}

impl MobileHttpClient {
    pub fn new(
        backend_ws_url: &str,
        device_id: DeviceId,
        cf_access: Option<(String, String)>,
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
            device_role: "ios-client",
            cf_access,
        })
    }

    /// Redeem a pairing token (the QR's one-shot secret) into a long-lived
    /// `DeviceSecret`. Phase 2 made the `ios-client` rail bearer-gated, so
    /// callers must supply a valid `access_token` minted by `register` /
    /// `login`. The bearer is bound to the device id by the JWT `did`
    /// claim, so the same token must come from the same MobileClient that
    /// is performing the pair.
    pub async fn pair_consume(
        &self,
        req: PairConsumeRequest,
        access_token: &str,
    ) -> Result<PairResponse, MinosError> {
        let url = format!("{}/v1/pairing/consume", self.base);
        let r = self
            .client
            .post(&url)
            .header("x-device-id", self.device_id.to_string())
            .header("x-device-role", self.device_role)
            .header("authorization", format!("Bearer {access_token}"));
        let r = stamp_cf(r, self.cf_access.as_ref());
        let resp = r
            .json(&req)
            .send()
            .await
            .map_err(|e| connect_err(&url, &e))?;
        let status = resp.status();
        if status.is_success() {
            resp.json().await.map_err(|e| MinosError::BackendInternal {
                message: format!("decode PairResponse: {e}"),
            })
        } else {
            Err(decode_error(status, resp).await)
        }
    }

    pub async fn forget_pairing(&self, secret: &DeviceSecret) -> Result<(), MinosError> {
        let url = format!("{}/v1/pairing", self.base);
        let r = self
            .client
            .delete(&url)
            .header("x-device-id", self.device_id.to_string())
            .header("x-device-role", self.device_role)
            .header("x-device-secret", secret.as_str());
        let r = stamp_cf(r, self.cf_access.as_ref());
        let resp = r.send().await.map_err(|e| connect_err(&url, &e))?;
        let status = resp.status();
        if status == reqwest::StatusCode::NO_CONTENT || status == reqwest::StatusCode::NOT_FOUND {
            Ok(())
        } else {
            Err(decode_error(status, resp).await)
        }
    }

    /// Phase 2 added a bearer requirement to `/v1/threads`. Callers must
    /// supply both the device-secret and the access-token; the device-
    /// secret authenticates the WS-paired device, the bearer scopes the
    /// query to the caller's account.
    pub async fn list_threads(
        &self,
        secret: &DeviceSecret,
        access_token: &str,
        params: ListThreadsParams,
    ) -> Result<ListThreadsResponse, MinosError> {
        let mut url = format!("{}/v1/threads?limit={}", self.base, params.limit);
        if let Some(before) = params.before_ts_ms {
            let _ = write!(url, "&before_ts_ms={before}");
        }
        if let Some(agent) = params.agent {
            let agent_str = serde_json::to_string(&agent).unwrap_or_default();
            let agent_str = agent_str.trim_matches('"');
            let _ = write!(url, "&agent={agent_str}");
        }
        let r = self
            .client
            .get(&url)
            .header("x-device-id", self.device_id.to_string())
            .header("x-device-role", self.device_role)
            .header("x-device-secret", secret.as_str())
            .header("authorization", format!("Bearer {access_token}"));
        let r = stamp_cf(r, self.cf_access.as_ref());
        let resp = r.send().await.map_err(|e| connect_err(&url, &e))?;
        let status = resp.status();
        if status.is_success() {
            resp.json().await.map_err(|e| MinosError::BackendInternal {
                message: format!("decode ListThreadsResponse: {e}"),
            })
        } else {
            Err(decode_error(status, resp).await)
        }
    }

    pub async fn read_thread(
        &self,
        secret: &DeviceSecret,
        access_token: &str,
        params: ReadThreadParams,
    ) -> Result<ReadThreadResponse, MinosError> {
        let mut url = format!(
            "{}/v1/threads/{}/events?limit={}",
            self.base, params.thread_id, params.limit
        );
        if let Some(from) = params.from_seq {
            let _ = write!(url, "&from_seq={from}");
        }
        let r = self
            .client
            .get(&url)
            .header("x-device-id", self.device_id.to_string())
            .header("x-device-role", self.device_role)
            .header("x-device-secret", secret.as_str())
            .header("authorization", format!("Bearer {access_token}"));
        let r = stamp_cf(r, self.cf_access.as_ref());
        let resp = r.send().await.map_err(|e| connect_err(&url, &e))?;
        let status = resp.status();
        if status.is_success() {
            resp.json().await.map_err(|e| MinosError::BackendInternal {
                message: format!("decode ReadThreadResponse: {e}"),
            })
        } else {
            Err(decode_error(status, resp).await)
        }
    }

    pub async fn get_thread_last_seq(
        &self,
        secret: &DeviceSecret,
        access_token: &str,
        thread_id: &str,
    ) -> Result<GetThreadLastSeqResponse, MinosError> {
        let url = format!("{}/v1/threads/{}/last_seq", self.base, thread_id);
        let r = self
            .client
            .get(&url)
            .header("x-device-id", self.device_id.to_string())
            .header("x-device-role", self.device_role)
            .header("x-device-secret", secret.as_str())
            .header("authorization", format!("Bearer {access_token}"));
        let r = stamp_cf(r, self.cf_access.as_ref());
        let resp = r.send().await.map_err(|e| connect_err(&url, &e))?;
        let status = resp.status();
        if status.is_success() {
            resp.json().await.map_err(|e| MinosError::BackendInternal {
                message: format!("decode GetThreadLastSeqResponse: {e}"),
            })
        } else {
            Err(decode_error(status, resp).await)
        }
    }

    // ─────────────────────────── auth endpoints ───────────────────────────

    /// `POST /v1/auth/register` — create an account on the backend.
    ///
    /// The pairing-rail `x-device-*` headers still authenticate the device;
    /// the new account-rail bearer/refresh tokens come back in the body.
    pub async fn register(&self, email: &str, password: &str) -> Result<AuthResponse, MinosError> {
        let url = format!("{}/v1/auth/register", self.base);
        let body = AuthRequest {
            email: email.into(),
            password: password.into(),
        };
        let req = self
            .client
            .post(&url)
            .header("x-device-id", self.device_id.to_string())
            .header("x-device-role", self.device_role)
            .json(&body);
        let req = stamp_cf(req, self.cf_access.as_ref());
        let resp = req.send().await.map_err(|e| connect_err(&url, &e))?;
        decode_auth_response(resp).await
    }

    /// `POST /v1/auth/login` — authenticate an existing account.
    pub async fn login(&self, email: &str, password: &str) -> Result<AuthResponse, MinosError> {
        let url = format!("{}/v1/auth/login", self.base);
        let body = AuthRequest {
            email: email.into(),
            password: password.into(),
        };
        let req = self
            .client
            .post(&url)
            .header("x-device-id", self.device_id.to_string())
            .header("x-device-role", self.device_role)
            .json(&body);
        let req = stamp_cf(req, self.cf_access.as_ref());
        let resp = req.send().await.map_err(|e| connect_err(&url, &e))?;
        decode_auth_response(resp).await
    }

    /// `POST /v1/auth/refresh` — rotate the bearer + refresh pair.
    ///
    /// The pairing-rail `x-device-*` headers must still be present so the
    /// backend can confirm the device is paired; the body carries the
    /// refresh-token plaintext (rotated server-side, returned new in the
    /// response).
    pub async fn refresh(&self, refresh_token: &str) -> Result<RefreshResponse, MinosError> {
        let url = format!("{}/v1/auth/refresh", self.base);
        let body = RefreshRequest {
            refresh_token: refresh_token.into(),
        };
        let req = self
            .client
            .post(&url)
            .header("x-device-id", self.device_id.to_string())
            .header("x-device-role", self.device_role)
            .json(&body);
        let req = stamp_cf(req, self.cf_access.as_ref());
        let resp = req.send().await.map_err(|e| connect_err(&url, &e))?;
        decode_refresh_response(resp).await
    }

    /// `POST /v1/auth/logout` — revoke the named refresh token.
    ///
    /// 204 No Content is the success status. The bearer token in
    /// `Authorization` authenticates the request; the body specifies which
    /// refresh token to revoke (the backend supports rotating-multi-device,
    /// so we name the specific one).
    pub async fn logout(&self, access_token: &str, refresh_token: &str) -> Result<(), MinosError> {
        let url = format!("{}/v1/auth/logout", self.base);
        let body = LogoutRequest {
            refresh_token: refresh_token.into(),
        };
        let req = self
            .client
            .post(&url)
            .header("x-device-id", self.device_id.to_string())
            .header("x-device-role", self.device_role)
            .header("authorization", format!("Bearer {access_token}"))
            .json(&body);
        let req = stamp_cf(req, self.cf_access.as_ref());
        let resp = req.send().await.map_err(|e| connect_err(&url, &e))?;
        let status = resp.status();
        if status == reqwest::StatusCode::NO_CONTENT || status.is_success() {
            Ok(())
        } else {
            Err(decode_kind_error(status, resp).await)
        }
    }

    /// Build a request stamped with the pairing-rail device headers + the
    /// bearer token. Cb-Access is also stamped if configured. Use this for
    /// any account-aware route the daemon adds in future phases.
    pub fn build_authed_request(
        &self,
        method: reqwest::Method,
        path: &str,
        access: &str,
    ) -> reqwest::RequestBuilder {
        let url = format!("{}{}", self.base, path);
        let req = self
            .client
            .request(method, &url)
            .header("x-device-id", self.device_id.to_string())
            .header("x-device-role", self.device_role)
            .header("authorization", format!("Bearer {access}"));
        stamp_cf(req, self.cf_access.as_ref())
    }
}

fn stamp_cf(
    req: reqwest::RequestBuilder,
    cf: Option<&(String, String)>,
) -> reqwest::RequestBuilder {
    let mut req = req;
    if let Some((id, sec)) = cf {
        req = req
            .header("cf-access-client-id", id.as_str())
            .header("cf-access-client-secret", sec.as_str());
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
        Ok(env) => MinosError::RpcCallFailed {
            method: format!("http {status}"),
            message: format!("{}: {}", env.error.code, env.error.message),
        },
        Err(_) => MinosError::BackendInternal {
            message: format!("backend {status}"),
        },
    }
}

/// Decode an `AuthResponse` from the backend, mapping `kind` strings on
/// the failure path to typed `MinosError` variants. Spec §5.4, §8.1.
async fn decode_auth_response(resp: reqwest::Response) -> Result<AuthResponse, MinosError> {
    let status = resp.status();
    if status.is_success() {
        return resp
            .json::<AuthResponse>()
            .await
            .map_err(|e| MinosError::BackendInternal {
                message: format!("decode AuthResponse: {e}"),
            });
    }
    Err(decode_kind_error(status, resp).await)
}

/// Decode a `RefreshResponse` from the backend, mapping `kind` strings on
/// the failure path to typed `MinosError` variants.
async fn decode_refresh_response(resp: reqwest::Response) -> Result<RefreshResponse, MinosError> {
    let status = resp.status();
    if status.is_success() {
        return resp
            .json::<RefreshResponse>()
            .await
            .map_err(|e| MinosError::BackendInternal {
                message: format!("decode RefreshResponse: {e}"),
            });
    }
    Err(decode_kind_error(status, resp).await)
}

/// Map an HTTP error response that carries a `{ "kind": "..." }` body to
/// a typed `MinosError`. Used by every `/v1/auth/*` endpoint. Spec §8.1.
async fn decode_kind_error(status: reqwest::StatusCode, resp: reqwest::Response) -> MinosError {
    let retry_after = resp
        .headers()
        .get("retry-after")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.parse::<u32>().ok())
        .unwrap_or(60);
    let body: serde_json::Value = resp.json().await.unwrap_or(serde_json::Value::Null);
    let kind = body
        .get("kind")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown")
        .to_string();
    match (status.as_u16(), kind.as_str()) {
        (400, "weak_password") => MinosError::WeakPassword,
        (401, "invalid_credentials") => MinosError::InvalidCredentials,
        (401, "invalid_refresh") => MinosError::AuthRefreshFailed {
            message: "invalid refresh token".into(),
        },
        (401, _) => MinosError::Unauthorized {
            reason: format!("auth failed ({kind})"),
        },
        (409, "email_taken") => MinosError::EmailTaken,
        (429, _) => MinosError::RateLimited {
            retry_after_s: retry_after,
        },
        _ => MinosError::BackendInternal {
            message: format!("{status} {kind}"),
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
