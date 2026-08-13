use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use minos_domain::{DeviceId, DeviceRole};

use crate::auth::{
    jwt,
    rate_limit::RateLimiter,
    realtime_ticket::{RealtimeTicketConsumeError, RealtimeTicketStore},
    supabase::{SupabaseAuthError, SupabaseTokenVerifier},
};
use crate::error::BackendError;
use crate::store::{accounts, refresh_tokens, StoreHandle};

const ACCOUNTS_REPO_METRIC_LABEL: &str = "accounts_repo";
const REFRESH_TOKEN_REPO_METRIC_LABEL: &str = "refresh_token_repo";

#[derive(Clone)]
pub struct AuthRateLimits {
    /// Shared bucket for Supabase exchange (account create / login via IdP).
    exchange_per_ip: Arc<RateLimiter>,
    refresh_per_acc: Arc<RateLimiter>,
}

impl Default for AuthRateLimits {
    fn default() -> Self {
        Self {
            exchange_per_ip: Arc::new(RateLimiter::new(3, Duration::from_hours(1))),
            refresh_per_acc: Arc::new(RateLimiter::new(60, Duration::from_hours(1))),
        }
    }
}

impl AuthRateLimits {
    fn check_exchange_per_ip(&self, client_ip: &str) -> Result<(), AuthUseCaseError> {
        Self::check(&self.exchange_per_ip, client_ip)
    }

    fn check_refresh_per_account(&self, account_id: &str) -> Result<(), AuthUseCaseError> {
        Self::check(&self.refresh_per_acc, account_id)
    }

    fn check(limiter: &RateLimiter, key: &str) -> Result<(), AuthUseCaseError> {
        limiter
            .check(key)
            .map_err(|retry_after_secs| AuthUseCaseError::RateLimited { retry_after_secs })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthUseCaseError {
    EmailTaken,
    InvalidRefresh,
    Internal,
    RateLimited {
        retry_after_secs: u32,
    },
    WsTicketAccountMismatch,
    UnsupportedWsTicketRole,
    /// Supabase exchange is not configured on this process.
    SupabaseNotConfigured,
    InvalidSupabaseToken,
    SupabaseTokenExpired,
    SupabaseTokenInvalid,
    IdpUnavailable,
    /// Verified email matches an account that already has a different sub.
    MergeConflict,
}

#[derive(Debug, Clone)]
pub struct AuthSession {
    pub account_id: String,
    pub email: String,
    pub access_token: String,
    pub refresh_token: String,
    pub expires_in: i64,
    /// Desktop login: opaque host token bound to `(account_id, device_id)`.
    /// Other account-client roles leave this unset.
    pub host_token: Option<String>,
}

#[derive(Debug, Clone)]
pub struct RefreshSession {
    pub access_token: String,
    pub refresh_token: String,
    pub expires_in: i64,
}

#[derive(Debug, Clone)]
pub struct WsTicketSession {
    pub ticket: String,
    pub expires_in: i64,
    pub device_id: String,
    pub device_role: DeviceRole,
}

#[async_trait]
trait RefreshTokenRepo: Send + Sync {
    async fn insert(
        &self,
        plaintext: &str,
        account_id: &str,
        device_id: &str,
    ) -> Result<refresh_tokens::RefreshTokenRow, BackendError>;

    async fn find_active(
        &self,
        plaintext: &str,
    ) -> Result<Option<refresh_tokens::RefreshTokenRow>, BackendError>;

    async fn find_any(
        &self,
        plaintext: &str,
    ) -> Result<Option<refresh_tokens::RefreshTokenRow>, BackendError>;

    async fn rotate(
        &self,
        old_plaintext: &str,
        new_plaintext: &str,
        account_id: &str,
        device_id: &str,
    ) -> Result<Option<refresh_tokens::RefreshTokenRow>, BackendError>;

    async fn revoke_one(&self, plaintext: &str) -> Result<(), BackendError>;

    async fn revoke_all_for_account(&self, account_id: &str) -> Result<u64, BackendError>;

    async fn revoke_all_for_device(&self, device_id: &str) -> Result<u64, BackendError>;
}

#[async_trait]
trait AccountsRepo: Send + Sync {
    async fn find_by_email(
        &self,
        email: &str,
    ) -> Result<Option<accounts::AccountRow>, BackendError>;

    async fn touch_last_login(&self, account_id: &str) -> Result<(), BackendError>;
}

struct SqlAccountsRepo {
    store: StoreHandle,
}

impl SqlAccountsRepo {
    fn new(store: StoreHandle) -> Self {
        Self { store }
    }
}

#[async_trait]
impl AccountsRepo for SqlAccountsRepo {
    async fn find_by_email(
        &self,
        email: &str,
    ) -> Result<Option<accounts::AccountRow>, BackendError> {
        let _db_timer = crate::telemetry::DbTimer::new(ACCOUNTS_REPO_METRIC_LABEL, "find_by_email");
        accounts::find_by_email(&self.store, email).await
    }

    async fn touch_last_login(&self, account_id: &str) -> Result<(), BackendError> {
        let _db_timer =
            crate::telemetry::DbTimer::new(ACCOUNTS_REPO_METRIC_LABEL, "touch_last_login");
        accounts::touch_last_login(&self.store, account_id).await
    }
}

struct SqlRefreshTokenRepo {
    store: StoreHandle,
}

impl SqlRefreshTokenRepo {
    fn new(store: StoreHandle) -> Self {
        Self { store }
    }
}

#[async_trait]
impl RefreshTokenRepo for SqlRefreshTokenRepo {
    async fn insert(
        &self,
        plaintext: &str,
        account_id: &str,
        device_id: &str,
    ) -> Result<refresh_tokens::RefreshTokenRow, BackendError> {
        let _db_timer = crate::telemetry::DbTimer::new(REFRESH_TOKEN_REPO_METRIC_LABEL, "insert");
        refresh_tokens::insert(&self.store, plaintext, account_id, device_id).await
    }

    async fn find_active(
        &self,
        plaintext: &str,
    ) -> Result<Option<refresh_tokens::RefreshTokenRow>, BackendError> {
        let _db_timer =
            crate::telemetry::DbTimer::new(REFRESH_TOKEN_REPO_METRIC_LABEL, "find_active");
        refresh_tokens::find_active(&self.store, plaintext).await
    }

    async fn find_any(
        &self,
        plaintext: &str,
    ) -> Result<Option<refresh_tokens::RefreshTokenRow>, BackendError> {
        let _db_timer = crate::telemetry::DbTimer::new(REFRESH_TOKEN_REPO_METRIC_LABEL, "find_any");
        refresh_tokens::find_any(&self.store, plaintext).await
    }

    async fn rotate(
        &self,
        old_plaintext: &str,
        new_plaintext: &str,
        account_id: &str,
        device_id: &str,
    ) -> Result<Option<refresh_tokens::RefreshTokenRow>, BackendError> {
        let _db_timer = crate::telemetry::DbTimer::new(REFRESH_TOKEN_REPO_METRIC_LABEL, "rotate");
        refresh_tokens::rotate(
            &self.store,
            old_plaintext,
            new_plaintext,
            account_id,
            device_id,
        )
        .await
    }

    async fn revoke_one(&self, plaintext: &str) -> Result<(), BackendError> {
        let _db_timer =
            crate::telemetry::DbTimer::new(REFRESH_TOKEN_REPO_METRIC_LABEL, "revoke_one");
        refresh_tokens::revoke_one(&self.store, plaintext).await
    }

    async fn revoke_all_for_account(&self, account_id: &str) -> Result<u64, BackendError> {
        let _db_timer = crate::telemetry::DbTimer::new(
            REFRESH_TOKEN_REPO_METRIC_LABEL,
            "revoke_all_for_account",
        );
        refresh_tokens::revoke_all_for_account(&self.store, account_id).await
    }

    async fn revoke_all_for_device(&self, device_id: &str) -> Result<u64, BackendError> {
        let _db_timer = crate::telemetry::DbTimer::new(
            REFRESH_TOKEN_REPO_METRIC_LABEL,
            "revoke_all_for_device",
        );
        refresh_tokens::revoke_all_for_device(&self.store, device_id).await
    }
}

pub struct AuthUseCase {
    store: StoreHandle,
    accounts: Arc<dyn AccountsRepo>,
    refresh_tokens: Arc<dyn RefreshTokenRepo>,
    realtime_tickets: Arc<RealtimeTicketStore>,
    jwt_secret: Arc<String>,
    limits: AuthRateLimits,
    /// When `None`, `supabase_exchange` returns [`AuthUseCaseError::SupabaseNotConfigured`].
    supabase: Option<Arc<SupabaseTokenVerifier>>,
}

impl AuthUseCase {
    #[must_use]
    pub fn new(store: impl Into<StoreHandle>, jwt_secret: String) -> Arc<Self> {
        Self::new_with_realtime_tickets(store, jwt_secret, Arc::new(RealtimeTicketStore::default()))
    }

    #[must_use]
    pub fn new_with_realtime_tickets(
        store: impl Into<StoreHandle>,
        jwt_secret: String,
        realtime_tickets: Arc<RealtimeTicketStore>,
    ) -> Arc<Self> {
        Self::new_with_realtime_tickets_and_supabase(store, jwt_secret, realtime_tickets, None)
    }

    #[must_use]
    pub fn new_with_realtime_tickets_and_supabase(
        store: impl Into<StoreHandle>,
        jwt_secret: String,
        realtime_tickets: Arc<RealtimeTicketStore>,
        supabase: Option<Arc<SupabaseTokenVerifier>>,
    ) -> Arc<Self> {
        let store = store.into();
        Self::with_repos(
            store.clone(),
            Arc::new(SqlAccountsRepo::new(store.clone())),
            Arc::new(SqlRefreshTokenRepo::new(store)),
            realtime_tickets,
            jwt_secret,
            supabase,
        )
    }

    #[must_use]
    fn with_repos(
        store: StoreHandle,
        accounts: Arc<dyn AccountsRepo>,
        refresh_tokens: Arc<dyn RefreshTokenRepo>,
        realtime_tickets: Arc<RealtimeTicketStore>,
        jwt_secret: String,
        supabase: Option<Arc<SupabaseTokenVerifier>>,
    ) -> Arc<Self> {
        Arc::new(Self {
            store,
            accounts,
            refresh_tokens,
            realtime_tickets,
            jwt_secret: Arc::new(jwt_secret),
            limits: AuthRateLimits::default(),
            supabase,
        })
    }

    #[must_use]
    pub fn jwt_secret(&self) -> &[u8] {
        self.jwt_secret.as_bytes()
    }

    pub async fn consume_ws_ticket(
        &self,
        claims: &jwt::WsTicketClaims,
    ) -> Result<(), RealtimeTicketConsumeError> {
        self.realtime_tickets
            .consume(claims, chrono::Utc::now().timestamp())
            .await
    }

    /// Exchange a Supabase access token for a Minos session.
    ///
    /// Does **not** go through device-secret `authenticate()`; the caller
    /// supplies a validated `device_id` + account-client `role` and this
    /// method ensures the device row is bound to the upserted account.
    pub async fn supabase_exchange(
        &self,
        device_id: DeviceId,
        device_role: DeviceRole,
        device_name: Option<&str>,
        supabase_jwt: &str,
        client_ip: &str,
    ) -> Result<AuthSession, AuthUseCaseError> {
        let result = self
            .supabase_exchange_inner(device_id, device_role, device_name, supabase_jwt, client_ip)
            .await;
        crate::telemetry::record_auth_supabase_exchange(auth_outcome_label(&result));
        result
    }

    async fn supabase_exchange_inner(
        &self,
        device_id: DeviceId,
        device_role: DeviceRole,
        device_name: Option<&str>,
        supabase_jwt: &str,
        client_ip: &str,
    ) -> Result<AuthSession, AuthUseCaseError> {
        if !device_role.is_account_client() {
            return Err(AuthUseCaseError::UnsupportedWsTicketRole);
        }
        // Same bucket as register: limit account-creation style abuse.
        self.limits.check_exchange_per_ip(client_ip)?;

        let verifier = self
            .supabase
            .as_ref()
            .ok_or(AuthUseCaseError::SupabaseNotConfigured)?;

        let claims = match verifier.verify(supabase_jwt).await {
            Ok(claims) => claims,
            Err(SupabaseAuthError::Expired) => {
                return Err(AuthUseCaseError::SupabaseTokenExpired);
            }
            Err(SupabaseAuthError::InvalidClaims) => {
                return Err(AuthUseCaseError::SupabaseTokenInvalid);
            }
            Err(SupabaseAuthError::InvalidToken) => {
                return Err(AuthUseCaseError::InvalidSupabaseToken);
            }
            Err(SupabaseAuthError::IdpUnavailable(msg)) => {
                tracing::warn!(
                    target: "minos_backend::auth",
                    error = %msg,
                    "supabase JWKS unavailable"
                );
                return Err(AuthUseCaseError::IdpUnavailable);
            }
        };

        let account = self.upsert_account_from_supabase(&claims).await?;

        let device_id_string = device_id.to_string();
        if let Err(error) = self
            .refresh_tokens
            .revoke_all_for_device(&device_id_string)
            .await
        {
            tracing::warn!(
                target: "minos_backend::auth",
                error = %error,
                device_id = %device_id,
                "failed to revoke stale refresh tokens for device before supabase exchange"
            );
        }

        self.accounts
            .touch_last_login(&account.account_id)
            .await
            .map_err(|error| Self::log_internal("supabase.touch_last_login", error))?;

        self.ensure_device_for_account(&device_id, device_role, device_name, &account.account_id)
            .await?;

        let mut session = self
            .issue_auth_session(&account.account_id, &account.email, &device_id)
            .await?;
        if device_role == DeviceRole::DesktopConsole {
            session.host_token = Some(
                self.issue_desktop_host_token(&device_id, &account.account_id)
                    .await?,
            );
        }
        Ok(session)
    }

    async fn upsert_account_from_supabase(
        &self,
        claims: &crate::auth::supabase::SupabaseClaims,
    ) -> Result<accounts::AccountRow, AuthUseCaseError> {
        // 1) Already linked by sub.
        match accounts::find_by_supabase_sub(&self.store, &claims.sub).await {
            Ok(Some(account)) => return Ok(account),
            Ok(None) => {}
            Err(error) => {
                return Err(Self::log_internal("supabase.find_by_sub", error));
            }
        }

        // 2) Verified email match → bind sub to existing unbound account.
        if claims.email_verified {
            if let Some(email) = claims.email.as_deref() {
                match self.accounts.find_by_email(email).await {
                    Ok(Some(existing)) => match existing.supabase_sub.as_deref() {
                        None => {
                            accounts::bind_supabase_sub(
                                &self.store,
                                &existing.account_id,
                                &claims.sub,
                            )
                            .await
                            .map_err(|error| Self::log_internal("supabase.bind_sub", error))?;
                            return accounts::find_by_id(&self.store, &existing.account_id)
                                .await
                                .map_err(|error| {
                                    Self::log_internal("supabase.reload_bound", error)
                                })?
                                .ok_or(AuthUseCaseError::Internal);
                        }
                        Some(bound) if bound == claims.sub => return Ok(existing),
                        Some(_) => return Err(AuthUseCaseError::MergeConflict),
                    },
                    Ok(None) => {}
                    Err(error) => {
                        return Err(Self::log_internal("supabase.find_by_email", error));
                    }
                }
            }
        }

        // 3) Create a new OAuth-linked account.
        let email = claims.email.clone().unwrap_or_else(|| {
            // Email is NOT NULL in SQLite; use a stable synthetic address when
            // the IdP omitted email (rare for Google / magic link).
            format!("{}@oauth.minos.local", claims.sub)
        });
        match accounts::create_with_supabase_sub(&self.store, &email, &claims.sub).await {
            Ok(account) => Ok(account),
            Err(BackendError::EmailTaken) => {
                // Race: email taken between find and create. If verified, try bind.
                if claims.email_verified {
                    if let Some(email) = claims.email.as_deref() {
                        if let Ok(Some(existing)) = self.accounts.find_by_email(email).await {
                            if existing.supabase_sub.is_none() {
                                accounts::bind_supabase_sub(
                                    &self.store,
                                    &existing.account_id,
                                    &claims.sub,
                                )
                                .await
                                .map_err(|error| {
                                    Self::log_internal("supabase.bind_sub_race", error)
                                })?;
                                return accounts::find_by_id(&self.store, &existing.account_id)
                                    .await
                                    .map_err(|error| {
                                        Self::log_internal("supabase.reload_bound_race", error)
                                    })?
                                    .ok_or(AuthUseCaseError::Internal);
                            }
                            if existing.supabase_sub.as_deref() == Some(claims.sub.as_str()) {
                                return Ok(existing);
                            }
                            return Err(AuthUseCaseError::MergeConflict);
                        }
                    }
                }
                Err(AuthUseCaseError::EmailTaken)
            }
            Err(error) => Err(Self::log_internal("supabase.create_account", error)),
        }
    }

    /// Ensure a client installation row exists and is bound to `account_id`.
    ///
    /// Uses [`insert_client_for_account`] so Postgres CHECK
    /// (`account_id IS NOT NULL` for mobile/browser/desktop) is satisfied in
    /// one insert — never create a null-account client row then patch later.
    async fn ensure_device_for_account(
        &self,
        device_id: &DeviceId,
        device_role: DeviceRole,
        device_name: Option<&str>,
        account_id: &str,
    ) -> Result<(), AuthUseCaseError> {
        if !device_role.is_account_client() {
            return Err(AuthUseCaseError::UnsupportedWsTicketRole);
        }
        let display_name = device_name
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or("unnamed");
        let existing = crate::store::devices::get_device(&self.store, *device_id)
            .await
            .map_err(|error| Self::log_internal("auth.get_device", error))?;

        match existing {
            None => {
                let now = chrono::Utc::now().timestamp_millis();
                crate::store::devices::insert_client_for_account(
                    &self.store,
                    *device_id,
                    display_name,
                    device_role,
                    account_id,
                    now,
                )
                .await
                .map_err(|error| Self::log_internal("auth.insert_client_for_account", error))?;
                // Fresh bind: still invalidate peer caches for the account.
                crate::ingest::invalidate_peer_targets_for_account(&self.store, account_id)
                    .await
                    .map_err(|error| {
                        Self::log_internal("auth.invalidate_peer_targets_new", error)
                    })?;
                Ok(())
            }
            Some(row) => {
                if row.role != device_role {
                    return Err(AuthUseCaseError::UnsupportedWsTicketRole);
                }
                if let Some(name) = device_name
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .filter(|s| *s != row.display_name.as_str())
                {
                    if let Err(error) =
                        crate::store::devices::set_display_name(&self.store, device_id, name).await
                    {
                        tracing::warn!(
                            target: "minos_backend::auth",
                            error = %error,
                            device_id = %device_id,
                            "failed to update installation display_name"
                        );
                    }
                }
                self.bind_device_to_account(device_id, account_id).await
            }
        }
    }

    pub async fn refresh(
        &self,
        device_id: DeviceId,
        refresh_token: &str,
    ) -> Result<RefreshSession, AuthUseCaseError> {
        let result = self.refresh_inner(device_id, refresh_token).await;
        crate::telemetry::record_auth_refresh(auth_outcome_label(&result));
        result
    }

    async fn refresh_inner(
        &self,
        device_id: DeviceId,
        refresh_token: &str,
    ) -> Result<RefreshSession, AuthUseCaseError> {
        let row = match self.refresh_tokens.find_active(refresh_token).await {
            Ok(Some(row)) => row,
            Ok(None) => return self.invalid_refresh(refresh_token).await,
            Err(error) => return Err(Self::log_internal("refresh.find_active", error)),
        };
        if row.device_id != device_id.to_string() {
            return Err(AuthUseCaseError::InvalidRefresh);
        }

        self.limits.check_refresh_per_account(&row.account_id)?;

        let new_refresh = refresh_tokens::generate_plaintext();
        match self
            .refresh_tokens
            .rotate(refresh_token, &new_refresh, &row.account_id, &row.device_id)
            .await
        {
            Ok(Some(_)) => {}
            Ok(None) => return Err(AuthUseCaseError::InvalidRefresh),
            Err(error) => return Err(Self::log_internal("refresh.rotate", error)),
        }

        let access_token = jwt::sign(self.jwt_secret(), &row.account_id, &row.device_id)
            .map_err(|error| Self::log_internal("refresh.sign_access", error))?;

        Ok(RefreshSession {
            access_token,
            refresh_token: new_refresh,
            expires_in: jwt::ACCESS_TTL_SECS,
        })
    }

    async fn invalid_refresh(
        &self,
        refresh_token: &str,
    ) -> Result<RefreshSession, AuthUseCaseError> {
        let row = match self.refresh_tokens.find_any(refresh_token).await {
            Ok(row) => row,
            Err(error) => return Err(Self::log_internal("refresh.find_any", error)),
        };

        if let Some(row) = row.filter(|row| row.revoked_at.is_some()) {
            crate::telemetry::record_auth_refresh_reuse();
            tracing::warn!(
                target: "minos_backend::auth",
                account_id = %row.account_id,
                device_id = %row.device_id,
                "revoked refresh token reused; revoking active tokens for account"
            );
            if let Err(error) = self
                .refresh_tokens
                .revoke_all_for_account(&row.account_id)
                .await
            {
                tracing::warn!(
                    target: "minos_backend::auth",
                    error = %error,
                    account_id = %row.account_id,
                    "failed to revoke active refresh tokens after reuse detection"
                );
            }
        }

        Err(AuthUseCaseError::InvalidRefresh)
    }

    pub async fn logout(
        &self,
        account_id: &str,
        refresh_token: &str,
    ) -> Result<(), AuthUseCaseError> {
        let result = self.logout_inner(account_id, refresh_token).await;
        crate::telemetry::record_auth_logout(auth_outcome_label(&result));
        result
    }

    async fn logout_inner(
        &self,
        account_id: &str,
        refresh_token: &str,
    ) -> Result<(), AuthUseCaseError> {
        self.limits.check_refresh_per_account(account_id)?;

        let device_id = match self.refresh_tokens.find_active(refresh_token).await {
            Ok(Some(row)) if row.account_id == account_id => row.device_id,
            Ok(Some(_)) | Ok(None) => return Ok(()),
            Err(error) => return Err(Self::log_internal("logout.find_active", error)),
        };

        self.refresh_tokens
            .revoke_one(refresh_token)
            .await
            .map_err(|error| Self::log_internal("logout.revoke_one", error))?;
        if let Ok(device_id) = uuid::Uuid::parse_str(&device_id).map(DeviceId) {
            let now = chrono::Utc::now().timestamp_millis();
            let _ =
                crate::store::host_tokens::revoke_all_for_host(&self.store, device_id, now).await;
        }
        Ok(())
    }

    pub async fn issue_ws_ticket(
        &self,
        account_id: &str,
        device_id: DeviceId,
        device_role: DeviceRole,
    ) -> Result<WsTicketSession, AuthUseCaseError> {
        if !device_role.is_account_client() {
            return Err(AuthUseCaseError::UnsupportedWsTicketRole);
        }

        let existing = crate::store::devices::get_device(&self.store, device_id)
            .await
            .map_err(|error| Self::log_internal("ws_ticket.get_device", error))?;
        match existing.as_ref().and_then(|row| row.account_id.as_deref()) {
            Some(bound_account_id) if bound_account_id != account_id => {
                tracing::warn!(
                    target: "minos_backend::auth",
                    device_id = %device_id,
                    bound_account_id,
                    requested_account_id = account_id,
                    "refusing ws ticket for device bound to a different account"
                );
                return Err(AuthUseCaseError::WsTicketAccountMismatch);
            }
            Some(_) => {}
            None => {
                // Insert or re-bind with account_id in a CHECK-compliant way.
                self.ensure_device_for_account(&device_id, device_role, None, account_id)
                    .await?;
            }
        }

        self.issue_tracked_ws_ticket(account_id, device_id, device_role)
            .await
    }

    pub async fn issue_host_ws_ticket(
        &self,
        host_device_id: DeviceId,
    ) -> Result<WsTicketSession, AuthUseCaseError> {
        self.issue_tracked_ws_ticket(
            &host_device_id.to_string(),
            host_device_id,
            DeviceRole::AgentHost,
        )
        .await
    }

    async fn issue_tracked_ws_ticket(
        &self,
        subject: &str,
        device_id: DeviceId,
        device_role: DeviceRole,
    ) -> Result<WsTicketSession, AuthUseCaseError> {
        let device_id_string = device_id.to_string();
        let ticket =
            jwt::sign_ws_ticket(self.jwt_secret(), subject, &device_id_string, device_role)
                .map_err(|error| Self::log_internal("ws_ticket.sign", error))?;
        let claims = jwt::verify_ws_ticket(self.jwt_secret(), &ticket)
            .map_err(|error| Self::log_internal("ws_ticket.verify_after_sign", error))?;
        self.realtime_tickets
            .register(&claims)
            .await
            .map_err(|error| Self::log_internal("ws_ticket.register", error))?;

        Ok(WsTicketSession {
            ticket,
            expires_in: jwt::WS_TICKET_TTL_SECS,
            device_id: device_id_string,
            device_role,
        })
    }

    async fn bind_device_to_account(
        &self,
        device_id: &DeviceId,
        account_id: &str,
    ) -> Result<(), AuthUseCaseError> {
        let previous_account_id = crate::store::devices::get_device(&self.store, *device_id)
            .await
            .map_err(|error| Self::log_internal("bind_device.get_device", error))?
            .and_then(|device| device.account_id);

        crate::store::devices::set_account_id(&self.store, device_id, account_id)
            .await
            .map_err(|error| Self::log_internal("bind_device.set_account_id", error))?;

        if let Some(previous_account_id) = previous_account_id.as_deref() {
            crate::ingest::invalidate_peer_targets_for_account(&self.store, previous_account_id)
                .await
                .map_err(|error| Self::log_internal("bind_device.invalidate_previous", error))?;
        }
        if previous_account_id.as_deref() != Some(account_id) {
            crate::ingest::invalidate_peer_targets_for_account(&self.store, account_id)
                .await
                .map_err(|error| Self::log_internal("bind_device.invalidate_current", error))?;
        }

        Ok(())
    }

    async fn issue_auth_session(
        &self,
        account_id: &str,
        email: &str,
        device_id: &DeviceId,
    ) -> Result<AuthSession, AuthUseCaseError> {
        let device_id_string = device_id.to_string();
        let access_token = jwt::sign(self.jwt_secret(), account_id, &device_id_string)
            .map_err(|error| Self::log_internal("issue_auth_session.sign_access", error))?;
        let refresh_token = refresh_tokens::generate_plaintext();
        self.refresh_tokens
            .insert(&refresh_token, account_id, &device_id_string)
            .await
            .map_err(|error| Self::log_internal("issue_auth_session.insert_refresh", error))?;

        Ok(AuthSession {
            account_id: account_id.to_string(),
            email: email.to_string(),
            access_token,
            refresh_token,
            expires_in: jwt::ACCESS_TTL_SECS,
            host_token: None,
        })
    }

    /// Bind this Desktop DeviceId as the account's Host and mint a host_token.
    ///
    /// Signing into Desktop owns this Mac: a prior account's link on the same
    /// DeviceId is replaced.
    async fn issue_desktop_host_token(
        &self,
        device_id: &DeviceId,
        account_id: &str,
    ) -> Result<String, AuthUseCaseError> {
        let svc = crate::host_link::HostLinkService::new(self.store.clone());
        match svc
            .link_host(*device_id, account_id, *device_id, Some("This Mac"))
            .await
        {
            Ok(outcome) => Ok(outcome.host_installation_token),
            Err(crate::host_link::HostLinkError::HostLinkedElsewhere { .. }) => {
                let links =
                    crate::store::host_links::list_accounts_for_host(&self.store, *device_id)
                        .await
                        .map_err(|error| Self::log_internal("desktop_host.list_links", error))?;
                let now = chrono::Utc::now().timestamp_millis();
                for link in links {
                    let _ = crate::store::host_links::delete_pair(
                        &self.store,
                        link.host_device_id,
                        &link.mobile_account_id,
                    )
                    .await;
                }
                let _ =
                    crate::store::host_tokens::revoke_all_for_host(&self.store, *device_id, now)
                        .await;
                svc.link_host(*device_id, account_id, *device_id, Some("This Mac"))
                    .await
                    .map(|outcome| outcome.host_installation_token)
                    .map_err(|error| {
                        Self::log_internal("desktop_host.relink", format!("{error:?}"))
                    })
            }
            Err(error) => Err(Self::log_internal(
                "desktop_host.link",
                format!("{error:?}"),
            )),
        }
    }

    fn log_internal(operation: &'static str, error: impl std::fmt::Display) -> AuthUseCaseError {
        tracing::warn!(
            target: "minos_backend::auth",
            operation,
            error = %error,
            "auth use case failed"
        );
        AuthUseCaseError::Internal
    }
}

/// Map an `AuthUseCase` result onto a stable counter label.
///
/// Mirrors the `outcome` convention in [`crate::telemetry`]: every
/// label value is one of the published `OUTCOME_*` constants so
/// dashboards can rely on bounded cardinality.
fn auth_outcome_label<T>(result: &Result<T, AuthUseCaseError>) -> &'static str {
    use crate::telemetry as t;
    match result {
        Ok(_) => t::OUTCOME_OK,
        Err(
            AuthUseCaseError::InvalidRefresh
            | AuthUseCaseError::WsTicketAccountMismatch
            | AuthUseCaseError::InvalidSupabaseToken
            | AuthUseCaseError::SupabaseTokenExpired
            | AuthUseCaseError::SupabaseTokenInvalid
            | AuthUseCaseError::SupabaseNotConfigured,
        ) => t::OUTCOME_UNAUTHORIZED,
        Err(AuthUseCaseError::EmailTaken | AuthUseCaseError::MergeConflict) => t::OUTCOME_CONFLICT,
        Err(AuthUseCaseError::RateLimited { .. }) => t::OUTCOME_RATE_LIMITED,
        Err(AuthUseCaseError::UnsupportedWsTicketRole) => t::OUTCOME_INVALID,
        Err(AuthUseCaseError::IdpUnavailable | AuthUseCaseError::Internal) => t::OUTCOME_ERROR,
    }
}
