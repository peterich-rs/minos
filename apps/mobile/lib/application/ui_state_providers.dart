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

@Riverpod(keepAlive: false)
class LoginPageStateController extends _$LoginPageStateController {
  @override
  LoginPageState build() => const LoginPageState();

  void seedInitialError(Object? error) {
    if (state.didHydrateInitialError) return;
    state = state.copyWith(error: error, didHydrateInitialError: true);
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
    this.didHydrateInitialError = false,
  });

  final LoginPageMode mode;
  final bool inFlight;
  final Object? error;
  final bool didHydrateInitialError;

  LoginPageState copyWith({
    LoginPageMode? mode,
    bool? inFlight,
    Object? error = _loginPageErrorUnchanged,
    bool? didHydrateInitialError,
  }) {
    return LoginPageState(
      mode: mode ?? this.mode,
      inFlight: inFlight ?? this.inFlight,
      error: identical(error, _loginPageErrorUnchanged) ? this.error : error,
      didHydrateInitialError:
          didHydrateInitialError ?? this.didHydrateInitialError,
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
