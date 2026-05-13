/// Domain Layer — Models
///
/// Immutable data classes representing the core business entities.
/// Domain models are independent of any framework, API format, or
/// persistence mechanism. They are the lingua franca between layers.
///
/// Rules:
///   - Domain models must NOT import from `infrastructure/`, `application/`,
///     or `presentation/`.
///   - They may import from `src/rust/api/minos.dart` only for shared
///     enums/types that are part of the domain contract (e.g. AgentName).
///   - All domain models should be immutable (use `@immutable` or sealed).
library;

export 'active_session.dart';
export 'agent_profile.dart';
export 'auth_state.dart';
export 'group_member.dart';
export 'minos_core_protocol.dart';
export 'minos_error_display.dart';
export 'social_message.dart';
