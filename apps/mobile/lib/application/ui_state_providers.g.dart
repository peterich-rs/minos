// GENERATED CODE - DO NOT MODIFY BY HAND

part of 'ui_state_providers.dart';

// **************************************************************************
// RiverpodGenerator
// **************************************************************************

// GENERATED CODE - DO NOT MODIFY BY HAND
// ignore_for_file: type=lint, type=warning

@ProviderFor(ShellTabIndex)
final shellTabIndexProvider = ShellTabIndexProvider._();

final class ShellTabIndexProvider
    extends $NotifierProvider<ShellTabIndex, int> {
  ShellTabIndexProvider._()
    : super(
        from: null,
        argument: null,
        retry: null,
        name: r'shellTabIndexProvider',
        isAutoDispose: false,
        dependencies: null,
        $allTransitiveDependencies: null,
      );

  @override
  String debugGetCreateSourceHash() => _$shellTabIndexHash();

  @$internal
  @override
  ShellTabIndex create() => ShellTabIndex();

  /// {@macro riverpod.override_with_value}
  Override overrideWithValue(int value) {
    return $ProviderOverride(
      origin: this,
      providerOverride: $SyncValueProvider<int>(value),
    );
  }
}

String _$shellTabIndexHash() => r'225104474efa89fdeaa4792cbe69e7802103d6f0';

abstract class _$ShellTabIndex extends $Notifier<int> {
  int build();
  @$mustCallSuper
  @override
  void runBuild() {
    final ref = this.ref as $Ref<int, int>;
    final element =
        ref.element
            as $ClassProviderElement<
              AnyNotifier<int, int>,
              int,
              Object?,
              Object?
            >;
    element.handleCreate(ref, build);
  }
}

/// Form UX for [LoginPage] (mode / in-flight / banner error).
///
/// Initial banner error is taken from [authControllerProvider] **inside**
/// [build] (not from a widget `initState`). Writing a provider from
/// `initState` trips Riverpod's "modify while the widget tree is building"
/// assert when go_router mounts [LoginPage] after [AuthRefreshFailed].

@ProviderFor(LoginPageStateController)
final loginPageStateControllerProvider = LoginPageStateControllerProvider._();

/// Form UX for [LoginPage] (mode / in-flight / banner error).
///
/// Initial banner error is taken from [authControllerProvider] **inside**
/// [build] (not from a widget `initState`). Writing a provider from
/// `initState` trips Riverpod's "modify while the widget tree is building"
/// assert when go_router mounts [LoginPage] after [AuthRefreshFailed].
final class LoginPageStateControllerProvider
    extends $NotifierProvider<LoginPageStateController, LoginPageState> {
  /// Form UX for [LoginPage] (mode / in-flight / banner error).
  ///
  /// Initial banner error is taken from [authControllerProvider] **inside**
  /// [build] (not from a widget `initState`). Writing a provider from
  /// `initState` trips Riverpod's "modify while the widget tree is building"
  /// assert when go_router mounts [LoginPage] after [AuthRefreshFailed].
  LoginPageStateControllerProvider._()
    : super(
        from: null,
        argument: null,
        retry: null,
        name: r'loginPageStateControllerProvider',
        isAutoDispose: true,
        dependencies: null,
        $allTransitiveDependencies: null,
      );

  @override
  String debugGetCreateSourceHash() => _$loginPageStateControllerHash();

  @$internal
  @override
  LoginPageStateController create() => LoginPageStateController();

  /// {@macro riverpod.override_with_value}
  Override overrideWithValue(LoginPageState value) {
    return $ProviderOverride(
      origin: this,
      providerOverride: $SyncValueProvider<LoginPageState>(value),
    );
  }
}

String _$loginPageStateControllerHash() =>
    r'e2ef8c2a89f6a9c11c7dafb93747c2ae269fa4d4';

/// Form UX for [LoginPage] (mode / in-flight / banner error).
///
/// Initial banner error is taken from [authControllerProvider] **inside**
/// [build] (not from a widget `initState`). Writing a provider from
/// `initState` trips Riverpod's "modify while the widget tree is building"
/// assert when go_router mounts [LoginPage] after [AuthRefreshFailed].

abstract class _$LoginPageStateController extends $Notifier<LoginPageState> {
  LoginPageState build();
  @$mustCallSuper
  @override
  void runBuild() {
    final ref = this.ref as $Ref<LoginPageState, LoginPageState>;
    final element =
        ref.element
            as $ClassProviderElement<
              AnyNotifier<LoginPageState, LoginPageState>,
              LoginPageState,
              Object?,
              Object?
            >;
    element.handleCreate(ref, build);
  }
}
