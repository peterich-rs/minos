# Minos Mobile — Architecture

This document describes the layered architecture of the Minos mobile app,
following the [Flutter Architecture Best Practices](.agents/skills/flutter-apply-architecture-best-practices/SKILL.md).

Product spine: **conversation-first collaboration IM**. Shell tabs are
Messages / Hosts / Account. Agent session transcript, projects, sessions
inbox, and Agents Hub were removed from the Mobile product surface.

## Directory Structure

```
lib/
├── main.dart                    # Entry point: init core, run app
├── architecture.dart            # Architecture overview (dartdoc)
│
├── domain/                      # Domain Layer (pure Dart)
│   ├── models.dart              # Barrel export for domain models
│   ├── minos_core_protocol.dart # Abstract service contract (DI boundary)
│   ├── auth_state.dart          # Auth lifecycle states
│   ├── agent_profile.dart       # Local bot cache / draft model
│   ├── social_message.dart      # Chat message model
│   ├── group_member.dart        # Group membership model
│   └── minos_error_display.dart # Error presentation helpers
│
├── infrastructure/              # Data Layer — Services
│   ├── minos_core.dart          # Rust FFI bridge (implements MinosCoreProtocol)
│   ├── secure_pairing_store.dart# Keychain persistence
│   ├── social_cache_store.dart  # SQLite message cache + outbox SQL
│   ├── im_outbox_store.dart     # Outbox policy helpers
│   ├── agent_profile_store.dart # JSON file persistence
│   └── app_paths.dart           # Platform path resolution
│
├── data/                        # Data Layer — Repositories / services
│   ├── repositories/            # Concrete repository providers + adapters
│   └── services/                # Concrete service/store providers
│
├── application/                 # Application Layer — ViewModels (Riverpod)
│   ├── minos_providers.dart     # Connection / hosts / presence
│   ├── auth_provider.dart       # Auth state controller
│   ├── social/                  # Conversation IM feature modules
│   │   ├── social_conversation_state.dart  # freezed timeline state
│   │   ├── social_conversation_notifier.dart
│   │   ├── social_inbox_notifier.dart
│   │   ├── social_chat_view_model.dart     # feature ViewModel
│   │   ├── social_chat_actions.dart        # intentful actions
│   │   ├── social_ui_state.dart
│   │   ├── social_friends_providers.dart
│   │   └── social_realtime_sync.dart
│   ├── social_providers.dart    # Compatibility barrel for social/*
│   ├── im_outbox_worker.dart    # Local IM outbox drain
│   ├── agent_profiles_provider.dart  # Local bot cache for compose
│   ├── agent_conversation_actions.dart # Create agent conversation
│   ├── group_agent_provider.dart     # Participants / mention roster
│   ├── social_actions.dart      # DM / group create actions
│   ├── preferred_agent_provider.dart # Preferred agent selection
│   ├── ui_state_providers.dart  # Shell tab + login form state
│   ├── log_records_provider.dart     # Debug log mirror
│   ├── request_trace_records_provider.dart # Request trace mirror
│   └── root_route_decision.dart # Navigation decision logic
│
├── ui/                          # UI Layer — Feature-organized implementation
│   ├── core/
│   │   └── widgets/widgets.dart # Shared widget exports
│   └── features/
│       ├── auth/                # Auth screens/widgets + barrel
│       ├── messages/            # Conversation inbox (primary tab)
│       ├── social/              # Conversation chat / members + chrome
│       ├── hosts/               # Linked hosts list
│       ├── account/             # Profile / logout
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
- `RealtimeEventsRepository` exposes Account `uiEvents` only (snapshot/presence/notices). Thread/project/agent-session compose APIs are not part of the Mobile contract.

### Application Layer (`lib/application/`)
- Riverpod providers acting as ViewModels (codegen `@riverpod` preferred).
- Per-feature aggregation: UI watches feature ViewModels (e.g. `SocialChatViewModel`) and calls Action facades instead of many fine-grained providers.
- Complex timeline state uses freezed immutable models (`SocialConversationState`).
- Depends on `domain/` models and `data/repositories/`, never on `ui/` or `infrastructure/` directly.
- Root dependency injection happens in `main()` by overriding `minosCoreServiceProvider`.
- Collaboration send path: `SocialChatActions` → `SocialConversation` → outbox → `sendChatMessage`.

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

## Routes (IM-first)

| Route | Page | Purpose |
|-------|------|---------|
| `/splash` | splash | bootstrap |
| `/login` | `LoginPage` | auth |
| `/` | `AppShellPage` | Messages / Hosts / Account |
| `/social` | redirect → `/` | legacy inbox deep link |
| `/social/chat/:conversationId` | `SocialChatPage` | conversation timeline |
| `/social/chat/:conversationId/members` | `GroupMembersPage` | members |
| `/log-viewer` | `LogViewerPage` | debug |

Removed from product: `/thread/*`, `/sessions`, `/agent-start`, `/agent-profile/*`, `/project/*`.

## Adding a New Feature

1. Define domain models in `lib/domain/` (if new entities are needed).
2. Extend repositories/services and `MinosCoreProtocol` + `MinosCore` when new API calls are required.
3. Create Riverpod providers or action facades in `lib/application/`.
4. Build screens/widgets in `lib/ui/features/<feature>/`.
5. Export the feature surface from `lib/ui/features/<feature>/<feature>.dart`.
6. Register provider overrides or routing changes in the shell when needed.
