/// Feature: Projects
///
/// Project management — list, create, rename, delete projects and their
/// associated threads. Discord-style project → thread hierarchy.
///
/// View Models:
///   - [ProjectList] (application/project_providers.dart)
///   - [ProjectThreads] (application/project_providers.dart)
///   - [SelectedProject] (application/project_providers.dart)
///
/// Views:
///   - [ProjectListPage]
///   - [ProjectDetailPage]
library;

export 'package:minos/application/project_providers.dart';
export 'package:minos/presentation/pages/project_detail_page.dart';
export 'package:minos/presentation/pages/project_list_page.dart';
