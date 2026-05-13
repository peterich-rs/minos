import 'package:minos/application/minos_providers.dart';
import 'package:minos/src/rust/api/minos.dart';
import 'package:riverpod_annotation/riverpod_annotation.dart';

part 'project_providers.g.dart';

/// Loads and caches the project list. Refresh via invalidation.
@Riverpod(keepAlive: true)
class ProjectList extends _$ProjectList {
  @override
  Future<List<ProjectSummary>> build() async {
    final core = ref.read(minosCoreProvider);
    final resp = await core.listProjects();
    return resp.projects;
  }

  Future<void> refresh() async {
    final previous = state;
    try {
      final core = ref.read(minosCoreProvider);
      final resp = await core.listProjects();
      state = AsyncValue.data(resp.projects);
    } catch (error, stackTrace) {
      if (previous.hasValue) {
        state = previous;
        Error.throwWithStackTrace(error, stackTrace);
      }
      state = AsyncValue.error(error, stackTrace);
    }
  }

  /// Create a new project and refresh the list.
  Future<ProjectSummary> createProject({
    required String name,
    required String workspaceSlug,
  }) async {
    final core = ref.read(minosCoreProvider);
    final resp = await core.createProject(
      name: name,
      workspaceSlug: workspaceSlug,
    );
    await refresh();
    return resp.project;
  }

  /// Delete a project and refresh the list.
  Future<void> deleteProject(String projectId) async {
    final core = ref.read(minosCoreProvider);
    await core.deleteProject(projectId: projectId);
    await refresh();
  }

  /// Update a project's name and refresh the list.
  Future<void> updateProject({
    required String projectId,
    required String name,
  }) async {
    final core = ref.read(minosCoreProvider);
    await core.updateProject(projectId: projectId, name: name);
    await refresh();
  }
}

/// Loads threads for a specific project.
@Riverpod(keepAlive: false)
class ProjectThreads extends _$ProjectThreads {
  @override
  Future<List<ThreadSummary>> build(String projectId) async {
    final core = ref.read(minosCoreProvider);
    final resp = await core.listProjectThreads(projectId: projectId);
    return resp.threads;
  }

  Future<void> refresh() async {
    final previous = state;
    final projectId = this.projectId;
    try {
      final core = ref.read(minosCoreProvider);
      final resp = await core.listProjectThreads(projectId: projectId);
      state = AsyncValue.data(resp.threads);
    } catch (error, stackTrace) {
      if (previous.hasValue) {
        state = previous;
        Error.throwWithStackTrace(error, stackTrace);
      }
      state = AsyncValue.error(error, stackTrace);
    }
  }
}

/// Currently selected project id. Persisted in memory for navigation.
@Riverpod(keepAlive: true)
class SelectedProject extends _$SelectedProject {
  @override
  String? build() => null;

  void select(String? projectId) {
    state = projectId;
  }
}
