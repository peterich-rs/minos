# UI Layer — Feature-Organized Widgets and Barrels

This directory contains the concrete widget implementation for the app, grouped
by feature, plus barrel exports that keep UI imports discoverable.

## Design system (Phase F)

Golden-path surfaces use the **Minos design tokens** under `ui/theme/`:

| File | Role |
|------|------|
| `theme/minos_colors.dart` | Semantic colors (light/dark) |
| `theme/minos_spacing.dart` | Spacing scale |
| `theme/minos_radii.dart` | Corner radii |
| `theme/minos_typography.dart` | Type scale |
| `theme/minos_theme.dart` | `ThemeData` + `MinosThemeExtension` |

Prefer `context.minosColors` and `MinosSpacing` / `MinosRadii` over one-off
`Color(0x…)` and over shadcn Material transplants.

Root app wiring (`shell/views/app.dart`): `MaterialApp.router` + Minos theme.
A residual `ShadTheme`/`ShadToaster` wrapper remains for unmigrated screens
(agent editor, social) until those surfaces are retired.

## Golden-path navigation

Bottom tabs (single column):

1. **Sessions** — agent session inbox (`features/sessions/`)
2. **Hosts** — linked Macs from `GET /v1/hosts` (`features/hosts/`)
3. **账户** — account + logout (`features/account/`)

Social / projects remain on secondary routes but are **not** in the primary tab bar.

## Feature Map

| Feature   | Barrel                                | Description                    |
|-----------|---------------------------------------|--------------------------------|
| auth      | `ui/features/auth/auth.dart`          | Login / register               |
| sessions  | `ui/features/sessions/sessions.dart`  | Golden-path inbox              |
| hosts     | `ui/features/hosts/hosts.dart`        | Linked hosts                   |
| chat      | `ui/features/chat/chat.dart`          | Agent session chat             |
| account   | `ui/features/account/`                | Account tab                    |
| projects  | `ui/features/projects/projects.dart`  | Project CRUD (secondary)       |
| agents    | `ui/features/agents/agents.dart`      | Agent profile management       |
| social    | `ui/features/social/social.dart`      | Friends & conversations        |
| debug     | `ui/features/debug/debug.dart`        | Log viewer & traces            |
| shell     | `ui/features/shell/shell.dart`        | Root navigation shell          |

## Shared Widgets

Cross-feature widgets live in `ui/core/widgets/widgets.dart`:

```dart
import 'package:minos/ui/core/widgets/widgets.dart';
```

Includes approval sheet, empty state, page header, surface card, status dot.

## Layer Boundary Rules

- Views (this layer) may import from `application/` providers.
- Views must NOT import from `infrastructure/` directly.
- Views must NOT contain business logic — delegate to providers.
- Shared widgets in `core/` must be stateless or self-contained.
