//! Notifications subsystem: push token management, preferences, channels,
//! decision engine, and push fanout.

pub mod channels;
pub mod decision;
pub mod preferences;
pub mod use_case;

pub use use_case::{
    DispatchOutcome, NotificationError, NotificationService, PushTokenDto, RegisterTokenInput,
    UnregisterTokenInput, UpdatePreferencesInput,
};
pub use preferences::NotificationPreferences;
pub use decision::{Decision, DecisionReason};
