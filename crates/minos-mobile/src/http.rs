//! HTTP client for the backend's `/v1/*` control plane.
//!
//! The mobile client uses this for the pre-WS pairing handshake (POST
//! `/v1/pairing/consume`), for listing the account's paired Macs
//! (`GET /v1/me/macs`), and for tearing a specific pair down
//! (`DELETE /v1/pairings/:mac_device_id`). The post-pair `Forward` /
//! `Forwarded` and event push traffic still flows over the WebSocket.
//!
//! ADR-0020 removed the iOS device-secret rail; every iOS-originated
//! request authenticates with the bearer alone.

use std::fmt::Write as _;
use std::sync::Once;
use std::time::Duration;

use http::header::CONTENT_TYPE;
use http::{Method, Request, Response, StatusCode};
use minos_domain::{DeviceId, MinosError};
use minos_protocol::{
    AuthRequest, AuthResponse, GetThreadLastSeqResponse, ListThreadsParams, ListThreadsResponse,
    LogoutRequest, MeHostsResponse, PairConsumeRequest, PairResponse, ReadThreadParams,
    ReadThreadResponse, RefreshRequest, RefreshResponse,
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
            device_role: "mobile-client",
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
        let request = self.request_with_json(Method::POST, &url, Some(access_token), &req)?;
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

    /// Tear down a specific account_mac_pairings row. The path-bound
    /// `host_device_id` is the Mac to forget; bearer-only auth post
    /// ADR-0020.
    pub async fn delete_pair(
        &self,
        access_token: &str,
        host_device_id: DeviceId,
    ) -> Result<(), MinosError> {
        let path = format!("/v1/pairings/{host_device_id}");
        let url = format!("{}{path}", self.base);
        let trace_id = start_http_trace(Method::DELETE.as_str(), &path, None, None);
        let request = self.request_without_body(Method::DELETE, &url, Some(access_token))?;
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

    /// List every Mac paired to the caller's account. Bearer-only.
    pub async fn list_paired_hosts(
        &self,
        access_token: &str,
    ) -> Result<MeHostsResponse, MinosError> {
        let url = format!("{}/v1/me/macs", self.base);
        let trace_id = start_http_trace(Method::GET.as_str(), "/v1/me/macs", None, None);
        let request = self.request_without_body(Method::GET, &url, Some(access_token))?;
        let resp = self.execute_with_trace(trace_id, &url, request).await?;
        let status = resp.status();
        if status.is_success() {
            let body: MeHostsResponse = decode_success_json(resp, "MeHostsResponse").await?;
            request_trace::finish_success(
                trace_id,
                Some(status.as_u16()),
                Some(format!("hosts={}", body.hosts.len())),
                None,
            );
            Ok(body)
        } else {
            let error = decode_error(resp).await;
            request_trace::finish_failure(trace_id, Some(status.as_u16()), error.to_string());
            Err(error)
        }
    }

    /// Bearer-only after ADR-0020. Lists the calling account's threads.
    pub async fn list_threads(
        &self,
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
        let request = self.request_without_body(Method::GET, &url, Some(access_token))?;
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
        let request = self.request_without_body(Method::GET, &url, Some(access_token))?;
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
        let request = self.request_without_body(Method::GET, &url, Some(access_token))?;
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
    /// Bearer-only post ADR-0020; the iOS rail no longer carries
    /// `X-Device-Secret`. Spec §5.2.
    pub async fn register(&self, email: &str, password: &str) -> Result<AuthResponse, MinosError> {
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
        let request = self.request_with_json(Method::POST, &url, None, &body)?;
        let resp = self.execute_with_trace(trace_id, &url, request).await?;
        decode_auth_response(resp, trace_id).await
    }

    /// `POST /v1/auth/login` — authenticate an existing account.
    /// Bearer-only post ADR-0020. Spec §5.2.
    pub async fn login(&self, email: &str, password: &str) -> Result<AuthResponse, MinosError> {
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
        let request = self.request_with_json(Method::POST, &url, None, &body)?;
        let resp = self.execute_with_trace(trace_id, &url, request).await?;
        decode_auth_response(resp, trace_id).await
    }

    /// `POST /v1/auth/refresh` — rotate the bearer + refresh pair.
    /// Bearer-only post ADR-0020.
    pub async fn refresh(&self, refresh_token: &str) -> Result<RefreshResponse, MinosError> {
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
        let request = self.request_with_json(Method::POST, &url, None, &body)?;
        let resp = self.execute_with_trace(trace_id, &url, request).await?;
        decode_refresh_response(resp, trace_id).await
    }

    /// `POST /v1/auth/logout` — revoke the named refresh token.
    /// Bearer-only post ADR-0020.
    pub async fn logout(&self, access_token: &str, refresh_token: &str) -> Result<(), MinosError> {
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
        let request = self.request_with_json(Method::POST, &url, Some(access_token), &body)?;
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

    /// Build a request stamped with the device-id + bearer token. CF-Access
    /// is also stamped if configured. Use this for any account-aware route
    /// the daemon adds in future phases.
    pub fn build_authed_request(
        &self,
        method: Method,
        path: &str,
        access: &str,
    ) -> Result<Request<RequestBody>, MinosError> {
        let url = format!("{}{}", self.base, path);
        self.request_without_body(method, &url, Some(access))
    }

    fn request_with_json<T>(
        &self,
        method: Method,
        url: &str,
        access_token: Option<&str>,
        body: &T,
    ) -> Result<Request<RequestBody>, MinosError>
    where
        T: Serialize,
    {
        let payload = RequestBody::from_json(body).map_err(|e| MinosError::BackendInternal {
            message: format!("encode request body {url}: {e}"),
        })?;
        Self::finish_request(
            self.request_builder(method, url, access_token)
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
    ) -> Result<Request<RequestBody>, MinosError> {
        Self::finish_request(
            self.request_builder(method, url, access_token),
            RequestBody::absent(),
            url,
        )
    }

    fn request_builder(
        &self,
        method: Method,
        url: &str,
        access_token: Option<&str>,
    ) -> http::request::Builder {
        let mut req = Request::builder()
            .method(method)
            .uri(url)
            .header("x-device-id", self.device_id.to_string())
            .header("x-device-role", self.device_role);
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
