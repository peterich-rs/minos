/// Data Layer — Repositories
///
/// Repositories consume one or more Services, transform raw API models
/// into clean Domain Models, and handle caching / offline sync / retry.
/// They expose Domain Models to the application (ViewModel) layer.
///
/// Architecture: repositories are the single source of truth for data.
/// ViewModels inject repositories via Riverpod and never call services
/// directly.
library;

export 'package:minos/data/repositories/agent_profile_repository.dart';
export 'package:minos/data/repositories/auth_repository.dart';
export 'package:minos/data/repositories/group_agent_repository.dart';
export 'package:minos/data/repositories/project_repository.dart';
export 'package:minos/data/repositories/runtime_repository.dart';
export 'package:minos/data/repositories/social_repository.dart';
export 'package:minos/data/repositories/thread_repository.dart';
