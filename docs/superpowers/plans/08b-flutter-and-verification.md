# Mobile Auth + Agent Session — Phases 8–12 (Flutter UI + Verification)

> **Companion to** `08-mobile-auth-and-agent-session.md` and `08a-mobile-rust-and-frb.md`. **REQUIRED SUB-SKILL:** Use superpowers:subagent-driven-development. Read the parent plan's preamble (worktree, critical clarifications) before starting.

These phases consume the frb surface from Phase 7 and deliver the user-facing surfaces: login, account-gated routing, Remodex-style chat, account settings, and the verification gates. After Phase 12 the spec ships.

---

## Phase 8: Flutter Domain + State

### Task 8.1: Auth state sealed class

**Files:**
- Create: `apps/mobile/lib/domain/auth_state.dart`

- [ ] **Step 1: Write**

```dart
import 'package:flutter/foundation.dart' show immutable;
import '../src/rust/api/minos.dart' show MinosError, AuthSummary;

@immutable
sealed class AuthState {
  const AuthState();
}

class AuthBootstrapping extends AuthState { const AuthBootstrapping(); }

class AuthUnauthenticated extends AuthState { const AuthUnauthenticated(); }

class AuthAuthenticated extends AuthState {
  final AuthSummary account;
  const AuthAuthenticated(this.account);
  @override
  bool operator ==(Object other) =>
      other is AuthAuthenticated && other.account.accountId == account.accountId;
  @override
  int get hashCode => account.accountId.hashCode;
}

class AuthRefreshing extends AuthState { const AuthRefreshing(); }

class AuthRefreshFailed extends AuthState {
  final MinosError error;
  const AuthRefreshFailed(this.error);
}
```

- [ ] **Step 2: Test**

```dart
// apps/mobile/test/unit/auth_state_test.dart
void main() {
  test('AuthAuthenticated equals by account_id', () {
    final a1 = AuthAuthenticated(AuthSummary(accountId: 'a', email: 'a@b'));
    final a2 = AuthAuthenticated(AuthSummary(accountId: 'a', email: 'b@c'));
    expect(a1, a2);
  });
}
```

- [ ] **Step 3: Run + commit**

```bash
cd apps/mobile && fvm flutter test test/unit/auth_state_test.dart
git add apps/mobile/lib/domain/auth_state.dart apps/mobile/test/unit/auth_state_test.dart
git commit -m "feat(mobile): AuthState sealed class"
```

---

### Task 8.2: Active session sealed class

**Files:**
- Create: `apps/mobile/lib/domain/active_session.dart`

- [ ] **Step 1: Write**

```dart
import 'package:flutter/foundation.dart' show immutable;
import '../src/rust/api/minos.dart' show AgentName, MinosError;

@immutable
sealed class ActiveSession { const ActiveSession(); }

class SessionIdle extends ActiveSession { const SessionIdle(); }

class SessionStarting extends ActiveSession {
  final AgentName agent;
  final String prompt;
  const SessionStarting({required this.agent, required this.prompt});
}

class SessionStreaming extends ActiveSession {
  final String threadId;
  final AgentName agent;
  const SessionStreaming({required this.threadId, required this.agent});
}

class SessionAwaitingInput extends ActiveSession {
  final String threadId;
  final AgentName agent;
  const SessionAwaitingInput({required this.threadId, required this.agent});
}

class SessionStopped extends ActiveSession {
  final String threadId;
  const SessionStopped(this.threadId);
}

class SessionError extends ActiveSession {
  final String? threadId;
  final MinosError error;
  const SessionError({this.threadId, required this.error});
}
```

- [ ] **Step 2: Commit**

```bash
git add apps/mobile/lib/domain/active_session.dart
git commit -m "feat(mobile): ActiveSession sealed class"
```

---

### Task 8.3: Extend `MinosCoreProtocol`

**Files:**
- Modify: `apps/mobile/lib/domain/minos_core_protocol.dart`

- [ ] **Step 1: Add new abstract methods**

```dart
abstract class MinosCoreProtocol {
  // existing: pairWithQrJson, forgetPeer, hasPersistedPairing, listThreads, readThread,
  //           connectionStates, uiEvents, currentConnectionState

  Future<AuthSummary> register({required String email, required String password});
  Future<AuthSummary> login({required String email, required String password});
  Future<void> refreshSession();
  Future<void> logout();

  Future<StartAgentResponse> startAgent({required AgentName agent, required String prompt});
  Future<void> sendUserMessage({required String sessionId, required String text});
  Future<void> stopAgent();

  void notifyForegrounded();
  void notifyBackgrounded();

  Stream<AuthStateFrame> get authStates;
}
```

- [ ] **Step 2: Cargo check on the dart side**

```bash
cd apps/mobile && fvm flutter analyze
```

Expected: existing `MinosCore` class will report missing implementations. Fix in Task 8.4.

- [ ] **Step 3: Commit (after 8.4 lands)**

---

### Task 8.4: Update `MinosCore` implementation

**Files:**
- Modify: `apps/mobile/lib/infrastructure/minos_core.dart`

- [ ] **Step 1: Add forwarders**

```dart
@override
Future<AuthSummary> register({required String email, required String password}) =>
    _client.register(email: email, password: password);

@override
Future<AuthSummary> login({required String email, required String password}) =>
    _client.login(email: email, password: password);

@override
Future<void> refreshSession() => _client.refreshSession();

@override
Future<void> logout() => _client.logout();

@override
Future<StartAgentResponse> startAgent({required AgentName agent, required String prompt}) =>
    _client.startAgent(agent: agent, prompt: prompt);

@override
Future<void> sendUserMessage({required String sessionId, required String text}) =>
    _client.sendUserMessage(sessionId: sessionId, text: text);

@override
Future<void> stopAgent() => _client.stopAgent();

@override
void notifyForegrounded() => _client.notifyForegrounded();

@override
void notifyBackgrounded() => _client.notifyBackgrounded();

@override
Stream<AuthStateFrame> get authStates => _client.subscribeAuthState();
```

- [ ] **Step 2: Update `_FakeCore` test fixture**

In all test files that mock `MinosCoreProtocol`, add the new method stubs.

- [ ] **Step 3: Run + commit**

```bash
cd apps/mobile && fvm flutter analyze && fvm flutter test
git add apps/mobile/lib/domain/minos_core_protocol.dart \
        apps/mobile/lib/infrastructure/minos_core.dart \
        apps/mobile/test/
git commit -m "feat(mobile): MinosCore exposes auth + agent + lifecycle"
```

---

### Task 8.5: `auth_provider`

**Files:**
- Create: `apps/mobile/lib/application/auth_provider.dart`

- [ ] **Step 1: Implement**

```dart
import 'dart:async';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:riverpod_annotation/riverpod_annotation.dart';

import '../domain/auth_state.dart';
import '../src/rust/api/minos.dart' show AuthStateFrame_Authenticated, AuthStateFrame_RefreshFailed,
    AuthStateFrame_Refreshing, AuthStateFrame_Unauthenticated;
import 'minos_providers.dart';

part 'auth_provider.g.dart';

@Riverpod(keepAlive: true)
class AuthController extends _$AuthController {
  StreamSubscription<dynamic>? _sub;

  @override
  AuthState build() {
    final core = ref.watch(minosCoreProvider);
    _sub = core.authStates.listen((frame) {
      switch (frame) {
        case AuthStateFrame_Unauthenticated():
          state = const AuthUnauthenticated();
        case AuthStateFrame_Authenticated(:final account):
          state = AuthAuthenticated(account);
        case AuthStateFrame_Refreshing():
          state = const AuthRefreshing();
        case AuthStateFrame_RefreshFailed(:final error):
          state = AuthRefreshFailed(error);
      }
    });
    ref.onDispose(() => _sub?.cancel());
    return const AuthBootstrapping();
  }

  Future<void> register(String email, String password) async {
    final core = ref.read(minosCoreProvider);
    await core.register(email: email, password: password);
    // AuthState is updated via the stream subscription above.
  }

  Future<void> login(String email, String password) async {
    await ref.read(minosCoreProvider).login(email: email, password: password);
  }

  Future<void> logout() async {
    await ref.read(minosCoreProvider).logout();
  }
}
```

- [ ] **Step 2: Run codegen for `*.g.dart`**

```bash
cd apps/mobile && fvm dart run build_runner build --delete-conflicting-outputs
```

- [ ] **Step 3: Test**

```dart
// apps/mobile/test/unit/auth_provider_test.dart
test('AuthController emits Authenticated on stream Authenticated', () async {
  final core = _FakeCore();
  final container = ProviderContainer(overrides: [
    minosCoreProvider.overrideWithValue(core),
  ]);
  addTearDown(container.dispose);
  expect(container.read(authControllerProvider), const AuthBootstrapping());
  core.emitAuthFrame(AuthStateFrame.authenticated(account: AuthSummary(accountId: 'a', email: 'a@b')));
  await pumpEventQueue();
  expect(container.read(authControllerProvider), isA<AuthAuthenticated>());
});
```

- [ ] **Step 4: Run + commit**

```bash
fvm flutter test test/unit/auth_provider_test.dart
git add apps/mobile/lib/application/auth_provider.dart \
        apps/mobile/lib/application/auth_provider.g.dart \
        apps/mobile/test/unit/auth_provider_test.dart
git commit -m "feat(mobile): AuthController Riverpod provider"
```

---

### Task 8.6: `active_session_provider`

**Files:**
- Create: `apps/mobile/lib/application/active_session_provider.dart`

- [ ] **Step 1: Implement state machine**

```dart
import 'dart:async';
import 'package:riverpod_annotation/riverpod_annotation.dart';

import '../domain/active_session.dart';
import '../src/rust/api/minos.dart' show AgentName, UiEventMessage_MessageCompleted,
    UiEventMessage_ThreadClosed, UiEventMessage_Error;
import 'minos_providers.dart';

part 'active_session_provider.g.dart';

@Riverpod(keepAlive: true)
class ActiveSessionController extends _$ActiveSessionController {
  StreamSubscription<dynamic>? _eventsSub;

  @override
  ActiveSession build() {
    final core = ref.watch(minosCoreProvider);
    _eventsSub = core.uiEvents.listen((frame) {
      final s = state;
      if (s is! SessionStreaming || s.threadId != frame.threadId) return;
      switch (frame.ui) {
        case UiEventMessage_MessageCompleted():
          state = SessionAwaitingInput(threadId: s.threadId, agent: s.agent);
        case UiEventMessage_ThreadClosed():
          state = SessionStopped(s.threadId);
        case UiEventMessage_Error(:final message):
          state = SessionError(threadId: s.threadId,
              error: MinosError.agentStartFailed(reason: message));
        default: break;
      }
    });
    ref.onDispose(() => _eventsSub?.cancel());
    return const SessionIdle();
  }

  Future<void> start({required AgentName agent, required String prompt}) async {
    state = SessionStarting(agent: agent, prompt: prompt);
    try {
      final resp = await ref.read(minosCoreProvider).startAgent(agent: agent, prompt: prompt);
      state = SessionStreaming(threadId: resp.sessionId, agent: agent);
    } on MinosError catch (e) {
      state = SessionError(error: e);
    }
  }

  Future<void> send(String text) async {
    final s = state;
    if (s is! SessionAwaitingInput && s is! SessionStreaming) return;
    final threadId = (s as dynamic).threadId as String;
    final agent = (s as dynamic).agent as AgentName;
    state = SessionStreaming(threadId: threadId, agent: agent);
    try {
      await ref.read(minosCoreProvider).sendUserMessage(sessionId: threadId, text: text);
    } on MinosError catch (e) {
      state = SessionError(threadId: threadId, error: e);
    }
  }

  Future<void> stop() async {
    final s = state;
    if (s is SessionStreaming || s is SessionAwaitingInput) {
      try {
        await ref.read(minosCoreProvider).stopAgent();
      } on MinosError {
        // best effort
      }
      state = SessionStopped((s as dynamic).threadId as String);
    }
  }
}
```

- [ ] **Step 2: Run codegen + test**

```bash
fvm dart run build_runner build --delete-conflicting-outputs
fvm flutter test test/unit/active_session_machine_test.dart
```

- [ ] **Step 3: Commit**

```bash
git add apps/mobile/lib/application/active_session_provider.dart \
        apps/mobile/lib/application/active_session_provider.g.dart \
        apps/mobile/test/unit/active_session_machine_test.dart
git commit -m "feat(mobile): ActiveSession state machine + provider"
```

---

### Task 8.7: `secure_storage_provider` (extend `SecurePairingStore` for auth)

**Files:**
- Modify: `apps/mobile/lib/infrastructure/secure_pairing_store.dart`

- [ ] **Step 1: Add three Keychain keys**

```dart
static const _accessTokenKey = 'minos.access_token';
static const _accessExpiresAtMsKey = 'minos.access_expires_at_ms';
static const _refreshTokenKey = 'minos.refresh_token';
static const _accountIdKey = 'minos.account_id';
static const _accountEmailKey = 'minos.account_email';
```

- [ ] **Step 2: Extend `loadState` and `saveState` to include the new fields**

Map them onto the new `PersistedPairingState` fields from Task 4.1.

- [ ] **Step 3: Add `clearAuth()`**

```dart
Future<void> clearAuth() async {
  await Future.wait([
    _storage.delete(key: _accessTokenKey),
    _storage.delete(key: _accessExpiresAtMsKey),
    _storage.delete(key: _refreshTokenKey),
    _storage.delete(key: _accountIdKey),
    _storage.delete(key: _accountEmailKey),
  ]);
}
```

- [ ] **Step 4: Update tests**

In `test/unit/secure_pairing_store_test.dart`, add roundtrip cases for the new fields.

- [ ] **Step 5: Run + commit**

```bash
fvm flutter test test/unit/secure_pairing_store_test.dart
git add apps/mobile/lib/infrastructure/secure_pairing_store.dart \
        apps/mobile/test/unit/secure_pairing_store_test.dart
git commit -m "feat(mobile): SecurePairingStore persists auth tokens"
```

---

### Task 8.8: Update `root_route_decision`

**Files:**
- Modify: `apps/mobile/lib/application/root_route_decision.dart`

- [ ] **Step 1: Replace decision logic**

```dart
import '../domain/auth_state.dart';
import '../src/rust/api/minos.dart' show ConnectionState_Connected,
    ConnectionState_Reconnecting, ConnectionState_Disconnected;

enum RootRoute { splash, login, pairing, threadList, threadListMacOffline }

RootRoute decideRootRoute({
  required AuthState authState,
  required ConnectionState? connectionState,
  required bool hasPersistedPairing,
}) {
  return switch (authState) {
    AuthBootstrapping()       => RootRoute.splash,
    AuthRefreshing()          => RootRoute.splash,
    AuthUnauthenticated()     => RootRoute.login,
    AuthRefreshFailed()       => RootRoute.login,
    AuthAuthenticated() when !hasPersistedPairing => RootRoute.pairing,
    AuthAuthenticated() => switch (connectionState) {
        ConnectionState_Connected() => RootRoute.threadList,
        ConnectionState_Reconnecting() => RootRoute.threadList,
        _ => RootRoute.threadListMacOffline,
      },
  };
}
```

- [ ] **Step 2: Update `_Router` in `presentation/app.dart`**

Watch `authControllerProvider` AND `connectionStateProvider` AND `hasPersistedPairingProvider`, then call `decideRootRoute`.

- [ ] **Step 3: Update test cases in `root_route_decision_test.dart`**

Add table cases for each new state combination.

- [ ] **Step 4: Run + commit**

```bash
fvm flutter test test/unit/root_route_decision_test.dart
git add apps/mobile/lib/application/root_route_decision.dart \
        apps/mobile/lib/presentation/app.dart \
        apps/mobile/test/unit/root_route_decision_test.dart
git commit -m "feat(mobile): root route gates on auth + pairing + connection"
```

---

### Task 8.9: Update `minos_providers` bootstrap order

**Files:**
- Modify: `apps/mobile/lib/main.dart`
- Modify: `apps/mobile/lib/infrastructure/minos_core.dart`

- [ ] **Step 1: Change `MinosCore.init` to defer WS connect**

WS startup should now happen on `AuthAuthenticated` transition, not in `init`. Modify `resolveClient` to NOT call `resumePersistedSession` automatically. The auth controller's stream listener will trigger reconnect on `Authenticated`.

- [ ] **Step 2: Hydrate auth state into Rust on boot**

In `main()`, after `MinosCore.init`, read persisted auth from secure storage and call into Rust to seed `auth_session` (add a `hydrateAuth(...)` frb method on `MobileClient` if not already covered by `newWithPersistedState`).

> **Implementation note:** the simplest path is to extend `newWithPersistedState` to take auth fields too — the secure store already round-trips them, and the Rust side then emits `AuthStateFrame::Authenticated` from its initial state.

- [ ] **Step 3: Run + commit**

```bash
fvm flutter analyze && fvm flutter test
git add apps/mobile/lib/main.dart apps/mobile/lib/infrastructure/minos_core.dart
git commit -m "feat(mobile): bootstrap defers WS until authenticated"
```

---

## Phase 9: Login UI

### Task 9.1: `AuthForm` widget

**Files:**
- Create: `apps/mobile/lib/presentation/widgets/auth/auth_form.dart`

- [ ] **Step 1: Implement**

```dart
import 'package:flutter/material.dart';
import 'package:shadcn_ui/shadcn_ui.dart';

enum AuthMode { login, register }

class AuthForm extends StatefulWidget {
  final AuthMode mode;
  final ValueChanged<AuthMode> onModeChanged;
  final Future<void> Function(String email, String password) onSubmit;
  final bool inFlight;
  const AuthForm({super.key, required this.mode, required this.onModeChanged,
                  required this.onSubmit, required this.inFlight});

  @override
  State<AuthForm> createState() => _AuthFormState();
}

class _AuthFormState extends State<AuthForm> {
  final _emailCtl = TextEditingController();
  final _passwordCtl = TextEditingController();
  final _confirmCtl = TextEditingController();
  String? _emailErr;
  String? _passwordErr;
  String? _confirmErr;

  static final _emailRe = RegExp(r'^[^\s@]+@[^\s@]+\.[^\s@]+$');

  bool _validate() {
    setState(() {
      _emailErr = _emailRe.hasMatch(_emailCtl.text) ? null : 'Invalid email';
      _passwordErr = _passwordCtl.text.length >= 8 ? null : 'Min 8 characters';
      if (widget.mode == AuthMode.register) {
        _confirmErr = _confirmCtl.text == _passwordCtl.text ? null : 'Does not match';
      } else {
        _confirmErr = null;
      }
    });
    return _emailErr == null && _passwordErr == null && _confirmErr == null;
  }

  @override
  Widget build(BuildContext context) {
    return Column(crossAxisAlignment: CrossAxisAlignment.stretch, children: [
      ShadInput(controller: _emailCtl, placeholder: const Text('Email'),
          enabled: !widget.inFlight),
      if (_emailErr != null) Text(_emailErr!, style: const TextStyle(color: Colors.red)),
      const SizedBox(height: 8),
      ShadInput(controller: _passwordCtl, placeholder: const Text('Password'),
          obscureText: true, enabled: !widget.inFlight),
      if (_passwordErr != null) Text(_passwordErr!, style: const TextStyle(color: Colors.red)),
      if (widget.mode == AuthMode.register) ...[
        const SizedBox(height: 8),
        ShadInput(controller: _confirmCtl, placeholder: const Text('Confirm password'),
            obscureText: true, enabled: !widget.inFlight),
        if (_confirmErr != null) Text(_confirmErr!, style: const TextStyle(color: Colors.red)),
      ],
      const SizedBox(height: 16),
      ShadButton(
        enabled: !widget.inFlight,
        onPressed: () async {
          if (!_validate()) return;
          await widget.onSubmit(_emailCtl.text.trim(), _passwordCtl.text);
        },
        child: widget.inFlight
            ? const SizedBox(width: 16, height: 16, child: CircularProgressIndicator(strokeWidth: 2))
            : Text(widget.mode == AuthMode.login ? 'Log in' : 'Register'),
      ),
      TextButton(
        onPressed: widget.inFlight ? null : () => widget.onModeChanged(
          widget.mode == AuthMode.login ? AuthMode.register : AuthMode.login,
        ),
        child: Text(widget.mode == AuthMode.login ? 'Create account' : 'Have an account? Log in'),
      ),
    ]);
  }
}
```

- [ ] **Step 2: Widget test**

`apps/mobile/test/widget/auth_form_test.dart`:
- Empty submit → button stays enabled, validation errors render.
- `inFlight=true` → button disabled, spinner shown.
- Mode toggle → confirm field appears in register mode.

- [ ] **Step 3: Run + commit**

```bash
fvm flutter test test/widget/auth_form_test.dart
git add apps/mobile/lib/presentation/widgets/auth/auth_form.dart \
        apps/mobile/test/widget/auth_form_test.dart
git commit -m "feat(mobile): AuthForm widget with validation"
```

---

### Task 9.2: `AuthErrorBanner` widget

**Files:**
- Create: `apps/mobile/lib/presentation/widgets/auth/auth_error_banner.dart`

- [ ] **Step 1: Implement (6 s auto-dismiss)**

```dart
import 'dart:async';
import 'package:flutter/material.dart';
import 'package:shadcn_ui/shadcn_ui.dart';
import '../../../src/rust/api/minos.dart' show MinosError;
import '../../../domain/minos_error_display.dart';

class AuthErrorBanner extends StatefulWidget {
  final MinosError? error;
  const AuthErrorBanner({super.key, required this.error});
  @override
  State<AuthErrorBanner> createState() => _AuthErrorBannerState();
}

class _AuthErrorBannerState extends State<AuthErrorBanner> {
  Timer? _timer;
  bool _visible = false;

  @override
  void didUpdateWidget(AuthErrorBanner old) {
    super.didUpdateWidget(old);
    if (widget.error != null && widget.error != old.error) {
      _timer?.cancel();
      setState(() => _visible = true);
      _timer = Timer(const Duration(seconds: 6), () {
        if (mounted) setState(() => _visible = false);
      });
    }
  }

  @override
  void dispose() { _timer?.cancel(); super.dispose(); }

  @override
  Widget build(BuildContext context) {
    if (!_visible || widget.error == null) return const SizedBox.shrink();
    final e = widget.error!;
    return ShadAlert.destructive(
      title: Text(e.userMessage()),
      description: Text(e.detail()),
    );
  }
}
```

- [ ] **Step 2: Commit**

```bash
git add apps/mobile/lib/presentation/widgets/auth/auth_error_banner.dart
git commit -m "feat(mobile): AuthErrorBanner with auto-dismiss"
```

---

### Task 9.3: `LoginPage`

**Files:**
- Create: `apps/mobile/lib/presentation/pages/login_page.dart`

- [ ] **Step 1: Implement**

```dart
import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:shadcn_ui/shadcn_ui.dart';

import '../../application/auth_provider.dart';
import '../../src/rust/api/minos.dart' show MinosError;
import '../widgets/auth/auth_form.dart';
import '../widgets/auth/auth_error_banner.dart';

class LoginPage extends ConsumerStatefulWidget {
  final MinosError? errorBanner;
  const LoginPage({super.key, this.errorBanner});
  @override
  ConsumerState<LoginPage> createState() => _LoginPageState();
}

class _LoginPageState extends ConsumerState<LoginPage> {
  AuthMode _mode = AuthMode.login;
  bool _inFlight = false;
  MinosError? _error;

  @override
  void initState() { super.initState(); _error = widget.errorBanner; }

  Future<void> _submit(String email, String password) async {
    setState(() { _inFlight = true; _error = null; });
    try {
      if (_mode == AuthMode.login) {
        await ref.read(authControllerProvider.notifier).login(email, password);
      } else {
        await ref.read(authControllerProvider.notifier).register(email, password);
      }
    } on MinosError catch (e) {
      if (e.kind == 'email_taken' && _mode == AuthMode.register) {
        setState(() { _mode = AuthMode.login; _error = e; });
      } else {
        setState(() => _error = e);
      }
    } finally {
      if (mounted) setState(() => _inFlight = false);
    }
  }

  @override
  Widget build(BuildContext context) {
    return Scaffold(
      body: SafeArea(child: Padding(
        padding: const EdgeInsets.all(24),
        child: Column(crossAxisAlignment: CrossAxisAlignment.stretch, children: [
          const SizedBox(height: 60),
          const Text('Minos', style: TextStyle(fontSize: 32, fontWeight: FontWeight.bold)),
          const SizedBox(height: 32),
          AuthErrorBanner(error: _error),
          const SizedBox(height: 16),
          AuthForm(
            mode: _mode,
            onModeChanged: (m) => setState(() => _mode = m),
            onSubmit: _submit,
            inFlight: _inFlight,
          ),
        ]),
      )),
    );
  }
}
```

- [ ] **Step 2: Wire from `_Router` in `app.dart`**

Add the `RouteLogin` case to render `LoginPage(errorBanner: ...)`.

- [ ] **Step 3: Widget test**

`test/widget/login_page_test.dart`:
- Submit valid login → `_FakeCore.login` called with the entered email/password.
- Submit with `email_taken` error → mode switches to login, banner shows.

- [ ] **Step 4: Run + commit**

```bash
fvm flutter test test/widget/login_page_test.dart
git add apps/mobile/lib/presentation/pages/login_page.dart \
        apps/mobile/lib/presentation/app.dart \
        apps/mobile/test/widget/login_page_test.dart
git commit -m "feat(mobile): LoginPage wired to AuthController"
```

---

## Phase 10: Chat UI Rework

### Task 10.1: Add markdown deps

**Files:**
- Modify: `apps/mobile/pubspec.yaml`

- [ ] **Step 1: Add dependencies**

```yaml
dependencies:
  flutter_markdown_plus: ^1
  flutter_highlight: ^0.7
```

- [ ] **Step 2: Resolve**

```bash
cd apps/mobile && fvm flutter pub get
```

- [ ] **Step 3: Commit**

```bash
git add apps/mobile/pubspec.yaml apps/mobile/pubspec.lock
git commit -m "build(mobile): add flutter_markdown_plus + flutter_highlight"
```

---

### Task 10.2: `MessageBubble` widget

**Files:**
- Create: `apps/mobile/lib/presentation/widgets/chat/message_bubble.dart`

- [ ] **Step 1: Implement**

```dart
import 'package:flutter/material.dart';
import 'package:flutter_markdown_plus/flutter_markdown_plus.dart';

class MessageBubble extends StatelessWidget {
  final bool isUser;
  final String markdownContent;
  final bool isStreaming;
  const MessageBubble({super.key, required this.isUser,
                        required this.markdownContent, this.isStreaming = false});

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final bg = isUser ? theme.colorScheme.primaryContainer : theme.colorScheme.surface;
    final align = isUser ? Alignment.centerRight : Alignment.centerLeft;
    return Align(
      alignment: align,
      child: ConstrainedBox(
        constraints: BoxConstraints(maxWidth: MediaQuery.of(context).size.width * 0.85),
        child: Container(
          margin: const EdgeInsets.symmetric(vertical: 4, horizontal: 8),
          padding: const EdgeInsets.all(12),
          decoration: BoxDecoration(color: bg, borderRadius: BorderRadius.circular(12)),
          child: Column(crossAxisAlignment: CrossAxisAlignment.start, children: [
            MarkdownBody(data: markdownContent, selectable: true),
            if (isStreaming) const _StreamingCursor(),
          ]),
        ),
      ),
    );
  }
}

class _StreamingCursor extends StatefulWidget {
  const _StreamingCursor();
  @override
  State<_StreamingCursor> createState() => _StreamingCursorState();
}

class _StreamingCursorState extends State<_StreamingCursor>
    with SingleTickerProviderStateMixin {
  late final AnimationController _ctl = AnimationController(
    vsync: this, duration: const Duration(milliseconds: 700))..repeat(reverse: true);
  @override
  void dispose() { _ctl.dispose(); super.dispose(); }
  @override
  Widget build(BuildContext context) => FadeTransition(
    opacity: _ctl,
    child: Container(width: 8, height: 16, color: Theme.of(context).colorScheme.primary),
  );
}
```

- [ ] **Step 2: Widget test**

```dart
testWidgets('streaming cursor shows when isStreaming', (tester) async {
  await tester.pumpWidget(const MaterialApp(home:
    MessageBubble(isUser: false, markdownContent: 'Hi', isStreaming: true)));
  expect(find.byType(FadeTransition), findsOneWidget);
});
```

- [ ] **Step 3: Run + commit**

```bash
fvm flutter test test/widget/message_bubble_test.dart
git add apps/mobile/lib/presentation/widgets/chat/message_bubble.dart \
        apps/mobile/test/widget/message_bubble_test.dart
git commit -m "feat(mobile): MessageBubble widget"
```

---

### Task 10.3: `StreamingText` accumulator widget

**Files:**
- Create: `apps/mobile/lib/presentation/widgets/chat/streaming_text.dart`

- [ ] **Step 1: Implement** — accumulates `UiEventMessage_TextDelta` events for a given `messageId`. The widget owns no logic for which events to consume; the parent passes `accumulatedText` and `isComplete`.

```dart
import 'package:flutter/material.dart';
import 'message_bubble.dart';

class StreamingText extends StatelessWidget {
  final String messageId;
  final String accumulatedText;
  final bool isComplete;
  const StreamingText({super.key, required this.messageId,
                       required this.accumulatedText, required this.isComplete});

  @override
  Widget build(BuildContext context) {
    return MessageBubble(
      isUser: false,
      markdownContent: accumulatedText.isEmpty ? '_thinking…_' : accumulatedText,
      isStreaming: !isComplete,
    );
  }
}
```

- [ ] **Step 2: Test + commit**

```bash
fvm flutter test test/widget/streaming_text_test.dart
git add apps/mobile/lib/presentation/widgets/chat/streaming_text.dart \
        apps/mobile/test/widget/streaming_text_test.dart
git commit -m "feat(mobile): StreamingText widget"
```

---

### Task 10.4: `ReasoningSection`, `ToolCallCard`, `MessageMetaRow`

**Files:**
- Create: `apps/mobile/lib/presentation/widgets/chat/reasoning_section.dart`
- Create: `apps/mobile/lib/presentation/widgets/chat/tool_call_card.dart`
- Create: `apps/mobile/lib/presentation/widgets/chat/message_meta_row.dart`

- [ ] **Step 1: Reasoning** — collapsible card with `ExpansionTile`, body is monospace italic.

- [ ] **Step 2: ToolCall** — collapsed by default, shows tool name + status icon. Expanded shows args (pretty-printed JSON), then output. Use `flutter_highlight` for JSON.

- [ ] **Step 3: MetaRow** — small row with timestamp + agent model name; styled as caption.

- [ ] **Step 4: Commit**

```bash
git add apps/mobile/lib/presentation/widgets/chat/
git commit -m "feat(mobile): chat reasoning + tool-call + meta widgets"
```

---

### Task 10.5: `InputBar` widget

**Files:**
- Create: `apps/mobile/lib/presentation/widgets/chat/input_bar.dart`

- [ ] **Step 1: Implement**

```dart
import 'package:flutter/material.dart';
import 'package:shadcn_ui/shadcn_ui.dart';

import '../../../domain/active_session.dart';

class InputBar extends StatefulWidget {
  final ActiveSession session;
  final ValueChanged<String> onSend;
  final VoidCallback onStop;
  const InputBar({super.key, required this.session, required this.onSend, required this.onStop});
  @override
  State<InputBar> createState() => _InputBarState();
}

class _InputBarState extends State<InputBar> {
  final _ctl = TextEditingController();
  static const _maxChars = 8000;

  bool get _canSend {
    final s = widget.session;
    return (s is SessionIdle || s is SessionAwaitingInput || s is SessionStopped)
        && _ctl.text.trim().isNotEmpty
        && _ctl.text.length <= _maxChars;
  }

  bool get _isStreaming => widget.session is SessionStreaming || widget.session is SessionStarting;

  @override
  Widget build(BuildContext context) {
    return Padding(
      padding: const EdgeInsets.all(8),
      child: Row(children: [
        Expanded(child: ShadInput(
          controller: _ctl,
          placeholder: const Text('Message…'),
          maxLines: 4,
          minLines: 1,
          onChanged: (_) => setState(() {}),
        )),
        const SizedBox(width: 8),
        if (_isStreaming)
          ShadButton.destructive(onPressed: widget.onStop, child: const Text('Stop'))
        else
          ShadButton(
            enabled: _canSend,
            onPressed: () {
              widget.onSend(_ctl.text);
              _ctl.clear();
              setState(() {});
            },
            child: const Text('Send'),
          ),
      ]),
    );
  }
}
```

- [ ] **Step 2: Test**

`test/widget/input_bar_test.dart`:
- Empty text → Send disabled.
- Streaming session → Stop button shown instead of Send.
- 8000+ chars → Send disabled.

- [ ] **Step 3: Run + commit**

```bash
fvm flutter test test/widget/input_bar_test.dart
git add apps/mobile/lib/presentation/widgets/chat/input_bar.dart \
        apps/mobile/test/widget/input_bar_test.dart
git commit -m "feat(mobile): InputBar widget gated on ActiveSession"
```

---

### Task 10.6: Rework `ThreadViewPage`

**Files:**
- Modify: `apps/mobile/lib/presentation/pages/thread_view_page.dart`

- [ ] **Step 1: Rebuild as bubble list + input bar**

```dart
class ThreadViewPage extends ConsumerStatefulWidget {
  final String? threadId;  // null = new thread (Idle)
  const ThreadViewPage({super.key, this.threadId});
  @override
  ConsumerState<ThreadViewPage> createState() => _ThreadViewPageState();
}

class _ThreadViewPageState extends ConsumerState<ThreadViewPage> {
  final _scrollCtl = ScrollController();
  bool _stickToBottom = true;

  @override
  Widget build(BuildContext context) {
    final session = ref.watch(activeSessionControllerProvider);
    final threadId = (session as dynamic).threadId as String? ?? widget.threadId;
    final eventsAsync = threadId == null
        ? const AsyncValue.data(<UiEventMessage>[])
        : ref.watch(threadEventsProvider(threadId));

    return Scaffold(
      appBar: AppBar(title: Text(threadId == null ? 'New chat' : 'Thread')),
      body: Column(children: [
        Expanded(child: eventsAsync.when(
          loading: () => const Center(child: CircularProgressIndicator()),
          error: (e, _) => Center(child: Text('Error: $e')),
          data: (events) => _buildList(events),
        )),
        InputBar(
          session: session,
          onSend: (text) {
            if (session is SessionIdle || session is SessionStopped) {
              ref.read(activeSessionControllerProvider.notifier)
                  .start(agent: AgentName.codex, prompt: text);
            } else {
              ref.read(activeSessionControllerProvider.notifier).send(text);
            }
          },
          onStop: () => ref.read(activeSessionControllerProvider.notifier).stop(),
        ),
      ]),
    );
  }

  Widget _buildList(List<UiEventMessage> events) {
    // Group events into bubbles by message_id; render via MessageBubble /
    // StreamingText / ReasoningSection / ToolCallCard.
    // (Detail in implementation; the spec leaves this open.)
    return ListView.builder(
      controller: _scrollCtl,
      itemCount: events.length,
      itemBuilder: (_, i) => /* dispatch by event variant */ const SizedBox(),
    );
  }
}
```

- [ ] **Step 2: Implement event-to-widget dispatch**

Pseudo:
- `MessageStarted{role:user}` → opens a user bubble that consumes subsequent `TextDelta` for that `message_id`.
- `MessageStarted{role:assistant}` → opens a `StreamingText` for that `message_id`.
- `TextDelta` → append to the open bubble for `message_id`.
- `MessageCompleted` → finalizes the bubble.
- `ReasoningDelta` → append into a `ReasoningSection` for that `message_id`.
- `ToolCallPlaced` → push a `ToolCallCard`. `ToolCallCompleted` → mark it complete.
- `ThreadClosed` → render a divider.
- `Error` → render in destructive style.

- [ ] **Step 3: Auto-scroll behavior**

If user is within 120 px of the bottom, auto-scroll on new messages. Else preserve position and show a "↓ N new" floating button.

- [ ] **Step 4: Widget test**

`test/widget/thread_view_page_test.dart`:
- Empty events + Idle session → input visible, bubble list empty.
- Stream `TextDelta`s → bubble accumulates text.
- `MessageCompleted` → cursor disappears.

- [ ] **Step 5: Run + commit**

```bash
fvm flutter test test/widget/thread_view_page_test.dart
git add apps/mobile/lib/presentation/pages/thread_view_page.dart \
        apps/mobile/test/widget/thread_view_page_test.dart
git commit -m "feat(mobile): ThreadViewPage rework with bubbles + streaming + input"
```

---

### Task 10.7: Rework `ThreadListPage`

**Files:**
- Modify: `apps/mobile/lib/presentation/pages/thread_list_page.dart`

- [ ] **Step 1: Add new-thread CTA**

A floating action button or app-bar action that pushes `ThreadViewPage(threadId: null)`.

- [ ] **Step 2: Add Mac-offline banner**

Watch `connectionStateProvider`; render a top banner when not connected.

- [ ] **Step 3: Add account settings entry**

App-bar `IconButton` opening `AccountSettingsPage` (Task 11.1).

- [ ] **Step 4: Run + commit**

```bash
fvm flutter test test/widget/thread_list_page_test.dart
git add apps/mobile/lib/presentation/pages/thread_list_page.dart \
        apps/mobile/test/widget/thread_list_page_test.dart
git commit -m "feat(mobile): ThreadListPage rework + new-thread CTA + offline banner"
```

---

## Phase 11: Account Settings + Lifecycle

### Task 11.1: `AccountSettingsPage`

**Files:**
- Create: `apps/mobile/lib/presentation/pages/account_settings_page.dart`

- [ ] **Step 1: Implement (email + version + logout)**

```dart
class AccountSettingsPage extends ConsumerWidget {
  const AccountSettingsPage({super.key});
  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final authState = ref.watch(authControllerProvider);
    final email = (authState is AuthAuthenticated) ? authState.account.email : '—';
    return Scaffold(
      appBar: AppBar(title: const Text('Account')),
      body: ListView(children: [
        ListTile(title: const Text('Email'), subtitle: Text(email)),
        ListTile(title: const Text('Version'), subtitle: Text(_appVersion)),
        const Divider(),
        ListTile(
          leading: const Icon(Icons.logout, color: Colors.red),
          title: const Text('Log out', style: TextStyle(color: Colors.red)),
          onTap: () async {
            await ref.read(authControllerProvider.notifier).logout();
            if (context.mounted) Navigator.of(context).pop();
          },
        ),
      ]),
    );
  }
}
```

(Source `_appVersion` from `package_info_plus` or hard-code from `pubspec.yaml` for MVP.)

- [ ] **Step 2: Commit**

```bash
git add apps/mobile/lib/presentation/pages/account_settings_page.dart
git commit -m "feat(mobile): AccountSettingsPage with email + logout"
```

---

### Task 11.2: `WidgetsBindingObserver` lifecycle wiring

**Files:**
- Modify: `apps/mobile/lib/presentation/app.dart`

- [ ] **Step 1: Convert `MinosApp` to ConsumerStatefulWidget + observer**

```dart
class MinosApp extends ConsumerStatefulWidget { const MinosApp({super.key}); @override ConsumerState<MinosApp> createState() => _MinosAppState(); }

class _MinosAppState extends ConsumerState<MinosApp> with WidgetsBindingObserver {
  @override
  void initState() {
    super.initState();
    WidgetsBinding.instance.addObserver(this);
  }
  @override
  void dispose() {
    WidgetsBinding.instance.removeObserver(this);
    super.dispose();
  }
  @override
  void didChangeAppLifecycleState(AppLifecycleState state) {
    final core = ref.read(minosCoreProvider);
    switch (state) {
      case AppLifecycleState.resumed:
        core.notifyForegrounded();
      case AppLifecycleState.paused:
      case AppLifecycleState.inactive:
      case AppLifecycleState.detached:
      case AppLifecycleState.hidden:
        core.notifyBackgrounded();
    }
  }
  @override
  Widget build(BuildContext context) => /* existing ShadApp wiring */;
}
```

- [ ] **Step 2: Commit**

```bash
git add apps/mobile/lib/presentation/app.dart
git commit -m "feat(mobile): wire WidgetsBindingObserver to notify_*grounded"
```

---

### Task 11.3: First-run migration handling

**Files:**
- Modify: `apps/mobile/lib/infrastructure/minos_core.dart`
- Modify: `apps/mobile/lib/application/auth_provider.dart`

- [ ] **Step 1: After login, check `device.account_id` consistency**

After `AuthAuthenticated` lands and the WS connects, call a backend introspection endpoint (or read from a `me` response embedded in the login response — extend the response type if needed) to get `device.account_id`. If it differs from any stale local pairing identity, call `forgetPeer()`.

> **Implementation note**: simplest path is to embed `device_account_id` in the `AuthResponse` server-side and check it Dart-side on login.

- [ ] **Step 2: Test the cross-account scenario**

Add an integration test: pair, log out, log in as a different account → existing pairing is dropped, route to pairing.

- [ ] **Step 3: Commit**

```bash
git add apps/mobile/lib/infrastructure/minos_core.dart \
        apps/mobile/lib/application/auth_provider.dart
git commit -m "feat(mobile): drop stale pairing on account switch"
```

---

## Phase 12: Tooling + Verification

### Task 12.1: `fake-peer` clap subcommands

**Files:**
- Modify: `crates/minos-mobile/src/bin/fake-peer.rs`

- [ ] **Step 1: Restructure to subcommands**

```rust
#[derive(clap::Parser)]
#[command(name = "fake-peer")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(clap::Subcommand)]
enum Cmd {
    /// Pair-only (current behavior).
    Pair { #[arg(long)] backend: String, #[arg(long)] token: String, #[arg(long)] device_name: String },
    /// Register an account, then pair, then exit.
    Register { #[arg(long)] backend: String, #[arg(long)] email: String, #[arg(long)] password: String, #[arg(long)] token: String, #[arg(long)] device_name: String },
    /// Drive a smoke session: register/login → pair → start_agent → send prompt → tail UiEvents.
    SmokeSession { #[arg(long)] backend: String, #[arg(long)] email: String, #[arg(long)] password: String, #[arg(long)] prompt: String, #[arg(long)] device_name: String },
}
```

- [ ] **Step 2: Implement `Register` subcommand**

POST `/v1/auth/register` with backend, then existing pair flow.

- [ ] **Step 3: Implement `SmokeSession`**

Login (or register if first run), pair if not paired, open WS, drive `start_agent` via `MobileClient.start_agent`, print received `UiEventFrame`s.

- [ ] **Step 4: Run + commit**

```bash
cargo build -p minos-mobile --bin fake-peer --features cli
git add crates/minos-mobile/src/bin/fake-peer.rs
git commit -m "feat(fake-peer): subcommands for register + smoke-session"
```

---

### Task 12.2: e2e integration test

**Files:**
- Create: `crates/minos-mobile/tests/e2e_register_login_dispatch_start_agent.rs`

- [ ] **Step 1: Write**

Mirror the structure of `tests/envelope_client.rs` — spin in-process backend (`spawn_backend_with_paired_mac` style helper) plus an in-process fake Mac handler that echoes back synthetic `Forwarded` replies for `minos_start_agent`. Drive `MobileClient.register → pair_with_qr_json → start_agent` end-to-end. Assert the response contains the synthetic `session_id`.

- [ ] **Step 2: Run + commit**

```bash
cargo test -p minos-mobile --test e2e_register_login_dispatch_start_agent
git add crates/minos-mobile/tests/e2e_register_login_dispatch_start_agent.rs
git commit -m "test(mobile): e2e register → pair → start_agent"
```

---

### Task 12.3: README update

**Files:**
- Modify: `README.md`

- [ ] **Step 1: Add a "Mobile login + agent session" section**

Describe: register → pair → start_agent flow, and the `MINOS_JWT_SECRET` env var requirement for `minos-backend` startup.

- [ ] **Step 2: Commit**

```bash
git add README.md
git commit -m "docs: README — mobile login + agent session"
```

---

### Task 12.4: Workspace gate green

- [ ] **Step 1: Run check-all**

```bash
cargo xtask check-all
```

Expected: green. Per memory, this is the workspace-level gate that catches frb drift, fmt, clippy, sqlx prepare, Flutter tests, etc.

- [ ] **Step 2: If anything fails — fix in-place + recommit, do not amend**

Per CLAUDE.md, always create new commits rather than amending.

---

### Task 12.5: Real-device smoke checklist (manual)

This is the sole functional gate per spec §9.5. Run on a physical iPhone + a real Mac running `minos-daemon`. Each box must be ticked.

- [ ] Fresh install → register new email → auto-login → pairing page
- [ ] `cargo run -p minos-daemon -- start --print-qr` → iPhone scans → pair OK
- [ ] Send "Hello" → see codex streamed reply, characters land progressively
- [ ] Tap Stop mid-stream → stream halts, session enters Stopped
- [ ] Send follow-up prompt → next streaming turn
- [ ] Background 30 s → foreground → WS reconnects (banner flashes)
- [ ] Force-quit app → reopen → auto-login → previous thread visible
- [ ] Login same account on second iPhone → first iPhone bumped to login within ~2 s
- [ ] Mac: stop daemon → iPhone shows "Mac 已离线" banner, input disabled
- [ ] Mac: start daemon → iPhone pull-to-refresh → banner clears
- [ ] Settings → Logout → routed to login → secure storage cleared
- [ ] Three wrong passwords on login → 429 → button countdown
- [ ] Airplane mode → banner "重连中" → restore network → auto-recovery

If any item fails, file a follow-up bug — do **not** ship until all 13 pass.

---

## Final Acceptance

Per spec §10:

- [ ] cargo xtask check-all green
- [ ] All Phase 1–12 tasks committed
- [ ] All listed Rust + Flutter tests pass in CI
- [ ] No stray `MINOS_JWT_SECRET` panics in CI fixture
- [ ] All 13 manual smoke items pass
- [ ] README updated
- [ ] Spec doc unchanged on main; this plan committed under `docs/superpowers/plans/`

When all boxes are checked and the worktree's branch is in sync with reviewers' expectations, run the **superpowers:finishing-a-development-branch** skill to choose merge / PR / cleanup.
