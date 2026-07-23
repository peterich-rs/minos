# UI Layer — Feature-Organized Widgets and Barrels

This directory contains the concrete widget implementation for the app, grouped
by feature, plus barrel exports that keep UI imports discoverable.

## Usage

Instead of importing individual view files from several feature folders, import
the feature barrel when you need that UI surface:

```dart
// Before (scattered view imports):
import 'package:minos/ui/features/projects/views/project_detail_page.dart';
import 'package:minos/ui/features/projects/views/project_list_page.dart';

// After (feature barrel):
import 'package:minos/ui/features/projects/projects.dart';
```

Application providers are still imported directly from `application/` when the
UI needs them.

## Feature Map

| Feature   | Barrel                              | Description                    |
|-----------|-------------------------------------|--------------------------------|
| auth      | `ui/features/auth/auth.dart`        | Login / register               |
| chat      | `ui/features/chat/chat.dart`        | Agent session chat              |
| projects  | `ui/features/projects/projects.dart`| Project CRUD + sessions         |
| agents    | `ui/features/agents/agents.dart`    | Agent profile management       |
| social    | `ui/features/social/social.dart`    | Friends & conversations        |
| pairing   | `ui/features/pairing/pairing.dart`  | QR device pairing              |
| debug     | `ui/features/debug/debug.dart`      | Log viewer & traces            |
| shell     | `ui/features/shell/shell.dart`      | Root navigation shell          |

## Shared Widgets

Cross-feature widgets live in `ui/core/widgets/widgets.dart`:

```dart
import 'package:minos/ui/core/widgets/widgets.dart';
```

## Layer Boundary Rules

- Views (this layer) may import from `application/` providers.
- Views must NOT import from `infrastructure/` directly.
- Views must NOT contain business logic — delegate to providers.
- Shared widgets in `core/` must be stateless or self-contained.
