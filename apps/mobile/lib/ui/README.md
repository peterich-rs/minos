# UI Layer — Feature-Organized Widgets and Barrels

This directory contains the concrete widget implementation for the app, grouped
by feature, plus barrel exports that keep UI imports discoverable.

## Design system

Golden-path surfaces use the **Minos design tokens** under `ui/theme/`:

| File | Role |
|------|------|
| `theme/minos_colors.dart` | Semantic colors (light/dark) |
| `theme/minos_spacing.dart` | Spacing scale |
| `theme/minos_radii.dart` | Corner radii |
| `theme/minos_typography.dart` | Type scale |
| `theme/minos_theme.dart` | `ThemeData` + `MinosThemeExtension` |

Prefer `context.minosColors` and `MinosSpacing` / `MinosRadii` over one-off
`Color(0x…)` literals. Shared chrome lives in `ui/core/widgets/`
(`MinosButton`, `MinosTextField`, `MinosProgress`, `showMinosToast`, empty
state, page header, surface card).

Root app wiring (`shell/views/app.dart`): `MaterialApp.router` + Minos theme
only — **no shadcn_ui**.

## Golden-path navigation

Bottom tabs (single column):

1. **消息** — conversation inbox (`features/messages/`), sorted by last activity
2. **Hosts** — linked Macs from `GET /v1/hosts` (`features/hosts/`)
3. **账户** — account + logout + developer log viewer

Conversation detail is `/social/chat/:conversationId` (`features/social/`).

## Feature Map

| Feature   | Barrel                                | Description                         |
|-----------|---------------------------------------|-------------------------------------|
| auth      | `ui/features/auth/auth.dart`          | Login / register                    |
| messages  | `ui/features/messages/messages.dart`  | Golden-path conversation inbox      |
| hosts     | `ui/features/hosts/hosts.dart`        | Linked hosts                        |
| account   | `ui/features/account/`                | Account tab                         |
| social    | `ui/features/social/social.dart`      | Collaboration IM (Slack-style rows) |
| debug     | `ui/features/debug/debug.dart`        | Log viewer & traces                 |
| shell     | `ui/features/shell/shell.dart`        | Root navigation shell               |

Removed product surfaces (no longer shipped on Mobile): sessions list, agent
session transcript chat, projects, agents hub.

## Shared Widgets

Cross-feature widgets live in `ui/core/widgets/widgets.dart`:

```dart
import 'package:minos/ui/core/widgets/widgets.dart';
```

Includes toast, buttons, text field, progress, empty state, page header,
surface card, status dot.

## Collaboration IM widgets (`features/social/`)

Conversation timeline is **Desktop-aligned Slack/Buzz full-width rows** (not
L/R messenger bubbles):

| Widget / lib | Role |
|--------------|------|
| `lib/message_grouping.dart` | 10 min author grouping + day dividers |
| `widgets/conversation_message_row.dart` | Full row: avatar, header, markdown, retry |
| `widgets/conversation_message_chrome.dart` | Left-aligned shell (parity with Desktop `MessageChrome`) |
| `widgets/conversation_day_divider.dart` | Today / Yesterday / date pills |
| `widgets/conversation_reply_preview.dart` | Reply quote chip |
| `widgets/conversation_system_message.dart` | Recall / system centered chrome |
| `widgets/conversation_message_actions.dart` | Long-press sheet: reply / copy / retry / recall |

Application orchestration for the open chat is
`SocialChatViewModel` + `SocialChatActions` under `application/social/`.

## Layer Boundary Rules

- Views (this layer) may import from `application/` providers.
- Views must NOT import from `infrastructure/` directly.
- Views must NOT contain business logic — delegate to providers / feature
  ViewModels and Actions.
- Shared widgets in `core/` must be stateless or self-contained.
