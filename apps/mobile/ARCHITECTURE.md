# Minos Mobile — Architecture

This document describes the layered architecture of the Minos mobile app,
following the [Flutter Architecture Best Practices](.agents/skills/flutter-apply-architecture-best-practices/SKILL.md).

## Directory Structure

```
lib/
├── main.dart                    # Entry point: init core, run app
├── architecture.dart            # Architecture overview (dartdoc)
│
├── domain/                      # Domain Layer (pure Dart)
│   ├── models.dart              # Barrel export for all domain models
│   ├── minos_core_protocol.dart # Abstract service contract (DI boundary)
│   ├── active_session.dart      # Agent session state machine
│   ├── auth_state.dart          # Auth lifecycle states
│   ├── agent_profile.dart       # Agent configuration model
│   ├── social_message.dart      # Chat message model
│   ├── group_member.dart        # Group membership model
│   └── minos_error_display.dart # Error presentation helpers
│
├── infrastructure/              # Data Layer — Services
│   ├── minos_core.dart          # Rust FFI bridge (implements MinosCoreProtocol)
│   ├── secure_pairing_store.dart# Keychain persistence
│   ├── social_cache_store.dart  # SQLite message cache
│   ├── agent_profile_store.dart # JSON file persistence
│   ├── app_paths.dart           # Platform path resolution
│   └── workspace_mru_store.dart # MRU workspace persistence
│
├── data/                        # Data Layer — Repositories (barrel exports)
│   ├── repositories/
│   │   └── repositories.dart    # Re-exports repository providers
│   └── services/
│       └── services.dart        # Re-exports infrastructure services
│
├── application/                 # Application Layer — ViewModels (Riverpod)
│   ├── minos_providers.dart     # Core data providers (repository role)
│   ├── auth_provider.dart       # Auth state controller
│   ├── active_session_provider.dart  # Agent session lifecycle
│   ├── thread_events_provider.dart   # Live event stream
│   ├── thread_list_provider.dart     # Thread list data
│   ├── project_providers.dart   # Project CRUD + selection
│   ├── social_providers.dart    # Social chat state management
│   ├── agent_profiles_provider.dart  # Agent profile CRUD
│   ├── group_agent_provider.dart     # Group agent bindings
│   ├── group_agent_dispatcher.dart   # Agent mention dispatch
│   ├── preferred_agent_provider.dart # Preferred agent selection
│   ├── log_records_provider.dart     # Debug log mirror
│   ├── request_trace_records_provider.dart # Request trace mirror
│   └── root_route_decision.dart # Navigation decision logic
│
├── presentation/                # UI Layer — Views (legacy path)
│   ├── app.dart                 # Root widget + theme + router
│   ├── error_feedback.dart      # Toast error helper
│   ├── pages/                   # Full-screen page widgets
│   │   ├── app_shell_page.dart  # Tab shell (Messages/Members/Profile)
│   │   ├── login_page.dart      # Auth surface
│   │   ├── thread_view_page.dart# Agent chat surface
│   │   ├── project_list_page.dart    # Project grid
│   │   ├── project_detail_page.dart  # Project detail
│   │   ├── agents_hub_page.dart # Agent management
│   │   ├── agent_start_page.dart# New agent session
│   │   ├── social_hub_page.dart # Friends & conversations
│   │   ├── social_chat_page.dart# Social chat
│   │   ├── group_members_page.dart   # Group member management
│   │   ├── pairing_page.dart    # QR pairing flow
│   │   ├── log_viewer_page.dart # Debug log viewer
│   │   └── permission_denied_page.dart
│   └── widgets/                 # Reusable widget components
│       ├── auth/                # Auth-specific widgets
│       ├── chat/                # Chat-specific widgets
│       ├── shimmer_box.dart     # Loading placeholder
│       ├── thread_list_tile.dart# Thread row widget
│       └── workspace_mru_chips.dart
│
├── ui/                          # UI Layer — Feature-organized (barrel exports)
│   ├── core/
│   │   └── widgets/widgets.dart # Shared widget exports
│   └── features/
│       ├── auth/auth.dart       # Auth feature barrel
│       ├── chat/chat.dart       # Chat feature barrel
│       ├── projects/projects.dart    # Projects feature barrel
│       ├── agents/agents.dart   # Agents feature barrel
│       ├── social/social.dart   # Social feature barrel
│       ├── pairing/pairing.dart # Pairing feature barrel
│       ├── debug/debug.dart     # Debug feature barrel
│       └── shell/shell.dart     # Shell feature barrel
│
└── src/rust/                    # Generated FRB code (do not edit)
```

## Layer Rules

### Domain Layer (`lib/domain/`)
- Pure Dart — no Flutter, no Riverpod, no infrastructure imports.
- Defines the `MinosCoreProtocol` abstract class (dependency inversion).
- All models are immutable value types.

### Data Layer (`lib/infrastructure/` + `lib/data/`)
- `infrastructure/` contains concrete service implementations.
- `MinosCore` implements `MinosCoreProtocol` — the only file that imports FRB.
- Stores handle persistence (keychain, SQLite, JSON files).
- `data/` provides barrel exports for clean imports from upper layers.

### Application Layer (`lib/application/`)
- Riverpod providers acting as ViewModels.
- Manages UI state, handles user interactions, orchestrates data flow.
- Depends on `domain/` protocols, never on `infrastructure/` directly.
- `minosCoreProvider` is the DI seam — overridden in `main()`.

### UI Layer (`lib/presentation/` + `lib/ui/`)
- `presentation/` contains the actual widget implementations.
- `ui/` provides feature-organized barrel exports for discoverability.
- Views are lean — delegate all logic to application-layer providers.
- Shared widgets live in `ui/core/widgets/`.

## Dependency Flow

```
UI → Application → Domain ← Infrastructure
         ↓                        ↑
    (reads providers)    (implements protocol)
```

The UI layer watches Riverpod providers from the Application layer.
The Application layer depends on the Domain protocol.
The Infrastructure layer implements the Domain protocol.
No circular dependencies exist between layers.

## Adding a New Feature

1. Define domain models in `lib/domain/` (if new entities are needed).
2. Add service methods to `MinosCoreProtocol` + `MinosCore` (if new API calls).
3. Create Riverpod providers in `lib/application/` (state management).
4. Build views in `lib/presentation/pages/` or `lib/presentation/widgets/`.
5. Add a barrel export in `lib/ui/features/<feature>/`.
6. Register any new providers in the dependency graph.
