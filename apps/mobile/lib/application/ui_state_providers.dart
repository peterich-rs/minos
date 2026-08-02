import 'package:minos/application/auth_provider.dart';
import 'package:minos/domain/auth_state.dart';
import 'package:riverpod_annotation/riverpod_annotation.dart';

part 'ui_state_providers.g.dart';

const Object _loginPageErrorUnchanged = Object();

enum LoginPageMode { login, register }

@Riverpod(keepAlive: true)
class ShellTabIndex extends _$ShellTabIndex {
  @override
  int build() => 0;

  void select(int index) {
    state = index;
  }
}

/// Form UX for [LoginPage] (mode / in-flight / banner error).
///
/// Initial banner error is taken from [authControllerProvider] **inside**
/// [build] (not from a widget `initState`). Writing a provider from
/// `initState` trips Riverpod's "modify while the widget tree is building"
/// assert when go_router mounts [LoginPage] after [AuthRefreshFailed].
@Riverpod(keepAlive: false)
class LoginPageStateController extends _$LoginPageStateController {
  @override
  LoginPageState build() {
    final authState = ref.read(authControllerProvider);
    final initialError = authState is AuthRefreshFailed
        ? authState.error
        : null;
    return LoginPageState(error: initialError);
  }

  void setMode(LoginPageMode mode) {
    if (state.inFlight && mode != state.mode) return;
    state = state.copyWith(mode: mode);
  }

  void startSubmitting() {
    state = state.copyWith(inFlight: true);
  }

  void finishSubmitting({
    required LoginPageMode mode,
    required bool clearError,
    Object? error = _loginPageErrorUnchanged,
  }) {
    state = state.copyWith(
      inFlight: false,
      mode: mode,
      error: clearError ? null : error,
    );
  }
}

class LoginPageState {
  const LoginPageState({
    this.mode = LoginPageMode.login,
    this.inFlight = false,
    this.error,
  });

  final LoginPageMode mode;
  final bool inFlight;
  final Object? error;

  LoginPageState copyWith({
    LoginPageMode? mode,
    bool? inFlight,
    Object? error = _loginPageErrorUnchanged,
  }) {
    return LoginPageState(
      mode: mode ?? this.mode,
      inFlight: inFlight ?? this.inFlight,
      error: identical(error, _loginPageErrorUnchanged) ? this.error : error,
    );
  }
}

@Riverpod(keepAlive: false)
class SelectedProjectThread extends _$SelectedProjectThread {
  @override
  String? build(String projectId) => null;

  void select(String? sessionId) {
    state = sessionId;
  }
}

@Riverpod(keepAlive: false)
class AgentStartPageStateController extends _$AgentStartPageStateController {
  @override
  AgentStartPageState build() => const AgentStartPageState();

  void selectProfile(String? profileId) {
    state = state.copyWith(selectedProfileId: profileId);
  }

  void setSubmitting(bool isSubmitting) {
    if (state.isSubmitting == isSubmitting) return;
    state = state.copyWith(isSubmitting: isSubmitting);
  }
}

class AgentStartPageState {
  const AgentStartPageState({
    this.selectedProfileId,
    this.isSubmitting = false,
  });

  final String? selectedProfileId;
  final bool isSubmitting;

  AgentStartPageState copyWith({
    String? selectedProfileId,
    bool clearSelectedProfileId = false,
    bool? isSubmitting,
  }) {
    return AgentStartPageState(
      selectedProfileId: clearSelectedProfileId
          ? null
          : selectedProfileId ?? this.selectedProfileId,
      isSubmitting: isSubmitting ?? this.isSubmitting,
    );
  }
}

@Riverpod(keepAlive: true)
class CollapsedProjectIds extends _$CollapsedProjectIds {
  @override
  Set<String> build() => <String>{};

  void toggle(String projectId) {
    final next = <String>{...state};
    if (!next.add(projectId)) {
      next.remove(projectId);
    }
    state = next;
  }
}
