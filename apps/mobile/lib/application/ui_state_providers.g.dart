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

@ProviderFor(SelectedProjectThread)
final selectedProjectThreadProvider = SelectedProjectThreadFamily._();

final class SelectedProjectThreadProvider
    extends $NotifierProvider<SelectedProjectThread, String?> {
  SelectedProjectThreadProvider._({
    required SelectedProjectThreadFamily super.from,
    required String super.argument,
  }) : super(
         retry: null,
         name: r'selectedProjectThreadProvider',
         isAutoDispose: true,
         dependencies: null,
         $allTransitiveDependencies: null,
       );

  @override
  String debugGetCreateSourceHash() => _$selectedProjectThreadHash();

  @override
  String toString() {
    return r'selectedProjectThreadProvider'
        ''
        '($argument)';
  }

  @$internal
  @override
  SelectedProjectThread create() => SelectedProjectThread();

  /// {@macro riverpod.override_with_value}
  Override overrideWithValue(String? value) {
    return $ProviderOverride(
      origin: this,
      providerOverride: $SyncValueProvider<String?>(value),
    );
  }

  @override
  bool operator ==(Object other) {
    return other is SelectedProjectThreadProvider && other.argument == argument;
  }

  @override
  int get hashCode {
    return argument.hashCode;
  }
}

String _$selectedProjectThreadHash() =>
    r'2f3c29b6e23760e3d228073d22477e95fecd7138';

final class SelectedProjectThreadFamily extends $Family
    with
        $ClassFamilyOverride<
          SelectedProjectThread,
          String?,
          String?,
          String?,
          String
        > {
  SelectedProjectThreadFamily._()
    : super(
        retry: null,
        name: r'selectedProjectThreadProvider',
        dependencies: null,
        $allTransitiveDependencies: null,
        isAutoDispose: true,
      );

  SelectedProjectThreadProvider call(String projectId) =>
      SelectedProjectThreadProvider._(argument: projectId, from: this);

  @override
  String toString() => r'selectedProjectThreadProvider';
}

abstract class _$SelectedProjectThread extends $Notifier<String?> {
  late final _$args = ref.$arg as String;
  String get projectId => _$args;

  String? build(String projectId);
  @$mustCallSuper
  @override
  void runBuild() {
    final ref = this.ref as $Ref<String?, String?>;
    final element =
        ref.element
            as $ClassProviderElement<
              AnyNotifier<String?, String?>,
              String?,
              Object?,
              Object?
            >;
    element.handleCreate(ref, () => build(_$args));
  }
}

@ProviderFor(AgentStartPageStateController)
final agentStartPageStateControllerProvider =
    AgentStartPageStateControllerProvider._();

final class AgentStartPageStateControllerProvider
    extends
        $NotifierProvider<AgentStartPageStateController, AgentStartPageState> {
  AgentStartPageStateControllerProvider._()
    : super(
        from: null,
        argument: null,
        retry: null,
        name: r'agentStartPageStateControllerProvider',
        isAutoDispose: true,
        dependencies: null,
        $allTransitiveDependencies: null,
      );

  @override
  String debugGetCreateSourceHash() => _$agentStartPageStateControllerHash();

  @$internal
  @override
  AgentStartPageStateController create() => AgentStartPageStateController();

  /// {@macro riverpod.override_with_value}
  Override overrideWithValue(AgentStartPageState value) {
    return $ProviderOverride(
      origin: this,
      providerOverride: $SyncValueProvider<AgentStartPageState>(value),
    );
  }
}

String _$agentStartPageStateControllerHash() =>
    r'1b6a694975eb755d9ea2735e1f4b2ab8beeda3c2';

abstract class _$AgentStartPageStateController
    extends $Notifier<AgentStartPageState> {
  AgentStartPageState build();
  @$mustCallSuper
  @override
  void runBuild() {
    final ref = this.ref as $Ref<AgentStartPageState, AgentStartPageState>;
    final element =
        ref.element
            as $ClassProviderElement<
              AnyNotifier<AgentStartPageState, AgentStartPageState>,
              AgentStartPageState,
              Object?,
              Object?
            >;
    element.handleCreate(ref, build);
  }
}

@ProviderFor(CollapsedProjectIds)
final collapsedProjectIdsProvider = CollapsedProjectIdsProvider._();

final class CollapsedProjectIdsProvider
    extends $NotifierProvider<CollapsedProjectIds, Set<String>> {
  CollapsedProjectIdsProvider._()
    : super(
        from: null,
        argument: null,
        retry: null,
        name: r'collapsedProjectIdsProvider',
        isAutoDispose: false,
        dependencies: null,
        $allTransitiveDependencies: null,
      );

  @override
  String debugGetCreateSourceHash() => _$collapsedProjectIdsHash();

  @$internal
  @override
  CollapsedProjectIds create() => CollapsedProjectIds();

  /// {@macro riverpod.override_with_value}
  Override overrideWithValue(Set<String> value) {
    return $ProviderOverride(
      origin: this,
      providerOverride: $SyncValueProvider<Set<String>>(value),
    );
  }
}

String _$collapsedProjectIdsHash() =>
    r'2b7767392954799b6eee1397abb9a730465836e4';

abstract class _$CollapsedProjectIds extends $Notifier<Set<String>> {
  Set<String> build();
  @$mustCallSuper
  @override
  void runBuild() {
    final ref = this.ref as $Ref<Set<String>, Set<String>>;
    final element =
        ref.element
            as $ClassProviderElement<
              AnyNotifier<Set<String>, Set<String>>,
              Set<String>,
              Object?,
              Object?
            >;
    element.handleCreate(ref, build);
  }
}
