//! Notifications subsystem: push token management, preferences, channels,
//! decision engine, and push fanout.

pub mod channels;
pub mod decision;
pub mod preferences;
pub mod use_case;

pub use decision::{AccountPresence, Decision, DecisionInput, DecisionReason};
pub use preferences::NotificationPreferences;
pub use use_case::{
    DispatchOutcome, NotificationError, NotificationService, OfflinePresence, PresencePort,
    PushTokenDto, RegisterTokenInput, UnregisterTokenInput, UpdatePreferencesInput,
};
