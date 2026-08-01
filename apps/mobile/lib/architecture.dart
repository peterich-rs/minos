/// # Minos Mobile — Architecture Overview
///
/// This application follows the recommended Flutter layered architecture
/// (UI → Application → Data → Domain) with strict separation of concerns.
///
/// ## Layer Diagram
///
/// ```
/// ┌─────────────────────────────────────────────────────────┐
/// │  UI Layer (lib/ui/)                                     │
/// │  ├── core/widgets/       Shared reusable widgets        │
/// │  └── features/           Feature-grouped views          │
/// │       ├── auth/          Login / register               │
/// │       ├── chat/          Agent session chat              │
/// │       ├── projects/      Project CRUD + sessions         │
/// │       ├── agents/        Agent profile management       │
/// │       ├── social/        Friends & conversations        │
/// /// │       ├── debug/         Log viewer & traces            │
/// │       └── shell/         Root navigation shell          │
/// ├─────────────────────────────────────────────────────────┤
/// │  Application Layer (lib/application/)                   │
/// │  Riverpod providers acting as ViewModels:               │
/// │  ├── auth_provider         Auth state machine           │
/// │  ├── active_session_provider  Agent session lifecycle   │
/// │  ├── thread_events_provider   Live event stream         │
/// │  ├── minos_providers      Pairing / runtime state       │
/// │  ├── project_providers     Project CRUD + selection     │
/// │  ├── social_providers      Chat, friends, conversations │
/// │  ├── agent_profiles_provider  Profile CRUD              │
/// │  ├── *_actions            User-triggered mutations      │
/// │  └── root_route_decision   Navigation logic             │
/// ├─────────────────────────────────────────────────────────┤
/// │  Data Layer (lib/data/ + lib/infrastructure/)           │
/// │  ├── repositories/       Single source of truth         │
/// │  │   ├── auth_repository     Auth / session IO          │
/// │  │   ├── runtime_repository  Pairing / host state       │
/// │  │   ├── project_repository  Project + session list      │
/// │  │   ├── thread_repository   Thread event / send IO     │
/// │  │   ├── social_repository   Social remote + cache      │
/// │  │   ├── agent_profile_repository  Local profile store  │
/// │  │   └── group_agent_repository  Conversation agents    │
/// │  ├── services/           Service providers / wrappers   │
/// │  │   ├── minos_core_service     Core service override   │
/// │  │   ├── social_cache_store_service  SQLite provider    │
/// │  │   └── agent_profile_store_service  JSON store        │
/// │  └── infrastructure/     Raw implementations           │
/// │       ├── minos_core       Rust FFI bridge              │
/// │       ├── secure_pairing_store  Keychain persistence    │
/// │       ├── social_cache_store    SQLite message cache    │
/// │       └── agent_profile_store   JSON file persistence   │
/// ├─────────────────────────────────────────────────────────┤
/// │  Domain Layer (lib/domain/)                             │
/// │  ├── minos_core_protocol   Abstract service contract    │
/// │  ├── active_session        Session state machine        │
/// │  ├── auth_state            Auth lifecycle states        │
/// │  ├── agent_profile         Agent configuration model    │
/// │  ├── social_message        Chat message model           │
/// │  ├── group_member          Group membership model       │
/// │  └── minos_error_display   Error presentation helpers   │
/// └─────────────────────────────────────────────────────────┘
/// ```
///
/// ## Key Principles
///
/// 1. **Strict layer boundaries**: UI never imports from `infrastructure/`
///    directly. All user-triggered reads and writes go through
///    `application/` providers / actions which depend on repositories.
///
/// 2. **Dependency inversion**: The `MinosCoreProtocol` abstract class in
///    `domain/` defines the contract. `MinosCore` in `infrastructure/`
///    implements it. `data/services` exposes the concrete service provider,
///    `data/repositories` wrap it, and `application/` consumes repositories.
///
/// 3. **Feature-based UI organization**: Each feature owns its views and
///    feature widgets and references its view-model providers. Shared
///    widgets live in `ui/core/widgets/`.
///
/// 4. **Immutable domain models**: `ActiveSession`, `AuthState`,
///    `SocialChatMessage`, `AgentProfile` are all immutable value types.
///
/// 5. **Single source of truth**: Each piece of data has exactly one
///    authoritative provider. E.g. `projectListProvider` is the single
///    source for the project list; UI reads it, never fetches directly.
library;
