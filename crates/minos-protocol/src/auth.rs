//! HTTP DTOs for the `/v1/auth/*` endpoints. Spec §5.2.
//!
//! Field shapes mirror `crates/minos-backend/src/http/v1/auth.rs::AuthResp`
//! / `AccountSummary` / `RefreshResp` so the JSON wire contract stays
//! single-sourced. Snake-case is implicit because the field idents
//! already use snake_case.

use serde::{Deserialize, Serialize};

/// Public summary of the account currently authenticated. Matches the
/// backend's `AccountSummary` payload one-for-one.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AuthSummary {
    pub account_id: String,
    pub email: String,
}

/// Body for `POST /v1/auth/register` and `POST /v1/auth/login`. Both
/// endpoints share this shape because register-then-login symmetry is a
/// hard requirement of the auth flow.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AuthRequest {
    pub email: String,
    pub password: String,
}

/// Successful response from `register` and `login`. Field names are
/// fixed to match the backend's `AuthResp` JSON shape — do not rename.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AuthResponse {
    pub account: AuthSummary,
    pub access_token: String,
    pub refresh_token: String,
    pub expires_in: i64,
}

/// Body for `POST /v1/auth/refresh`. The refresh token is rotated on
/// every call, so callers must persist the new token from
/// [`RefreshResponse`].
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RefreshRequest {
    pub refresh_token: String,
}

/// Successful response from `refresh`. Carries the rotated refresh
/// token alongside a new access token.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RefreshResponse {
    pub access_token: String,
    pub refresh_token: String,
    pub expires_in: i64,
}

/// Body for `POST /v1/auth/logout`. The bearer token in the
/// `Authorization` header still authenticates the caller; this body
/// names the specific refresh token to revoke.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LogoutRequest {
    pub refresh_token: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Roundtrip the canonical `AuthResponse` JSON shape so a future edit
    /// that drops a field or renames it (e.g. `access_token` →
    /// `accessToken`) trips this test instead of silently breaking the
    /// backend ↔ mobile contract.
    #[test]
    fn auth_response_round_trip() {
        let expected_json = serde_json::json!({
            "account": {
                "account_id": "acct-1",
                "email": "a@b.com",
            },
            "access_token": "tok",
            "refresh_token": "ref",
            "expires_in": 900,
        });
        let r = AuthResponse {
            account: AuthSummary {
                account_id: "acct-1".into(),
                email: "a@b.com".into(),
            },
            access_token: "tok".into(),
            refresh_token: "ref".into(),
            expires_in: 900,
        };
        let serialized = serde_json::to_value(&r).unwrap();
        assert_eq!(serialized, expected_json);
        let decoded: AuthResponse = serde_json::from_value(expected_json).unwrap();
        assert_eq!(decoded, r);
    }

    #[test]
    fn auth_request_round_trip() {
        let r = AuthRequest {
            email: "a@b.com".into(),
            password: "hunter22".into(),
        };
        let s = serde_json::to_string(&r).unwrap();
        let back: AuthRequest = serde_json::from_str(&s).unwrap();
        assert_eq!(r, back);
    }

    #[test]
    fn refresh_response_round_trip() {
        let r = RefreshResponse {
            access_token: "tok2".into(),
            refresh_token: "ref2".into(),
            expires_in: 900,
        };
        let s = serde_json::to_string(&r).unwrap();
        let back: RefreshResponse = serde_json::from_str(&s).unwrap();
        assert_eq!(r, back);
    }

    #[test]
    fn logout_request_round_trip() {
        let r = LogoutRequest {
            refresh_token: "ref".into(),
        };
        let s = serde_json::to_string(&r).unwrap();
        let back: LogoutRequest = serde_json::from_str(&s).unwrap();
        assert_eq!(r, back);
    }
}
