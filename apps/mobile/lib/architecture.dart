/// # Minos Mobile — Architecture Overview
///
/// This application follows the recommended Flutter layered architecture
/// (UI → Application → Data → Domain) with strict separation of concerns.
///
/// Product spine: **conversation-first collaboration IM**. The shell is
/// Messages / Hosts / Account. Agent session transcript and project/session
/// product surfaces were removed from Mobile; bots participate via Hub
/// conversation membership and bubbles.
///
/// ## Layer Diagram
///
/// ```
/// ┌─────────────────────────────────────────────────────────┐
/// │  UI Layer (lib/ui/)                                     │
/// │  ├── core/widgets/       Shared reusable widgets        │
/// │  └── features/           Feature-grouped views          │
/// │       ├── auth/          Login / register               │
/// │       ├── messages/      Conversation inbox (primary)   │
/// │       ├── social/        Conversation chat / members    │
/// │       ├── hosts/         Linked hosts list              │
/// │       ├── account/       Profile / logout               │
/// │       ├── debug/         Log viewer & traces            │
/// │       └── shell/         Root navigation shell          │
/// ├─────────────────────────────────────────────────────────┤
/// │  Application Layer (lib/application/)                   │
/// │  Riverpod providers acting as ViewModels:               │
/// │  ├── auth_provider         Auth state machine           │
/// │  ├── minos_providers       Connection / hosts / presence│
/// │  ├── social_providers      Timeline + inbox + friends   │
/// │  ├── im_outbox_worker      Local IM outbox drain        │
/// │  ├── agent_profiles_provider  Local bot cache for compose│
/// │  ├── group_agent_provider  Conversation participants    │
/// │  ├── *_actions              User-triggered mutations    │
/// │  └── root_route_decision   Navigation logic             │
/// ├─────────────────────────────────────────────────────────┤
/// │  Data Layer (lib/data/ + lib/infrastructure/)           │
/// │  ├── repositories/       Single source of truth         │
/// │  │   ├── auth_repository     Auth / session IO          │
/// │  │   ├── runtime_repository  Pairing / host state       │
/// │  │   ├── hosts_repository    Linked hosts HTTP          │
/// │  │   ├── social_repository   Social remote + cache      │
/// │  │   ├── thread_repository   uiEvents + residual APIs   │
/// │  │   ├── agent_profile_repository  Device cache of bots │
/// │  │   └── group_agent_repository  Conversation agents    │
/// │  ├── services/           Service providers / wrappers   │
/// │  │   ├── minos_core_service     Core service override   │
/// │  │   ├── social_cache_store_service  SQLite provider    │
/// │  │   └── agent_profile_store_service  JSON cache        │
/// │  └── infrastructure/     Raw implementations           │
/// │       ├── minos_core       Rust FFI bridge (Hub CRUD)   │
/// │       ├── secure_pairing_store  Keychain persistence    │
/// │       ├── social_cache_store    SQLite message cache    │
/// │       ├── im_outbox_store       Outbox policy helpers   │
/// │       └── agent_profile_store   Local bot draft cache   │
/// ├─────────────────────────────────────────────────────────┤
/// │  Domain Layer (lib/domain/)                             │
/// │  ├── minos_core_protocol   Abstract service contract    │
/// │  ├── auth_state            Auth lifecycle states        │
/// │  ├── agent_profile         Bot body draft / cache model │
/// │  ├── social_message        Chat message model           │
/// │  ├── group_member          Group membership model       │
/// │  └── minos_error_display   Error presentation helpers   │
/// │  Note: Hub `agents` is bot identity SSOT; local JSON is │
/// │  cache/draft only.                                     │
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
/// 4. **Immutable domain models**: `AuthState`, `SocialChatMessage`,
///    `AgentProfile` are immutable value types.
///
/// 5. **Single source of truth**: Each piece of data has exactly one
///    authoritative provider. E.g. `conversationsProvider` is the single
///    source for the inbox list; UI reads it, never fetches directly.
///
/// 6. **IM send path**: Collaboration messages go through
///    `SocialConversation.sendMessage` → local outbox →
///    `sendChatMessage` (Account WS AppendMessage). No second product
///    send path on Mobile.
library;
