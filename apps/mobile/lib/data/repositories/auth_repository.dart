import 'package:flutter_riverpod/flutter_riverpod.dart';

import 'package:minos/data/services/minos_core_service.dart';
import 'package:minos/domain/minos_core_protocol.dart';
import 'package:minos/src/rust/api/minos.dart' show AuthStateFrame;

final authRepositoryProvider = Provider<AuthRepository>((ref) {
  return AuthRepository(ref.watch(minosCoreServiceProvider));
});

class AuthRepository {
  const AuthRepository(this._core);

  final MinosCoreProtocol _core;

  Stream<AuthStateFrame> get authStates => _core.authStates;

  Future<void> resumePersistedSession() {
    return _core.resumePersistedSession();
  }

  Future<void> register(String email, String password) {
    return _core.register(email: email, password: password);
  }

  Future<void> login(String email, String password) {
    return _core.login(email: email, password: password);
  }

  Future<void> logout() {
    return _core.logout();
  }
}
