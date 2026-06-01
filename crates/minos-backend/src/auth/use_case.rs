use std::sync::{Arc, OnceLock};
use std::time::Duration;

use async_trait::async_trait;
use minos_domain::{DeviceId, DeviceRole};

use crate::auth::{
    jwt, passwords,
    rate_limit::RateLimiter,
    realtime_ticket::{RealtimeTicketConsumeError, RealtimeTicketStore},
};
use crate::error::BackendError;
use crate::store::{accounts, refresh_tokens, StoreHandle};

const ACCOUNTS_REPO_METRIC_LABEL: &str = "accounts_repo";
const REFRESH_TOKEN_REPO_METRIC_LABEL: &str = "refresh_token_repo";

#[derive(Clone)]
pub struct AuthRateLimits {
    login_per_email: Arc<RateLimiter>,
    login_per_ip: Arc<RateLimiter>,
    register_per_ip: Arc<RateLimiter>,
    refresh_per_acc: Arc<RateLimiter>,
}

impl Default for AuthRateLimits {
    fn default() -> Self {
        Self {
            login_per_email: Arc::new(RateLimiter::new(10, Duration::from_mins(1))),
            login_per_ip: Arc::new(RateLimiter::new(5, Duration::from_mins(1))),
            register_per_ip: Arc::new(RateLimiter::new(3, Duration::from_hours(1))),
            refresh_per_acc: Arc::new(RateLimiter::new(60, Duration::from_hours(1))),
        }
    }
}

impl AuthRateLimits {
    fn check_register_per_ip(&self, client_ip: &str) -> Result<(), AuthUseCaseError> {
        Self::check(&self.register_per_ip, client_ip)
    }

    fn check_login_per_ip(&self, client_ip: &str) -> Result<(), AuthUseCaseError> {
        Self::check(&self.login_per_ip, client_ip)
    }

    fn check_login_per_email(&self, email: &str) -> Result<(), AuthUseCaseError> {
        Self::check(&self.login_per_email, email)
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
    AccountNotFound,
    EmailTaken,
    InvalidCredentials,
    InvalidRefresh,
    Internal,
    RateLimited { retry_after_secs: u32 },
    WsTicketAccountMismatch,
    UnsupportedWsTicketRole,
    WeakPassword,
}

#[derive(Debug, Clone)]
pub struct AuthSession {
    pub account_id: String,
    pub email: String,
    pub access_token: String,
    pub refresh_token: String,
    pub expires_in: i64,
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
    async fn create(
        &self,
        email: &str,
        password_hash: &str,
    ) -> Result<accounts::AccountRow, BackendError>;

    async fn find_by_email(
        &self,
        email: &str,
    ) -> Result<Option<accounts::AccountRow>, BackendError>;

    async fn find_by_id(
        &self,
        account_id: &str,
    ) -> Result<Option<accounts::AccountRow>, BackendError>;

    async fn touch_last_login(&self, account_id: &str) -> Result<(), BackendError>;

    async fn set_password_hash(
        &self,
        account_id: &str,
        password_hash: &str,
    ) -> Result<(), BackendError>;
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
    async fn create(
        &self,
        email: &str,
        password_hash: &str,
    ) -> Result<accounts::AccountRow, BackendError> {
        let _db_timer = crate::telemetry::DbTimer::new(ACCOUNTS_REPO_METRIC_LABEL, "create");
        accounts::create(&self.store, email, password_hash).await
    }

    async fn find_by_email(
        &self,
        email: &str,
    ) -> Result<Option<accounts::AccountRow>, BackendError> {
        let _db_timer = crate::telemetry::DbTimer::new(ACCOUNTS_REPO_METRIC_LABEL, "find_by_email");
        accounts::find_by_email(&self.store, email).await
    }

    async fn find_by_id(
        &self,
        account_id: &str,
    ) -> Result<Option<accounts::AccountRow>, BackendError> {
        let _db_timer = crate::telemetry::DbTimer::new(ACCOUNTS_REPO_METRIC_LABEL, "find_by_id");
        accounts::find_by_id(&self.store, account_id).await
    }

    async fn touch_last_login(&self, account_id: &str) -> Result<(), BackendError> {
        let _db_timer =
            crate::telemetry::DbTimer::new(ACCOUNTS_REPO_METRIC_LABEL, "touch_last_login");
        accounts::touch_last_login(&self.store, account_id).await
    }

    async fn set_password_hash(
        &self,
        account_id: &str,
        password_hash: &str,
    ) -> Result<(), BackendError> {
        let _db_timer =
            crate::telemetry::DbTimer::new(ACCOUNTS_REPO_METRIC_LABEL, "set_password_hash");
        accounts::set_password_hash(&self.store, account_id, password_hash).await
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
}

impl AuthUseCase {
    #[must_use]
    pub fn new(store: impl Into<StoreHandle>, jwt_secret: String) -> Arc<Self> {
        Self::new_with_realtime_tickets(
            store,
            jwt_secret,
            Arc::new(RealtimeTicketStore::default()),
        )
    }

    #[must_use]
    pub fn new_with_realtime_tickets(
        store: impl Into<StoreHandle>,
        jwt_secret: String,
        realtime_tickets: Arc<RealtimeTicketStore>,
    ) -> Arc<Self> {
        let store = store.into();
        Self::with_repos(
            store.clone(),
            Arc::new(SqlAccountsRepo::new(store.clone())),
            Arc::new(SqlRefreshTokenRepo::new(store)),
            realtime_tickets,
            jwt_secret,
        )
    }

    #[must_use]
    fn with_repos(
        store: StoreHandle,
        accounts: Arc<dyn AccountsRepo>,
        refresh_tokens: Arc<dyn RefreshTokenRepo>,
        realtime_tickets: Arc<RealtimeTicketStore>,
        jwt_secret: String,
    ) -> Arc<Self> {
        Arc::new(Self {
            store,
            accounts,
            refresh_tokens,
            realtime_tickets,
            jwt_secret: Arc::new(jwt_secret),
            limits: AuthRateLimits::default(),
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

    pub async fn register(
        &self,
        device_id: DeviceId,
        email: &str,
        password: &str,
        client_ip: &str,
    ) -> Result<AuthSession, AuthUseCaseError> {
        let result = self
            .register_inner(device_id, email, password, client_ip)
            .await;
        crate::telemetry::record_auth_register(auth_outcome_label(&result));
        result
    }

    async fn register_inner(
        &self,
        device_id: DeviceId,
        email: &str,
        password: &str,
        client_ip: &str,
    ) -> Result<AuthSession, AuthUseCaseError> {
        self.limits.check_register_per_ip(client_ip)?;
        validate_password(password)?;

        let hash = passwords::hash(password)
            .map_err(|error| Self::log_internal("register.hash_password", error))?;
        let account = match self.accounts.create(email, &hash).await {
            Ok(account) => account,
            Err(BackendError::EmailTaken) => return Err(AuthUseCaseError::EmailTaken),
            Err(error) => return Err(Self::log_internal("register.create_account", error)),
        };

        self.bind_device_to_account(&device_id, &account.account_id)
            .await?;
        self.issue_auth_session(&account.account_id, &account.email, &device_id)
            .await
    }

    pub async fn login(
        &self,
        device_id: DeviceId,
        email: &str,
        password: &str,
        client_ip: &str,
    ) -> Result<AuthSession, AuthUseCaseError> {
        let result = self
            .login_inner(device_id, email, password, client_ip)
            .await;
        crate::telemetry::record_auth_login(auth_outcome_label(&result));
        result
    }

    async fn login_inner(
        &self,
        device_id: DeviceId,
        email: &str,
        password: &str,
        client_ip: &str,
    ) -> Result<AuthSession, AuthUseCaseError> {
        self.limits.check_login_per_ip(client_ip)?;
        self.limits.check_login_per_email(&email.to_lowercase())?;

        let account = match self.accounts.find_by_email(email).await {
            Ok(Some(account)) => account,
            Ok(None) => {
                let _ = passwords::verify(password, dummy_password_hash());
                return Err(AuthUseCaseError::InvalidCredentials);
            }
            Err(error) => return Err(Self::log_internal("login.find_by_email", error)),
        };
        let password_matches = passwords::verify(password, &account.password_hash)
            .map_err(|error| Self::log_internal("login.verify_password", error))?;
        if !password_matches {
            return Err(AuthUseCaseError::InvalidCredentials);
        }

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
                "failed to revoke stale refresh tokens for device before login"
            );
        }
        self.accounts
            .touch_last_login(&account.account_id)
            .await
            .map_err(|error| Self::log_internal("login.touch_last_login", error))?;
        self.bind_device_to_account(&device_id, &account.account_id)
            .await?;
        self.issue_auth_session(&account.account_id, &account.email, &device_id)
            .await
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

        match self.refresh_tokens.find_active(refresh_token).await {
            Ok(Some(row)) if row.account_id == account_id => {}
            Ok(Some(_)) | Ok(None) => return Ok(()),
            Err(error) => return Err(Self::log_internal("logout.find_active", error)),
        }

        self.refresh_tokens
            .revoke_one(refresh_token)
            .await
            .map_err(|error| Self::log_internal("logout.revoke_one", error))?;
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

        let existing_account_id = crate::store::devices::get_device(&self.store, device_id)
            .await
            .map_err(|error| Self::log_internal("ws_ticket.get_device", error))?
            .and_then(|row| row.account_id);
        match existing_account_id.as_deref() {
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
            None => self.bind_device_to_account(&device_id, account_id).await?,
            Some(_) => {}
        }

        self.issue_tracked_ws_ticket(account_id, device_id, device_role)
            .await
    }

    pub async fn issue_host_ws_ticket(
        &self,
        host_installation_id: DeviceId,
    ) -> Result<WsTicketSession, AuthUseCaseError> {
        self.issue_tracked_ws_ticket(
            &host_installation_id.to_string(),
            host_installation_id,
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

    pub async fn change_password(
        &self,
        account_id: &str,
        current_password: &str,
        new_password: &str,
    ) -> Result<(), AuthUseCaseError> {
        validate_password(new_password)?;
        self.limits.check_refresh_per_account(account_id)?;

        let account = match self.accounts.find_by_id(account_id).await {
            Ok(Some(account)) => account,
            Ok(None) => return Err(AuthUseCaseError::AccountNotFound),
            Err(error) => return Err(Self::log_internal("change_password.find_by_id", error)),
        };
        let current_password_matches = passwords::verify(current_password, &account.password_hash)
            .map_err(|error| Self::log_internal("change_password.verify_current", error))?;
        if !current_password_matches {
            return Err(AuthUseCaseError::InvalidCredentials);
        }

        let next_hash = passwords::hash(new_password)
            .map_err(|error| Self::log_internal("change_password.hash_new", error))?;
        self.accounts
            .set_password_hash(&account.account_id, &next_hash)
            .await
            .map_err(|error| Self::log_internal("change_password.set_hash", error))?;

        if let Err(error) = self
            .refresh_tokens
            .revoke_all_for_account(&account.account_id)
            .await
        {
            tracing::warn!(
                target: "minos_backend::auth",
                error = %error,
                account_id = %account.account_id,
                "failed to revoke active refresh tokens after password change"
            );
        }
        Ok(())
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
        })
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

fn validate_password(password: &str) -> Result<(), AuthUseCaseError> {
    if password.len() < 8 || password.chars().count() < 8 {
        return Err(AuthUseCaseError::WeakPassword);
    }
    Ok(())
}

fn dummy_password_hash() -> &'static str {
    static DUMMY_HASH: OnceLock<String> = OnceLock::new();
    DUMMY_HASH.get_or_init(|| {
        passwords::hash("dummy_for_constant_time_check_xxxxxxx")
            .expect("argon2id default params must hash a static string")
    })
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
        Err(AuthUseCaseError::AccountNotFound | AuthUseCaseError::InvalidCredentials) => {
            t::OUTCOME_UNAUTHORIZED
        }
        Err(AuthUseCaseError::InvalidRefresh) => t::OUTCOME_UNAUTHORIZED,
        Err(AuthUseCaseError::WsTicketAccountMismatch) => t::OUTCOME_UNAUTHORIZED,
        Err(AuthUseCaseError::EmailTaken) => t::OUTCOME_CONFLICT,
        Err(AuthUseCaseError::RateLimited { .. }) => t::OUTCOME_RATE_LIMITED,
        Err(AuthUseCaseError::WeakPassword | AuthUseCaseError::UnsupportedWsTicketRole) => {
            t::OUTCOME_INVALID
        }
        Err(AuthUseCaseError::Internal) => t::OUTCOME_ERROR,
    }
}
