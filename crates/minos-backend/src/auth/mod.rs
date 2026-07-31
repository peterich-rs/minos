//! Account-auth (bearer-token) rail. Coexists with the device-secret
//! rail (`crate::http::auth`). Spec §5.3–5.4.

pub mod bearer;
pub mod host_bootstrap;
pub mod host_installation;
pub mod jwt;
pub mod passwords;
pub mod rate_limit;
pub mod realtime_ticket;
pub mod supabase;
pub mod use_case;
