# UI Layer — Feature-Organized Barrel Exports

This directory provides **feature-organized barrel exports** that map the
presentation layer into a discoverable, architecture-aligned structure.

## Usage

Instead of importing individual files scattered across `presentation/` and
`application/`, import the feature barrel:

```dart
// Before (scattered imports):
import 'package:minos/application/project_providers.dart';
import 'package:minos/presentation/pages/project_list_page.dart';
import 'package:minos/presentation/pages/project_detail_page.dart';

// After (feature barrel):
import 'package:minos/ui/features/projects/projects.dart';
```

## Feature Map

| Feature   | Barrel                              | Description                    |
|-----------|-------------------------------------|--------------------------------|
| auth      | `ui/features/auth/auth.dart`        | Login / register               |
| chat      | `ui/features/chat/chat.dart`        | Agent thread chat              |
| projects  | `ui/features/projects/projects.dart`| Project CRUD + threads         |
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
