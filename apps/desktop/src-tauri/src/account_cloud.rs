//! Account HTTP: Supabase exchange, refresh, logout. Desktop never uses a WS ticket.

use minos_domain::DeviceId;
use serde::{Deserialize, Serialize};

use crate::identity::{self, DesktopAccount};

fn backend_http_base() -> String {
    std::env::var("MINOS_BACKEND_URL")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .or_else(|| option_env!("VITE_MINOS_BACKEND_URL").map(str::to_string))
        .unwrap_or_else(|| "http://127.0.0.1:8787".into())
        .trim_end_matches('/')
        .to_string()
}

#[derive(Debug, Deserialize)]
struct AuthResp {
    account: AccountSummary,
    access_token: String,
    refresh_token: String,
    expires_in: i64,
    #[serde(default)]
    host_token: Option<String>,
}

#[derive(Debug, Deserialize)]
struct AccountSummary {
    account_id: String,
    email: String,
}

#[derive(Debug, Deserialize)]
struct RefreshResp {
    access_token: String,
    refresh_token: String,
    expires_in: i64,
}

#[derive(Debug, Serialize)]
struct SupabaseBody<'a> {
    access_token: &'a str,
    device_name: &'a str,
}

#[derive(Debug, Serialize)]
struct RefreshBody<'a> {
    refresh_token: &'a str,
}

#[derive(Debug, Serialize)]
struct LogoutBody<'a> {
    refresh_token: &'a str,
}

fn device_headers(device_id: DeviceId, access: Option<&str>) -> reqwest::header::HeaderMap {
    let mut headers = reqwest::header::HeaderMap::new();
    headers.insert("content-type", "application/json".parse().unwrap());
    headers.insert("x-device-id", device_id.to_string().parse().unwrap());
    headers.insert("x-device-role", "desktop-console".parse().unwrap());
    headers.insert("x-device-name", "Minos Desktop".parse().unwrap());
    if let Some(token) = access {
        if let Ok(value) = format!("Bearer {token}").parse() {
            headers.insert(reqwest::header::AUTHORIZATION, value);
        }
    }
    headers
}

pub async fn exchange_supabase(
    device_id: DeviceId,
    supabase_access: &str,
) -> anyhow::Result<DesktopAccount> {
    let client = reqwest::Client::new();
    let url = format!("{}/v1/auth/supabase", backend_http_base());
    let resp = client
        .post(url)
        .headers(device_headers(device_id, None))
        .json(&SupabaseBody {
            access_token: supabase_access,
            device_name: "Minos Desktop",
        })
        .send()
        .await?;
    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        anyhow::bail!("supabase exchange failed ({status}): {body}");
    }
    let auth: AuthResp = resp.json().await?;
    Ok(DesktopAccount {
        device_id: device_id.to_string(),
        account_id: auth.account.account_id,
        email: auth.account.email,
        access_token: auth.access_token,
        refresh_token: auth.refresh_token,
        expires_in: auth.expires_in,
        host_token: auth.host_token,
    })
}

pub async fn refresh(device_id: DeviceId, refresh_token: &str) -> anyhow::Result<DesktopAccount> {
    let client = reqwest::Client::new();
    let url = format!("{}/v1/auth/refresh", backend_http_base());
    let resp = client
        .post(url)
        .headers(device_headers(device_id, None))
        .json(&RefreshBody { refresh_token })
        .send()
        .await?;
    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        anyhow::bail!("refresh failed ({status}): {body}");
    }
    let auth: RefreshResp = resp.json().await?;
    Ok(DesktopAccount {
        device_id: device_id.to_string(),
        account_id: identity::load_account_id()?.unwrap_or_default(),
        email: identity::load_email()?.unwrap_or_default(),
        access_token: auth.access_token,
        refresh_token: auth.refresh_token,
        expires_in: auth.expires_in,
        host_token: identity::load_host_token()?,
    })
}

pub async fn logout(
    device_id: DeviceId,
    access_token: &str,
    refresh_token: &str,
) -> anyhow::Result<()> {
    let client = reqwest::Client::new();
    let url = format!("{}/v1/auth/logout", backend_http_base());
    let resp = client
        .post(url)
        .headers(device_headers(device_id, Some(access_token)))
        .json(&LogoutBody { refresh_token })
        .send()
        .await?;
    if !resp.status().is_success() && resp.status().as_u16() != 204 {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        anyhow::bail!("logout failed ({status}): {body}");
    }
    Ok(())
}

/// `ws://` / `wss://` origin for `/ws/client`.
pub fn client_ws_url() -> String {
    let http = backend_http_base();
    let ws = if let Some(rest) = http.strip_prefix("https://") {
        format!("wss://{rest}")
    } else if let Some(rest) = http.strip_prefix("http://") {
        format!("ws://{rest}")
    } else {
        format!("ws://{http}")
    };
    format!("{ws}/ws/client")
}
