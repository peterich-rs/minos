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
├── data/
│   ├── repositories/           # Concrete repository providers + adapters
│   └── services/               # Concrete service/store providers
│
├── ui/                         # UI Layer — Feature-organized implementation
│   ├── core/
│   │   └── widgets/widgets.dart # Shared widget exports
│   └── features/
│       ├── auth/                # Auth screens/widgets + barrel
│       ├── chat/                # Agent thread screens/widgets + barrel
│       ├── projects/            # Project screens/widgets + barrel
│       ├── agents/              # Agent management screens/widgets + barrel
│       ├── social/              # Social screens/widgets + barrel
│       ├── pairing/             # Pairing screens/widgets + barrel
│       ├── debug/               # Debug screens/widgets + barrel
│       └── shell/               # Root app shell, router, navigation
│
└── src/rust/                    # Generated FRB code (do not edit)
```

## Layer Rules

### Domain Layer (`lib/domain/`)
- Pure Dart — no Flutter, no Riverpod, no infrastructure imports.
- Defines the `MinosCoreProtocol` abstract class (dependency inversion).
- All models are immutable value types.

### Data Layer (`lib/data/` + `lib/infrastructure/`)
- `data/repositories/` contains the concrete repository interfaces consumed by `application/`.
- `data/services/` contains provider-backed service/store access used by repositories.
- `infrastructure/` contains raw concrete implementations such as `MinosCore` and persistence stores.
- `MinosCore` implements `MinosCoreProtocol`; FRB remains isolated to infrastructure/generated code.

### Application Layer (`lib/application/`)
- Riverpod providers acting as ViewModels.
- Manages UI state, handles user interactions, orchestrates data flow.
- Depends on `domain/` models and `data/repositories/`, never on `ui/` or `infrastructure/` directly.
- Root dependency injection happens in `main()` by overriding `minosCoreServiceProvider`.

### UI Layer (`lib/ui/`)
- `ui/` contains the actual widget implementations and feature barrels.
- Views are lean — delegate all logic to application-layer providers.
- Shared widgets live in `ui/core/widgets/`.

## Dependency Flow

```
UI → Application → Data/Repositories → Data/Services → Infrastructure
          \_________________ Domain models / protocols _________________/
```

The UI layer watches Riverpod providers from the Application layer.
The Application layer depends on repositories instead of concrete services.
Repositories coordinate service/store access while preserving domain-facing APIs.
The Infrastructure layer implements domain protocols and low-level persistence.
No circular dependencies exist between layers.

## Adding a New Feature

1. Define domain models in `lib/domain/` (if new entities are needed).
2. Extend repositories/services and `MinosCoreProtocol` + `MinosCore` when new API calls are required.
3. Create Riverpod providers or action facades in `lib/application/`.
4. Build screens/widgets in `lib/ui/features/<feature>/`.
5. Export the feature surface from `lib/ui/features/<feature>/<feature>.dart`.
6. Register provider overrides or routing changes in the shell when needed.
