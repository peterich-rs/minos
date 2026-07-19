import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';

import 'package:minos/data/repositories/runtime_repository.dart';

final runtimeActionsProvider = Provider<RuntimeActions>((ref) {
  return RuntimeActions(ref);
});

class RuntimeActions {
  RuntimeActions(this._ref);

  final Ref _ref;

  void notifyAppLifecycle(AppLifecycleState state) {
    final repository = _ref.read(runtimeRepositoryProvider);
    switch (state) {
      case AppLifecycleState.resumed:
        repository.notifyForegrounded();
      case AppLifecycleState.paused:
      case AppLifecycleState.inactive:
      case AppLifecycleState.detached:
      case AppLifecycleState.hidden:
        repository.notifyBackgrounded();
    }
  }

  Future<void> forgetHost(String hostDeviceId) async {
    await _ref.read(runtimeRepositoryProvider).forgetHost(hostDeviceId);
  }

  Future<void> writeHostSkillConfig({
    required String hostDeviceId,
    required String path,
    required bool enabled,
  }) async {
    await _ref
        .read(runtimeRepositoryProvider)
        .writeHostSkillConfig(
          hostDeviceId: hostDeviceId,
          path: path,
          enabled: enabled,
        );
  }
}
