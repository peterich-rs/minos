/// Data Layer — Services
///
/// Stateless classes that wrap external APIs (Rust FFI bridge, local
/// databases, platform plugins). Return raw API models.
///
/// Architecture: services sit at the bottom of the data layer and are
/// consumed by repositories. They should never be accessed directly from
/// the UI layer.
library;

export 'package:minos/infrastructure/agent_profile_store.dart';
export 'package:minos/infrastructure/app_paths.dart';
export 'package:minos/infrastructure/minos_core.dart';
export 'package:minos/infrastructure/secure_pairing_store.dart';
export 'package:minos/infrastructure/social_cache_store.dart';
export 'package:minos/infrastructure/workspace_mru_store.dart';
