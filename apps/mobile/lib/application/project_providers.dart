import 'package:minos/data/repositories/project_repository.dart';
import 'package:minos/src/rust/api/minos.dart';
import 'package:riverpod_annotation/riverpod_annotation.dart';

part 'project_providers.g.dart';

/// Loads and caches the project list. Refresh via invalidation.
@Riverpod(keepAlive: true)
class ProjectList extends _$ProjectList {
  @override
  Future<List<ProjectSummary>> build() async {
    return ref.read(projectRepositoryProvider).listProjects();
  }

  Future<void> refresh() async {
    final previous = state;
    try {
      state = AsyncValue.data(
        await ref.read(projectRepositoryProvider).listProjects(),
      );
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
    String? workspacePath,
  }) async {
    final project = await ref
        .read(projectRepositoryProvider)
        .createProject(
          name: name,
          workspaceSlug: workspaceSlug,
          workspacePath: workspacePath,
        );
    await refresh();
    return project;
  }

  /// Delete a project and refresh the list.
  Future<void> deleteProject(String projectId) async {
    await ref.read(projectRepositoryProvider).deleteProject(projectId);
    await refresh();
  }

  /// Update a project's name and refresh the list.
  Future<void> updateProject({
    required String projectId,
    required String name,
  }) async {
    await ref
        .read(projectRepositoryProvider)
        .updateProject(projectId: projectId, name: name);
    await refresh();
  }
}

/// Loads threads for a specific project.
@Riverpod(keepAlive: false)
class ProjectThreads extends _$ProjectThreads {
  @override
  Future<List<ThreadSummary>> build(String projectId) async {
    return ref.read(projectRepositoryProvider).listProjectThreads(projectId);
  }

  Future<void> refresh() async {
    final previous = state;
    final projectId = this.projectId;
    try {
      state = AsyncValue.data(
        await ref.read(projectRepositoryProvider).listProjectThreads(projectId),
      );
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
