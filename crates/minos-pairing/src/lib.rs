#![forbid(unsafe_code)]

pub mod state_machine;
pub mod store;
pub mod token;

pub use state_machine::*;
pub use store::*;
pub use token::*;

// UniFFI 0.31 per-crate scaffolding: every crate that carries `uniffi::*`
// derives must define `UniFfiTag` locally via `setup_scaffolding!()`; the
// derive expansions reference `crate::UniFfiTag`. Feature-gated so the
// non-UniFFI build path (plan-03 Dart/frb consumers) pays nothing.
#[cfg(feature = "uniffi")]
uniffi::setup_scaffolding!();

#[cfg(feature = "uniffi")]
mod uniffi_bridges {
    use chrono::{DateTime, Utc};
    use minos_domain::PairingToken;
    use std::time::SystemTime;
    use uuid::Uuid;

    // Type alias satisfies UniFFI 0.31's single-ident requirement on the
    // first argument of `custom_type!`. Trait-transparent — the generated
    // impls land on `DateTime<Utc>`.
    type DateTimeUtc = DateTime<Utc>;

    uniffi::custom_type!(Uuid, String, {
        remote,
        lower: |u| u.to_string(),
        try_lift: |s| Uuid::parse_str(&s).map_err(Into::into),
    });

    uniffi::custom_type!(DateTimeUtc, SystemTime, {
        remote,
        lower: |dt| dt.into(),
        try_lift: |st| Ok(st.into()),
    });

    // `DeviceId` is now registered in its home crate (`minos-domain`) with
    // blanket `impl<UT>` coverage, which already applies to this crate's
    // tag — no local registration needed here.
    uniffi::custom_type!(PairingToken, String, {
        remote,
        lower: |t| t.0,
        try_lift: |s| Ok(PairingToken(s)),
    });
}
