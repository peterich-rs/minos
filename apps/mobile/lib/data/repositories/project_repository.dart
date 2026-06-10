import 'package:flutter_riverpod/flutter_riverpod.dart';

import 'package:minos/data/services/minos_core_service.dart';
import 'package:minos/domain/minos_core_protocol.dart';
import 'package:minos/src/rust/api/minos.dart';

final projectRepositoryProvider = Provider<ProjectRepository>((ref) {
  return ProjectRepository(ref.watch(minosCoreServiceProvider));
});

class ProjectRepository {
  const ProjectRepository(this._core);

  final MinosCoreProtocol _core;

  Future<List<ProjectSummary>> listProjects() async {
    final response = await _core.listProjects();
    return response.projects;
  }

  Future<ProjectSummary> createProject({
    required String name,
    required String workspaceSlug,
    String? workspacePath,
  }) async {
    final response = await _core.createProject(
      name: name,
      workspaceSlug: workspaceSlug,
      workspacePath: workspacePath,
    );
    return response.project;
  }

  Future<void> deleteProject(String projectId) {
    return _core.deleteProject(projectId: projectId);
  }

  Future<void> updateProject({
    required String projectId,
    required String name,
  }) {
    return _core.updateProject(projectId: projectId, name: name);
  }

  Future<List<ThreadSummary>> listProjectThreads(String projectId) async {
    final response = await _core.listProjectThreads(projectId: projectId);
    return response.threads;
  }
}
