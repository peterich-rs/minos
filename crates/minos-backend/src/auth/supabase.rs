//! Supabase Auth JWT verification (JWKS + optional HS256 secret) for the
//! token-exchange path.
//!
//! Minos never treats Supabase as the business-session authority: clients
//! exchange a short-lived Supabase `access_token` for Minos access/refresh
//! tokens via `POST /v1/auth/supabase`. This module only validates the IdP
//! JWT (issuer, audience, expiry, signature) and extracts claims.
//!
//! Signing reality (2025–2026 Supabase projects):
//! - New projects publish **ES256** keys at `…/auth/v1/.well-known/jwks.json`.
//! - Some projects still issue **HS256** tokens with the legacy JWT secret.
//!   We accept either: try JWKS first, then optional `SUPABASE_JWT_SECRET`.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use jsonwebtoken::{decode, decode_header, Algorithm, DecodingKey, Validation};
use serde::Deserialize;
use tokio::sync::RwLock;

/// How long a fetched JWKS document is considered fresh.
const JWKS_TTL: Duration = Duration::from_mins(5);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SupabaseClaims {
    pub sub: String,
    pub email: Option<String>,
    pub email_verified: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SupabaseAuthError {
    /// Missing token, bad structure, bad signature, wrong alg.
    InvalidToken,
    /// `iss` / `aud` mismatch or other claim validation failure.
    InvalidClaims,
    /// JWT `exp` is in the past.
    Expired,
    /// JWKS endpoint unreachable or returned unusable keys.
    IdpUnavailable(String),
}

impl std::fmt::Display for SupabaseAuthError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidToken => write!(f, "invalid_supabase_token"),
            Self::InvalidClaims => write!(f, "supabase_token_invalid"),
            Self::Expired => write!(f, "supabase_token_expired"),
            Self::IdpUnavailable(msg) => write!(f, "idp_unavailable: {msg}"),
        }
    }
}

/// Runtime config for verifying Supabase access tokens.
#[derive(Debug, Clone)]
pub struct SupabaseConfig {
    /// Expected `iss` claim, e.g. `https://<ref>.supabase.co/auth/v1`.
    pub issuer: String,
    /// Expected `aud` claim (usually `"authenticated"`).
    pub audience: String,
    /// JWKS URL: `{SUPABASE_URL}/auth/v1/.well-known/jwks.json`.
    pub jwks_url: String,
    /// Optional legacy HS256 secret (Dashboard → Settings → API → JWT Secret).
    pub jwt_secret: Option<String>,
}

impl SupabaseConfig {
    /// Build from `SUPABASE_URL` + optional `SUPABASE_JWT_AUD` + optional secret.
    ///
    /// `supabase_url` should be `https://<project-ref>.supabase.co` (no trailing slash).
    pub fn from_url(
        supabase_url: &str,
        audience: Option<&str>,
        jwt_secret: Option<&str>,
    ) -> Result<Self, String> {
        let base = supabase_url.trim().trim_end_matches('/');
        if base.is_empty() {
            return Err("SUPABASE_URL is empty".into());
        }
        if !(base.starts_with("https://") || base.starts_with("http://")) {
            return Err("SUPABASE_URL must start with https:// or http://".into());
        }
        let issuer = format!("{base}/auth/v1");
        let jwks_url = format!("{base}/auth/v1/.well-known/jwks.json");
        // Supabase access tokens use audience `"authenticated"` by default.
        let audience = audience
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
            .unwrap_or_else(|| "authenticated".to_owned());
        let jwt_secret = jwt_secret
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_owned);
        Ok(Self {
            issuer,
            audience,
            jwks_url,
            jwt_secret,
        })
    }
}

#[derive(Debug, Deserialize)]
struct JwksDocument {
    keys: Vec<RawJwk>,
}

/// Minimal JWK fields we need. Extra Supabase fields (`ext`, `key_ops`, …)
/// are ignored by serde.
#[derive(Debug, Deserialize)]
struct RawJwk {
    kid: Option<String>,
    kty: String,
    #[allow(dead_code)]
    alg: Option<String>,
    n: Option<String>,
    e: Option<String>,
    crv: Option<String>,
    x: Option<String>,
    y: Option<String>,
}

struct CachedJwks {
    fetched_at: Instant,
    keys: HashMap<String, DecodingKey>,
    default_key: Option<DecodingKey>,
}

/// Thread-safe JWKS cache with TTL refresh.
pub struct JwksCache {
    url: String,
    inner: RwLock<Option<CachedJwks>>,
    http: reqwest::Client,
}

impl JwksCache {
    #[must_use]
    pub fn new(jwks_url: impl Into<String>) -> Self {
        Self {
            url: jwks_url.into(),
            inner: RwLock::new(None),
            http: reqwest::Client::builder()
                .timeout(Duration::from_secs(10))
                .build()
                .expect("reqwest client"),
        }
    }

    async fn get_key(&self, kid: Option<&str>) -> Result<DecodingKey, SupabaseAuthError> {
        {
            let guard = self.inner.read().await;
            if let Some(cache) = guard.as_ref() {
                if cache.fetched_at.elapsed() < JWKS_TTL {
                    if let Some(key) = resolve_key(cache, kid) {
                        return Ok(key);
                    }
                }
            }
        }
        self.refresh().await?;
        let guard = self.inner.read().await;
        let cache = guard.as_ref().ok_or_else(|| {
            SupabaseAuthError::IdpUnavailable("JWKS cache empty after refresh".into())
        })?;
        resolve_key(cache, kid).ok_or(SupabaseAuthError::InvalidToken)
    }

    async fn refresh(&self) -> Result<(), SupabaseAuthError> {
        let doc: JwksDocument = self
            .http
            .get(&self.url)
            .send()
            .await
            .map_err(|e| SupabaseAuthError::IdpUnavailable(e.to_string()))?
            .error_for_status()
            .map_err(|e| SupabaseAuthError::IdpUnavailable(e.to_string()))?
            .json()
            .await
            .map_err(|e| SupabaseAuthError::IdpUnavailable(e.to_string()))?;

        let mut keys = HashMap::new();
        let mut default_key = None;
        for jwk in &doc.keys {
            let Ok(key) = decoding_key_from_jwk(jwk) else {
                tracing::warn!(
                    target: "minos_backend::auth::supabase",
                    kty = %jwk.kty,
                    kid = ?jwk.kid,
                    "skipping unusable JWKS key"
                );
                continue;
            };
            if let Some(kid) = jwk.kid.clone() {
                keys.insert(kid, key.clone());
            }
            if default_key.is_none() {
                default_key = Some(key);
            }
        }
        if keys.is_empty() && default_key.is_none() {
            return Err(SupabaseAuthError::IdpUnavailable(
                "JWKS document contained no usable keys".into(),
            ));
        }
        tracing::info!(
            target: "minos_backend::auth::supabase",
            key_count = keys.len(),
            jwks_url = %self.url,
            "refreshed Supabase JWKS"
        );
        let mut guard = self.inner.write().await;
        *guard = Some(CachedJwks {
            fetched_at: Instant::now(),
            keys,
            default_key,
        });
        Ok(())
    }
}

fn resolve_key(cache: &CachedJwks, kid: Option<&str>) -> Option<DecodingKey> {
    if let Some(kid) = kid {
        if let Some(key) = cache.keys.get(kid) {
            return Some(key.clone());
        }
    }
    cache.default_key.clone()
}

fn decoding_key_from_jwk(jwk: &RawJwk) -> Result<DecodingKey, SupabaseAuthError> {
    match jwk.kty.as_str() {
        "RSA" => {
            let n = jwk.n.as_deref().ok_or(SupabaseAuthError::InvalidToken)?;
            let e = jwk.e.as_deref().ok_or(SupabaseAuthError::InvalidToken)?;
            DecodingKey::from_rsa_components(n, e).map_err(|_| SupabaseAuthError::InvalidToken)
        }
        "EC" => {
            let x = jwk.x.as_deref().ok_or(SupabaseAuthError::InvalidToken)?;
            let y = jwk.y.as_deref().ok_or(SupabaseAuthError::InvalidToken)?;
            if let Some(crv) = jwk.crv.as_deref() {
                if crv != "P-256" && crv != "P-384" && crv != "P-521" {
                    return Err(SupabaseAuthError::InvalidToken);
                }
            }
            DecodingKey::from_ec_components(x, y).map_err(|_| SupabaseAuthError::InvalidToken)
        }
        _ => Err(SupabaseAuthError::InvalidToken),
    }
}

/// Claims extracted from a Supabase access token. Unknown fields are ignored.
#[derive(Debug, Deserialize)]
struct RawClaims {
    sub: String,
    #[serde(default)]
    email: Option<String>,
    /// Accept bool or stringy `"true"` / `"false"`.
    #[serde(default, deserialize_with = "deserialize_flexible_bool")]
    email_verified: Option<bool>,
    #[serde(default, deserialize_with = "deserialize_flexible_bool")]
    email_confirmed: Option<bool>,
}

fn deserialize_flexible_bool<'de, D>(deserializer: D) -> Result<Option<bool>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de::{self, Visitor};
    use std::fmt;

    struct FlexBool;
    impl<'de> Visitor<'de> for FlexBool {
        type Value = Option<bool>;

        fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str("bool, string, or null")
        }

        fn visit_none<E: de::Error>(self) -> Result<Self::Value, E> {
            Ok(None)
        }

        fn visit_unit<E: de::Error>(self) -> Result<Self::Value, E> {
            Ok(None)
        }

        fn visit_bool<E: de::Error>(self, v: bool) -> Result<Self::Value, E> {
            Ok(Some(v))
        }

        fn visit_str<E: de::Error>(self, v: &str) -> Result<Self::Value, E> {
            match v.trim().to_ascii_lowercase().as_str() {
                "true" | "1" | "yes" => Ok(Some(true)),
                "false" | "0" | "no" => Ok(Some(false)),
                "" => Ok(None),
                other => Err(E::custom(format!("invalid bool string: {other}"))),
            }
        }

        fn visit_u64<E: de::Error>(self, v: u64) -> Result<Self::Value, E> {
            Ok(Some(v != 0))
        }

        fn visit_i64<E: de::Error>(self, v: i64) -> Result<Self::Value, E> {
            Ok(Some(v != 0))
        }
    }

    deserializer.deserialize_any(FlexBool)
}

/// Verifies Supabase access tokens via JWKS (asymmetric) and/or HS256 secret.
pub struct SupabaseTokenVerifier {
    config: SupabaseConfig,
    jwks: Option<JwksCache>,
    /// Production legacy secret and/or test HMAC material.
    hmac_secret: Option<Vec<u8>>,
}

impl SupabaseTokenVerifier {
    #[must_use]
    pub fn from_config(config: SupabaseConfig) -> Arc<Self> {
        let jwks = JwksCache::new(config.jwks_url.clone());
        let hmac_secret = config.jwt_secret.as_ref().map(|s| s.as_bytes().to_vec());
        Arc::new(Self {
            config,
            jwks: Some(jwks),
            hmac_secret,
        })
    }

    /// Construct a verifier that accepts HS256 tokens signed with `secret`.
    /// Used by unit/integration tests to avoid network JWKS fetches.
    #[must_use]
    pub fn for_tests(
        issuer: impl Into<String>,
        audience: impl Into<String>,
        secret: &[u8],
    ) -> Arc<Self> {
        Arc::new(Self {
            config: SupabaseConfig {
                issuer: issuer.into(),
                audience: audience.into(),
                jwks_url: "http://test.invalid/jwks".into(),
                jwt_secret: None,
            },
            jwks: None,
            hmac_secret: Some(secret.to_vec()),
        })
    }

    pub async fn verify(&self, token: &str) -> Result<SupabaseClaims, SupabaseAuthError> {
        let token = token.trim().trim_start_matches("Bearer ").trim();
        if token.is_empty() || token.matches('.').count() != 2 {
            tracing::warn!(
                target: "minos_backend::auth::supabase",
                "supabase token missing or not a JWT (expected 3 segments)"
            );
            return Err(SupabaseAuthError::InvalidToken);
        }

        let header = decode_header(token).map_err(|e| {
            tracing::warn!(
                target: "minos_backend::auth::supabase",
                error = %e,
                "failed to decode JWT header"
            );
            SupabaseAuthError::InvalidToken
        })?;

        let alg = header.alg;
        tracing::debug!(
            target: "minos_backend::auth::supabase",
            ?alg,
            kid = ?header.kid,
            "verifying supabase access token"
        );

        // Route by the algorithm in the JWT header (do not guess).
        match alg {
            Algorithm::HS256 | Algorithm::HS384 | Algorithm::HS512 => {
                self.verify_with_hmac(token, alg)
            }
            Algorithm::ES256
            | Algorithm::ES384
            | Algorithm::RS256
            | Algorithm::RS384
            | Algorithm::RS512 => self.verify_with_jwks(token, &header).await,
            other => {
                tracing::warn!(
                    target: "minos_backend::auth::supabase",
                    ?other,
                    "unsupported supabase JWT alg"
                );
                Err(SupabaseAuthError::InvalidToken)
            }
        }
    }

    async fn verify_with_jwks(
        &self,
        token: &str,
        header: &jsonwebtoken::Header,
    ) -> Result<SupabaseClaims, SupabaseAuthError> {
        let jwks = self
            .jwks
            .as_ref()
            .ok_or_else(|| SupabaseAuthError::IdpUnavailable("JWKS not configured".into()))?;
        let decoding_key = jwks.get_key(header.kid.as_deref()).await?;

        // IMPORTANT: jsonwebtoken rejects the whole verification if *any*
        // entry in `validation.algorithms` has a different key family than
        // the DecodingKey (Ec vs Rsa → InvalidAlgorithm). Keep one family.
        let mut validation = Validation::new(header.alg);
        validation.algorithms = match header.alg {
            Algorithm::ES256 | Algorithm::ES384 => {
                vec![Algorithm::ES256, Algorithm::ES384]
            }
            Algorithm::RS256
            | Algorithm::RS384
            | Algorithm::RS512
            | Algorithm::PS256
            | Algorithm::PS384
            | Algorithm::PS512 => {
                vec![
                    Algorithm::RS256,
                    Algorithm::RS384,
                    Algorithm::RS512,
                    Algorithm::PS256,
                    Algorithm::PS384,
                    Algorithm::PS512,
                ]
            }
            other => vec![other],
        };
        self.apply_common_validation(&mut validation);
        self.decode_claims(token, &decoding_key, &validation)
    }

    fn verify_with_hmac(
        &self,
        token: &str,
        alg: Algorithm,
    ) -> Result<SupabaseClaims, SupabaseAuthError> {
        let secret = self.hmac_secret.as_ref().ok_or_else(|| {
            tracing::warn!(
                target: "minos_backend::auth::supabase",
                "token is HS* but SUPABASE_JWT_SECRET is not configured"
            );
            SupabaseAuthError::InvalidToken
        })?;
        let decoding_key = DecodingKey::from_secret(secret);
        // HMAC family only — do not mix with RS/ES (see verify_with_jwks).
        let mut validation = Validation::new(alg);
        validation.algorithms = vec![Algorithm::HS256, Algorithm::HS384, Algorithm::HS512];
        self.apply_common_validation(&mut validation);
        self.decode_claims(token, &decoding_key, &validation)
    }

    fn apply_common_validation(&self, validation: &mut Validation) {
        validation.set_issuer(std::slice::from_ref(&self.config.issuer));
        validation.set_audience(&[self.config.audience.as_str()]);
        validation.validate_exp = true;
        // Small clock skew allowance for laptop/VPS drift.
        validation.leeway = 30;
    }

    fn decode_claims(
        &self,
        token: &str,
        decoding_key: &DecodingKey,
        validation: &Validation,
    ) -> Result<SupabaseClaims, SupabaseAuthError> {
        let data = decode::<RawClaims>(token, decoding_key, validation).map_err(|e| {
            use jsonwebtoken::errors::ErrorKind;
            let kind = e.kind();
            tracing::warn!(
                target: "minos_backend::auth::supabase",
                error = %e,
                kind = ?kind,
                issuer = %self.config.issuer,
                audience = %self.config.audience,
                "supabase JWT verification failed"
            );
            match kind {
                ErrorKind::ExpiredSignature => SupabaseAuthError::Expired,
                ErrorKind::InvalidIssuer
                | ErrorKind::InvalidAudience
                | ErrorKind::InvalidSubject => SupabaseAuthError::InvalidClaims,
                _ => SupabaseAuthError::InvalidToken,
            }
        })?;

        let email = data
            .claims
            .email
            .map(|e| e.trim().to_lowercase())
            .filter(|e| !e.is_empty());
        let email_verified = data.claims.email_verified.unwrap_or(false)
            || data.claims.email_confirmed.unwrap_or(false);

        if data.claims.sub.trim().is_empty() {
            return Err(SupabaseAuthError::InvalidClaims);
        }

        Ok(SupabaseClaims {
            sub: data.claims.sub,
            email,
            email_verified,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use jsonwebtoken::{encode, EncodingKey, Header};

    const SECRET: &[u8] = b"test-supabase-hmac-secret-32b!";
    const ISS: &str = "https://example.supabase.co/auth/v1";
    const AUD: &str = "authenticated";

    fn sign(claims: serde_json::Value) -> String {
        encode(
            &Header::new(Algorithm::HS256),
            &claims,
            &EncodingKey::from_secret(SECRET),
        )
        .unwrap()
    }

    #[tokio::test]
    async fn accepts_valid_hs256_token() {
        let verifier = SupabaseTokenVerifier::for_tests(ISS, AUD, SECRET);
        let now = chrono::Utc::now().timestamp();
        let token = sign(serde_json::json!({
            "sub": "user-1",
            "email": "Alice@Example.com",
            "email_verified": true,
            "iss": ISS,
            "aud": AUD,
            "exp": now + 3600,
            "iat": now,
        }));
        let claims = verifier.verify(&token).await.unwrap();
        assert_eq!(claims.sub, "user-1");
        assert_eq!(claims.email.as_deref(), Some("alice@example.com"));
        assert!(claims.email_verified);
    }

    #[tokio::test]
    async fn accepts_email_verified_string_true() {
        let verifier = SupabaseTokenVerifier::for_tests(ISS, AUD, SECRET);
        let now = chrono::Utc::now().timestamp();
        let token = sign(serde_json::json!({
            "sub": "user-2",
            "email": "bob@example.com",
            "email_verified": "true",
            "iss": ISS,
            "aud": AUD,
            "exp": now + 3600,
            "iat": now,
        }));
        let claims = verifier.verify(&token).await.unwrap();
        assert!(claims.email_verified);
    }

    #[tokio::test]
    async fn rejects_expired_token() {
        let verifier = SupabaseTokenVerifier::for_tests(ISS, AUD, SECRET);
        let now = chrono::Utc::now().timestamp();
        let token = sign(serde_json::json!({
            "sub": "user-1",
            "iss": ISS,
            "aud": AUD,
            "exp": now - 120,
            "iat": now - 200,
        }));
        let err = verifier.verify(&token).await.unwrap_err();
        assert_eq!(err, SupabaseAuthError::Expired);
    }

    #[tokio::test]
    async fn rejects_wrong_audience() {
        let verifier = SupabaseTokenVerifier::for_tests(ISS, AUD, SECRET);
        let now = chrono::Utc::now().timestamp();
        let token = sign(serde_json::json!({
            "sub": "user-1",
            "iss": ISS,
            "aud": "other",
            "exp": now + 3600,
            "iat": now,
        }));
        let err = verifier.verify(&token).await.unwrap_err();
        assert!(matches!(
            err,
            SupabaseAuthError::InvalidClaims | SupabaseAuthError::InvalidToken
        ));
    }

    #[test]
    fn config_from_url_derives_issuer_and_default_aud() {
        let cfg = SupabaseConfig::from_url("https://abcd.supabase.co/", None, None).unwrap();
        assert_eq!(cfg.issuer, "https://abcd.supabase.co/auth/v1");
        assert_eq!(
            cfg.jwks_url,
            "https://abcd.supabase.co/auth/v1/.well-known/jwks.json"
        );
        assert_eq!(cfg.audience, "authenticated");
        assert!(cfg.jwt_secret.is_none());
    }

    #[test]
    fn parses_es256_jwk_like_current_supabase_projects() {
        let jwk = RawJwk {
            kid: Some("test-kid".into()),
            kty: "EC".into(),
            alg: Some("ES256".into()),
            n: None,
            e: None,
            crv: Some("P-256".into()),
            x: Some("AT_bG4Ab47fVzuIvjAqgA9nMqAPuEW3RSGKbXHQYa4c".into()),
            y: Some("UhskU6_xQ5PPCtub8fgwlzLmzQV6XAGaEaCmdgp1iE8".into()),
        };
        decoding_key_from_jwk(&jwk).expect("ES256 JWK must decode");
    }

    #[tokio::test]
    async fn strips_bearer_prefix() {
        let verifier = SupabaseTokenVerifier::for_tests(ISS, AUD, SECRET);
        let now = chrono::Utc::now().timestamp();
        let token = sign(serde_json::json!({
            "sub": "user-1",
            "iss": ISS,
            "aud": AUD,
            "exp": now + 3600,
            "iat": now,
        }));
        let claims = verifier.verify(&format!("Bearer {token}")).await.unwrap();
        assert_eq!(claims.sub, "user-1");
    }
}
