//! HTTP client for the backend's `/v1/*` control plane.
//!
//! The mobile client uses this for the pre-WS pairing handshake (POST
//! `/v1/pairing/consume`) and for tearing the pair down (DELETE
//! `/v1/pairing`). The post-pair `Forward`/`Forwarded` and event push
//! traffic still flows over the WebSocket.

use std::fmt::Write as _;
use std::sync::Once;
use std::time::Duration;

use http::header::CONTENT_TYPE;
use http::{Method, Request, Response, StatusCode};
use minos_domain::{DeviceId, DeviceSecret, MinosError};
use minos_protocol::{
    AuthRequest, AuthResponse, GetThreadLastSeqResponse, ListThreadsParams, ListThreadsResponse,
    LogoutRequest, PairConsumeRequest, PairResponse, ReadThreadParams, ReadThreadResponse,
    RefreshRequest, RefreshResponse,
};
use openwire::{Client, RequestBody, ResponseBody, WireError};
use serde::{de::DeserializeOwned, Deserialize, Serialize};

use crate::request_trace::{self, RequestTransport};

const HTTP_TIMEOUT: Duration = Duration::from_secs(15);
static INSTALL_RUSTLS_PROVIDER: Once = Once::new();

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
        INSTALL_RUSTLS_PROVIDER.call_once(|| {
            let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
        });

        let base = http_base(backend_ws_url).ok_or_else(|| MinosError::ConnectFailed {
            url: backend_ws_url.into(),
            message: "cannot derive HTTP base from backend URL".into(),
        })?;
        let client = Client::builder()
            .call_timeout(HTTP_TIMEOUT)
            .build()
            .map_err(|e| MinosError::BackendInternal {
                message: format!("openwire build: {e}"),
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
        let trace_id = start_http_trace(
            Method::POST.as_str(),
            "/v1/pairing/consume",
            None,
            Some(format!("device_name={}", req.device_name)),
        );
        let request = self.request_with_json(Method::POST, &url, Some(access_token), None, &req)?;
        let resp = self.execute_with_trace(trace_id, &url, request).await?;
        let status = resp.status();
        if status.is_success() {
            let pair: PairResponse = decode_success_json(resp, "PairResponse").await?;
            request_trace::finish_success(
                trace_id,
                Some(status.as_u16()),
                Some(format!("peer_name={}", pair.peer_name)),
                None,
            );
            Ok(pair)
        } else {
            let error = decode_error(resp).await;
            request_trace::finish_failure(trace_id, Some(status.as_u16()), error.to_string());
            Err(error)
        }
    }

    pub async fn forget_pairing(&self, secret: &DeviceSecret) -> Result<(), MinosError> {
        let url = format!("{}/v1/pairing", self.base);
        let trace_id = start_http_trace(Method::DELETE.as_str(), "/v1/pairing", None, None);
        let request = self.request_without_body(Method::DELETE, &url, None, Some(secret))?;
        let resp = self.execute_with_trace(trace_id, &url, request).await?;
        let status = resp.status();
        if status == StatusCode::NO_CONTENT || status == StatusCode::NOT_FOUND {
            request_trace::finish_success(
                trace_id,
                Some(status.as_u16()),
                Some("pairing cleared".into()),
                None,
            );
            Ok(())
        } else {
            let error = decode_error(resp).await;
            request_trace::finish_failure(trace_id, Some(status.as_u16()), error.to_string());
            Err(error)
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
        let trace_id = start_http_trace(
            Method::GET.as_str(),
            "/v1/threads",
            None,
            Some(format!(
                "limit={} before_ts_ms={:?} agent={:?}",
                params.limit, params.before_ts_ms, params.agent
            )),
        );
        let mut url = format!("{}/v1/threads?limit={}", self.base, params.limit);
        if let Some(before) = params.before_ts_ms {
            let _ = write!(url, "&before_ts_ms={before}");
        }
        if let Some(agent) = params.agent {
            let agent_str = serde_json::to_string(&agent).unwrap_or_default();
            let agent_str = agent_str.trim_matches('"');
            let _ = write!(url, "&agent={agent_str}");
        }
        let request =
            self.request_without_body(Method::GET, &url, Some(access_token), Some(secret))?;
        let resp = self.execute_with_trace(trace_id, &url, request).await?;
        let status = resp.status();
        if status.is_success() {
            let threads: ListThreadsResponse =
                decode_success_json(resp, "ListThreadsResponse").await?;
            request_trace::finish_success(
                trace_id,
                Some(status.as_u16()),
                Some(format!("threads={}", threads.threads.len())),
                None,
            );
            Ok(threads)
        } else {
            let error = decode_error(resp).await;
            request_trace::finish_failure(trace_id, Some(status.as_u16()), error.to_string());
            Err(error)
        }
    }

    pub async fn read_thread(
        &self,
        secret: &DeviceSecret,
        access_token: &str,
        params: ReadThreadParams,
    ) -> Result<ReadThreadResponse, MinosError> {
        let thread_id = params.thread_id.clone();
        let trace_id = start_http_trace(
            Method::GET.as_str(),
            &format!("/v1/threads/{thread_id}/events"),
            Some(thread_id.clone()),
            Some(format!(
                "limit={} from_seq={:?}",
                params.limit, params.from_seq
            )),
        );
        let mut url = format!(
            "{}/v1/threads/{}/events?limit={}",
            self.base, params.thread_id, params.limit
        );
        if let Some(from) = params.from_seq {
            let _ = write!(url, "&from_seq={from}");
        }
        let request =
            self.request_without_body(Method::GET, &url, Some(access_token), Some(secret))?;
        let resp = self.execute_with_trace(trace_id, &url, request).await?;
        let status = resp.status();
        if status.is_success() {
            let thread: ReadThreadResponse =
                decode_success_json(resp, "ReadThreadResponse").await?;
            request_trace::finish_success(
                trace_id,
                Some(status.as_u16()),
                Some(format!(
                    "events={} next_seq={:?} end_reason={:?}",
                    thread.ui_events.len(),
                    thread.next_seq,
                    thread.thread_end_reason
                )),
                Some(thread_id),
            );
            Ok(thread)
        } else {
            let error = decode_error(resp).await;
            request_trace::finish_failure(trace_id, Some(status.as_u16()), error.to_string());
            Err(error)
        }
    }

    pub async fn get_thread_last_seq(
        &self,
        secret: &DeviceSecret,
        access_token: &str,
        thread_id: &str,
    ) -> Result<GetThreadLastSeqResponse, MinosError> {
        let url = format!("{}/v1/threads/{}/last_seq", self.base, thread_id);
        let trace_id = start_http_trace(
            Method::GET.as_str(),
            &format!("/v1/threads/{thread_id}/last_seq"),
            Some(thread_id.into()),
            None,
        );
        let request =
            self.request_without_body(Method::GET, &url, Some(access_token), Some(secret))?;
        let resp = self.execute_with_trace(trace_id, &url, request).await?;
        let status = resp.status();
        if status.is_success() {
            let last_seq: GetThreadLastSeqResponse =
                decode_success_json(resp, "GetThreadLastSeqResponse").await?;
            request_trace::finish_success(
                trace_id,
                Some(status.as_u16()),
                Some(format!("last_seq={}", last_seq.last_seq)),
                Some(thread_id.into()),
            );
            Ok(last_seq)
        } else {
            let error = decode_error(resp).await;
            request_trace::finish_failure(trace_id, Some(status.as_u16()), error.to_string());
            Err(error)
        }
    }

    // ─────────────────────────── auth endpoints ───────────────────────────

    /// `POST /v1/auth/register` — create an account on the backend.
    ///
    /// The pairing-rail `x-device-*` headers still authenticate the device;
    /// the new account-rail bearer/refresh tokens come back in the body.
    /// `device_secret` MUST be supplied if the device has been paired
    /// before — backend `authenticate()` rejects an empty secret on a row
    /// with `secret_hash != NULL`. Spec §5.2.
    pub async fn register(
        &self,
        email: &str,
        password: &str,
        device_secret: Option<&DeviceSecret>,
    ) -> Result<AuthResponse, MinosError> {
        let url = format!("{}/v1/auth/register", self.base);
        let trace_id = start_http_trace(
            Method::POST.as_str(),
            "/v1/auth/register",
            None,
            Some(format!("email={email}")),
        );
        let body = AuthRequest {
            email: email.into(),
            password: password.into(),
        };
        let request = self.request_with_json(Method::POST, &url, None, device_secret, &body)?;
        let resp = self.execute_with_trace(trace_id, &url, request).await?;
        decode_auth_response(resp, trace_id).await
    }

    /// `POST /v1/auth/login` — authenticate an existing account.
    ///
    /// `device_secret` MUST be supplied if the device has been paired
    /// before — same constraint as `register`. Spec §5.2.
    pub async fn login(
        &self,
        email: &str,
        password: &str,
        device_secret: Option<&DeviceSecret>,
    ) -> Result<AuthResponse, MinosError> {
        let url = format!("{}/v1/auth/login", self.base);
        let trace_id = start_http_trace(
            Method::POST.as_str(),
            "/v1/auth/login",
            None,
            Some(format!("email={email}")),
        );
        let body = AuthRequest {
            email: email.into(),
            password: password.into(),
        };
        let request = self.request_with_json(Method::POST, &url, None, device_secret, &body)?;
        let resp = self.execute_with_trace(trace_id, &url, request).await?;
        decode_auth_response(resp, trace_id).await
    }

    /// `POST /v1/auth/refresh` — rotate the bearer + refresh pair.
    ///
    /// The pairing-rail `x-device-*` headers must still be present so the
    /// backend can confirm the device is paired; the body carries the
    /// refresh-token plaintext (rotated server-side, returned new in the
    /// response).
    pub async fn refresh(
        &self,
        refresh_token: &str,
        device_secret: Option<&DeviceSecret>,
    ) -> Result<RefreshResponse, MinosError> {
        let url = format!("{}/v1/auth/refresh", self.base);
        let trace_id = start_http_trace(
            Method::POST.as_str(),
            "/v1/auth/refresh",
            None,
            Some("refresh session".into()),
        );
        let body = RefreshRequest {
            refresh_token: refresh_token.into(),
        };
        let request = self.request_with_json(Method::POST, &url, None, device_secret, &body)?;
        let resp = self.execute_with_trace(trace_id, &url, request).await?;
        decode_refresh_response(resp, trace_id).await
    }

    /// `POST /v1/auth/logout` — revoke the named refresh token.
    ///
    /// 204 No Content is the success status. The bearer token in
    /// `Authorization` authenticates the request; the body specifies which
    /// refresh token to revoke (the backend supports rotating-multi-device,
    /// so we name the specific one). Like the other auth endpoints, the
    /// pairing-rail `x-device-secret` is required once the device is paired.
    pub async fn logout(
        &self,
        access_token: &str,
        refresh_token: &str,
        device_secret: Option<&DeviceSecret>,
    ) -> Result<(), MinosError> {
        let url = format!("{}/v1/auth/logout", self.base);
        let trace_id = start_http_trace(
            Method::POST.as_str(),
            "/v1/auth/logout",
            None,
            Some("logout current session".into()),
        );
        let body = LogoutRequest {
            refresh_token: refresh_token.into(),
        };
        let request =
            self.request_with_json(Method::POST, &url, Some(access_token), device_secret, &body)?;
        let resp = self.execute_with_trace(trace_id, &url, request).await?;
        let status = resp.status();
        if status == StatusCode::NO_CONTENT || status.is_success() {
            request_trace::finish_success(
                trace_id,
                Some(status.as_u16()),
                Some("logged out".into()),
                None,
            );
            Ok(())
        } else {
            let error = decode_kind_error(resp).await;
            request_trace::finish_failure(trace_id, Some(status.as_u16()), error.to_string());
            Err(error)
        }
    }

    /// Build a request stamped with the pairing-rail device headers + the
    /// bearer token. Cb-Access is also stamped if configured. Use this for
    /// any account-aware route the daemon adds in future phases.
    pub fn build_authed_request(
        &self,
        method: Method,
        path: &str,
        access: &str,
    ) -> Result<Request<RequestBody>, MinosError> {
        let url = format!("{}{}", self.base, path);
        self.request_without_body(method, &url, Some(access), None)
    }

    fn request_with_json<T>(
        &self,
        method: Method,
        url: &str,
        access_token: Option<&str>,
        secret: Option<&DeviceSecret>,
        body: &T,
    ) -> Result<Request<RequestBody>, MinosError>
    where
        T: Serialize,
    {
        let payload = RequestBody::from_json(body).map_err(|e| MinosError::BackendInternal {
            message: format!("encode request body {url}: {e}"),
        })?;
        Self::finish_request(
            self.request_builder(method, url, access_token, secret)
                .header(CONTENT_TYPE, "application/json"),
            payload,
            url,
        )
    }

    fn request_without_body(
        &self,
        method: Method,
        url: &str,
        access_token: Option<&str>,
        secret: Option<&DeviceSecret>,
    ) -> Result<Request<RequestBody>, MinosError> {
        Self::finish_request(
            self.request_builder(method, url, access_token, secret),
            RequestBody::absent(),
            url,
        )
    }

    fn request_builder(
        &self,
        method: Method,
        url: &str,
        access_token: Option<&str>,
        secret: Option<&DeviceSecret>,
    ) -> http::request::Builder {
        let mut req = Request::builder()
            .method(method)
            .uri(url)
            .header("x-device-id", self.device_id.to_string())
            .header("x-device-role", self.device_role);
        if let Some(secret) = secret {
            req = req.header("x-device-secret", secret.as_str());
        }
        if let Some(access_token) = access_token {
            req = req.header("authorization", format!("Bearer {access_token}"));
        }
        if let Some((id, sec)) = &self.cf_access {
            req = req
                .header("cf-access-client-id", id)
                .header("cf-access-client-secret", sec);
        }
        req
    }

    fn finish_request(
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

    async fn execute_with_trace(
        &self,
        trace_id: u64,
        url: &str,
        request: Request<RequestBody>,
    ) -> Result<Response<ResponseBody>, MinosError> {
        match self.execute(url, request).await {
            Ok(resp) => Ok(resp),
            Err(error) => {
                request_trace::finish_failure(trace_id, None, error.to_string());
                Err(error)
            }
        }
    }
}

fn connect_err(url: &str, e: &WireError) -> MinosError {
    if e.response_status() == Some(StatusCode::UNAUTHORIZED) {
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

async fn decode_success_json<T>(
    resp: Response<ResponseBody>,
    type_name: &str,
) -> Result<T, MinosError>
where
    T: DeserializeOwned,
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
    let body: Result<ErrorEnvelope, _> = resp.into_body().json().await;
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
async fn decode_auth_response(
    resp: Response<ResponseBody>,
    trace_id: u64,
) -> Result<AuthResponse, MinosError> {
    let status = resp.status();
    if status.is_success() {
        let auth = decode_success_json::<AuthResponse>(resp, "AuthResponse").await?;
        request_trace::finish_success(
            trace_id,
            Some(status.as_u16()),
            Some(format!(
                "account={} expires_in={}s",
                auth.account.email, auth.expires_in
            )),
            None,
        );
        return Ok(auth);
    }
    let error = decode_kind_error(resp).await;
    request_trace::finish_failure(trace_id, Some(status.as_u16()), error.to_string());
    Err(error)
}

/// Decode a `RefreshResponse` from the backend, mapping `kind` strings on
/// the failure path to typed `MinosError` variants.
async fn decode_refresh_response(
    resp: Response<ResponseBody>,
    trace_id: u64,
) -> Result<RefreshResponse, MinosError> {
    let status = resp.status();
    if status.is_success() {
        let refresh = decode_success_json::<RefreshResponse>(resp, "RefreshResponse").await?;
        request_trace::finish_success(
            trace_id,
            Some(status.as_u16()),
            Some(format!("expires_in={}s", refresh.expires_in)),
            None,
        );
        return Ok(refresh);
    }
    let error = decode_kind_error(resp).await;
    request_trace::finish_failure(trace_id, Some(status.as_u16()), error.to_string());
    Err(error)
}

/// Map an HTTP error response that carries a `{ "kind": "..." }` body to
/// a typed `MinosError`. Used by every `/v1/auth/*` endpoint. Spec §8.1.
async fn decode_kind_error(resp: Response<ResponseBody>) -> MinosError {
    let (parts, body) = resp.into_parts();
    let retry_after = parts
        .headers
        .get("retry-after")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.parse::<u32>().ok())
        .unwrap_or(60);
    let body: serde_json::Value = body.json().await.unwrap_or(serde_json::Value::Null);
    let kind = body
        .get("kind")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown")
        .to_string();
    match (parts.status.as_u16(), kind.as_str()) {
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
            message: format!("{} {kind}", parts.status),
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

fn start_http_trace(
    method: &str,
    target: &str,
    thread_id: Option<String>,
    request_summary: Option<String>,
) -> u64 {
    request_trace::start(
        RequestTransport::Http,
        method,
        target,
        thread_id,
        request_summary,
    )
}
