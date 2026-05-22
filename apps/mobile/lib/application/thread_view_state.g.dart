// GENERATED CODE - DO NOT MODIFY BY HAND

part of 'thread_view_state.dart';

// **************************************************************************
// RiverpodGenerator
// **************************************************************************

// GENERATED CODE - DO NOT MODIFY BY HAND
// ignore_for_file: type=lint, type=warning

@ProviderFor(ThreadViewStateController)
final threadViewStateControllerProvider = ThreadViewStateControllerFamily._();

final class ThreadViewStateControllerProvider
    extends $NotifierProvider<ThreadViewStateController, ThreadViewState> {
  ThreadViewStateControllerProvider._({
    required ThreadViewStateControllerFamily super.from,
    required String super.argument,
  }) : super(
         retry: null,
         name: r'threadViewStateControllerProvider',
         isAutoDispose: true,
         dependencies: null,
         $allTransitiveDependencies: null,
       );

  @override
  String debugGetCreateSourceHash() => _$threadViewStateControllerHash();

  @override
  String toString() {
    return r'threadViewStateControllerProvider'
        ''
        '($argument)';
  }

  @$internal
  @override
  ThreadViewStateController create() => ThreadViewStateController();

  /// {@macro riverpod.override_with_value}
  Override overrideWithValue(ThreadViewState value) {
    return $ProviderOverride(
      origin: this,
      providerOverride: $SyncValueProvider<ThreadViewState>(value),
    );
  }

  @override
  bool operator ==(Object other) {
    return other is ThreadViewStateControllerProvider &&
        other.argument == argument;
  }

  @override
  int get hashCode {
    return argument.hashCode;
  }
}

String _$threadViewStateControllerHash() =>
    r'36fe0ed17b01122c38f03b5f0dbd490d3b2ea451';

final class ThreadViewStateControllerFamily extends $Family
    with
        $ClassFamilyOverride<
          ThreadViewStateController,
          ThreadViewState,
          ThreadViewState,
          ThreadViewState,
          String
        > {
  ThreadViewStateControllerFamily._()
    : super(
        retry: null,
        name: r'threadViewStateControllerProvider',
        dependencies: null,
        $allTransitiveDependencies: null,
        isAutoDispose: true,
      );

  ThreadViewStateControllerProvider call(String viewId) =>
      ThreadViewStateControllerProvider._(argument: viewId, from: this);

  @override
  String toString() => r'threadViewStateControllerProvider';
}

abstract class _$ThreadViewStateController extends $Notifier<ThreadViewState> {
  late final _$args = ref.$arg as String;
  String get viewId => _$args;

  ThreadViewState build(String viewId);
  @$mustCallSuper
  @override
  void runBuild() {
    final ref = this.ref as $Ref<ThreadViewState, ThreadViewState>;
    final element =
        ref.element
            as $ClassProviderElement<
              AnyNotifier<ThreadViewState, ThreadViewState>,
              ThreadViewState,
              Object?,
              Object?
            >;
    element.handleCreate(ref, () => build(_$args));
  }
}
