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
    GetThreadLastSeqResponse, ListThreadsParams, ListThreadsResponse, PairConsumeRequest,
    PairResponse, ReadThreadParams, ReadThreadResponse,
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

    pub async fn pair_consume(&self, req: PairConsumeRequest) -> Result<PairResponse, MinosError> {
        let url = format!("{}/v1/pairing/consume", self.base);
        let r = self
            .client
            .post(&url)
            .header("x-device-id", self.device_id.to_string())
            .header("x-device-role", self.device_role);
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

    pub async fn list_threads(
        &self,
        secret: &DeviceSecret,
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
            .header("x-device-secret", secret.as_str());
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
            .header("x-device-secret", secret.as_str());
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
        thread_id: &str,
    ) -> Result<GetThreadLastSeqResponse, MinosError> {
        let url = format!("{}/v1/threads/{}/last_seq", self.base, thread_id);
        let r = self
            .client
            .get(&url)
            .header("x-device-id", self.device_id.to_string())
            .header("x-device-role", self.device_role)
            .header("x-device-secret", secret.as_str());
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
