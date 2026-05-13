# Mobile Project Refactoring Guide

## Overview

This refactoring introduces a **Project** concept to Minos. A project groups threads (conversations) under a named workspace, stored at `.minos/workspaces/<slug>`. The mobile app's home screen is now a **Project List** (full-screen), and tapping a project navigates to a **Discord-style channel list** showing all threads within that project.

## Architecture Changes

### Daemon (Rust)

1. **New migration** (`crates/minos-daemon/migrations/0002_projects.sql`):
   - `projects` table: `project_id`, `name`, `workspace_slug`, `created_at`, `updated_at`
   - `threads.project_id` column (nullable FK to projects)

2. **LocalStore** (`crates/minos-daemon/src/store/mod.rs`):
   - `create_project`, `list_projects`, `get_project`, `update_project_name`, `delete_project`
   - `list_threads_by_project`, `assign_thread_to_project`, `touch_project`
   - `ProjectRow` struct added

3. **AgentGlue** (`crates/minos-daemon/src/agent.rs`):
   - `create_project`, `list_projects`, `update_project`, `delete_project`
   - `list_project_threads`, `start_agent_in_project`

4. **RPC Server** (`crates/minos-daemon/src/rpc_server.rs`):
   - New methods: `minos_create_project`, `minos_list_projects`, `minos_update_project`, `minos_delete_project`, `minos_list_project_threads`, `minos_start_agent_in_project`

### Protocol (Rust)

5. **Messages** (`crates/minos-protocol/src/messages.rs`):
   - `ProjectSummary`, `CreateProjectRequest/Response`, `UpdateProjectRequest`, `DeleteProjectRequest`
   - `ListProjectsResponse`, `ListProjectThreadsParams/Response`

### Mobile Client (Rust)

6. **minos-mobile** (`crates/minos-mobile/src/client.rs`):
   - `create_project`, `list_projects`, `update_project`, `delete_project`
   - `list_project_threads`, `start_agent_in_project`

7. **FFI Bridge** (`crates/minos-ffi-frb/src/api/minos.rs`):
   - Corresponding FFI methods + type exports

### Mobile App (Flutter/Dart)

8. **Domain** (`apps/mobile/lib/domain/`):
   - `project_types.dart` — temporary type definitions (replaced by FRB codegen)
   - `minos_core_protocol.dart` — added project method signatures

9. **Infrastructure** (`apps/mobile/lib/infrastructure/`):
   - `minos_core.dart` — project method implementations (calls FRB-generated client)

10. **Application** (`apps/mobile/lib/application/`):
    - `project_providers.dart` — `ProjectList`, `ProjectThreads`, `SelectedProject` providers
    - `root_route_decision.dart` — `RootRoute.projectList` / `projectListOffline` (replaces `threadList`)

11. **Presentation** (`apps/mobile/lib/presentation/`):
    - `pages/project_list_page.dart` — full-screen project list (new home)
    - `pages/project_detail_page.dart` — Discord-style channel sidebar + thread list
    - `app.dart` — routes to `ProjectListPage` instead of `AppShellPage`

## Navigation Flow

```
Login → ProjectListPage (full-screen grid/list of projects)
         ↓ tap project
       ProjectDetailPage (Discord-style: channel sidebar + thread list)
         ↓ tap thread
       ThreadViewPage (existing chat surface)
         ← swipe back
       ProjectDetailPage
         ← swipe back
       ProjectListPage
```

## Steps to Complete

After merging these changes, run:

1. **FRB Codegen** (generates Dart bindings for new Rust methods):
   ```bash
   cd apps/mobile
   flutter_rust_bridge_codegen generate
   ```

2. **Riverpod Codegen** (regenerates `.g.dart` files):
   ```bash
   cd apps/mobile
   dart run build_runner build --delete-conflicting-outputs
   ```

3. **Delete temporary types** once FRB codegen produces them:
   - Remove `apps/mobile/lib/domain/project_types.dart`
   - Update imports in `minos_core_protocol.dart`, `minos_core.dart`, `project_providers.dart`, `project_list_page.dart`, `project_detail_page.dart` to use `package:minos/src/rust/api/minos.dart` instead

4. **Test the flow**:
   ```bash
   cd apps/mobile
   flutter run
   ```

## Design Decisions

- **Projects are daemon-local**: Projects live in the daemon's SQLite, not the backend. This keeps the feature simple and avoids backend migration.
- **Threads can exist without a project**: The `project_id` column is nullable for backward compatibility with existing threads.
- **Workspace directories**: Each project creates a folder at `.minos/workspaces/<slug>` where the agent operates.
- **AppShellPage preserved**: The old `AppShellPage` (3-tab layout) is still available but no longer the root. It can be accessed from within a project or removed later.
