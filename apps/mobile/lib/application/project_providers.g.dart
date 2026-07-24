// GENERATED CODE - DO NOT MODIFY BY HAND

part of 'project_providers.dart';

// **************************************************************************
// RiverpodGenerator
// **************************************************************************

// GENERATED CODE - DO NOT MODIFY BY HAND
// ignore_for_file: type=lint, type=warning
/// Loads and caches the project list. Refresh via invalidation.

@ProviderFor(ProjectList)
final projectListProvider = ProjectListProvider._();

/// Loads and caches the project list. Refresh via invalidation.
final class ProjectListProvider
    extends $AsyncNotifierProvider<ProjectList, List<ProjectSummary>> {
  /// Loads and caches the project list. Refresh via invalidation.
  ProjectListProvider._()
    : super(
        from: null,
        argument: null,
        retry: null,
        name: r'projectListProvider',
        isAutoDispose: false,
        dependencies: null,
        $allTransitiveDependencies: null,
      );

  @override
  String debugGetCreateSourceHash() => _$projectListHash();

  @$internal
  @override
  ProjectList create() => ProjectList();
}

String _$projectListHash() => r'ee5288d606b69d703648e673e0d8326831aaefbd';

/// Loads and caches the project list. Refresh via invalidation.

abstract class _$ProjectList extends $AsyncNotifier<List<ProjectSummary>> {
  FutureOr<List<ProjectSummary>> build();
  @$mustCallSuper
  @override
  void runBuild() {
    final ref =
        this.ref
            as $Ref<AsyncValue<List<ProjectSummary>>, List<ProjectSummary>>;
    final element =
        ref.element
            as $ClassProviderElement<
              AnyNotifier<
                AsyncValue<List<ProjectSummary>>,
                List<ProjectSummary>
              >,
              AsyncValue<List<ProjectSummary>>,
              Object?,
              Object?
            >;
    element.handleCreate(ref, build);
  }
}

/// Loads sessions for a specific project.

@ProviderFor(ProjectSessions)
final projectSessionsProvider = ProjectSessionsFamily._();

/// Loads sessions for a specific project.
final class ProjectSessionsProvider
    extends $AsyncNotifierProvider<ProjectSessions, List<SessionSummary>> {
  /// Loads sessions for a specific project.
  ProjectSessionsProvider._({
    required ProjectSessionsFamily super.from,
    required String super.argument,
  }) : super(
         retry: null,
         name: r'projectSessionsProvider',
         isAutoDispose: true,
         dependencies: null,
         $allTransitiveDependencies: null,
       );

  @override
  String debugGetCreateSourceHash() => _$projectSessionsHash();

  @override
  String toString() {
    return r'projectSessionsProvider'
        ''
        '($argument)';
  }

  @$internal
  @override
  ProjectSessions create() => ProjectSessions();

  @override
  bool operator ==(Object other) {
    return other is ProjectSessionsProvider && other.argument == argument;
  }

  @override
  int get hashCode {
    return argument.hashCode;
  }
}

String _$projectSessionsHash() => r'76903f9cca7df6089089b63852bd6cbcfe4ea370';

/// Loads sessions for a specific project.

final class ProjectSessionsFamily extends $Family
    with
        $ClassFamilyOverride<
          ProjectSessions,
          AsyncValue<List<SessionSummary>>,
          List<SessionSummary>,
          FutureOr<List<SessionSummary>>,
          String
        > {
  ProjectSessionsFamily._()
    : super(
        retry: null,
        name: r'projectSessionsProvider',
        dependencies: null,
        $allTransitiveDependencies: null,
        isAutoDispose: true,
      );

  /// Loads sessions for a specific project.

  ProjectSessionsProvider call(String projectId) =>
      ProjectSessionsProvider._(argument: projectId, from: this);

  @override
  String toString() => r'projectSessionsProvider';
}

/// Loads sessions for a specific project.

abstract class _$ProjectSessions extends $AsyncNotifier<List<SessionSummary>> {
  late final _$args = ref.$arg as String;
  String get projectId => _$args;

  FutureOr<List<SessionSummary>> build(String projectId);
  @$mustCallSuper
  @override
  void runBuild() {
    final ref =
        this.ref
            as $Ref<AsyncValue<List<SessionSummary>>, List<SessionSummary>>;
    final element =
        ref.element
            as $ClassProviderElement<
              AnyNotifier<
                AsyncValue<List<SessionSummary>>,
                List<SessionSummary>
              >,
              AsyncValue<List<SessionSummary>>,
              Object?,
              Object?
            >;
    element.handleCreate(ref, () => build(_$args));
  }
}

/// Currently selected project id. Persisted in memory for navigation.

@ProviderFor(SelectedProject)
final selectedProjectProvider = SelectedProjectProvider._();

/// Currently selected project id. Persisted in memory for navigation.
final class SelectedProjectProvider
    extends $NotifierProvider<SelectedProject, String?> {
  /// Currently selected project id. Persisted in memory for navigation.
  SelectedProjectProvider._()
    : super(
        from: null,
        argument: null,
        retry: null,
        name: r'selectedProjectProvider',
        isAutoDispose: false,
        dependencies: null,
        $allTransitiveDependencies: null,
      );

  @override
  String debugGetCreateSourceHash() => _$selectedProjectHash();

  @$internal
  @override
  SelectedProject create() => SelectedProject();

  /// {@macro riverpod.override_with_value}
  Override overrideWithValue(String? value) {
    return $ProviderOverride(
      origin: this,
      providerOverride: $SyncValueProvider<String?>(value),
    );
  }
}

String _$selectedProjectHash() => r'e80e496ab33e02f77a7a9b35f05befb2b4ee6b0f';

/// Currently selected project id. Persisted in memory for navigation.

abstract class _$SelectedProject extends $Notifier<String?> {
  String? build();
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
    element.handleCreate(ref, build);
  }
}
