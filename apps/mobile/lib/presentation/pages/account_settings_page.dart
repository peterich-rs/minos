import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';

import 'package:minos/application/auth_provider.dart';
import 'package:minos/domain/auth_state.dart';

/// Account settings: shows the signed-in email + app version, and exposes
/// a destructive "log out" entry that calls [AuthController.logout] and
/// pops back to the previous route.
///
/// Phase 11.1 — minimal MVP. The version string is sourced from the
/// `pubspec.yaml` `version:` field; we don't take a `package_info_plus`
/// dep for the first cut so this surface stays cheap. When/if the spec
/// asks for build numbers + platform metadata, swap in `package_info_plus`
/// without changing this widget's API.
const String _appVersion = '1.0.0';

class AccountSettingsPage extends ConsumerWidget {
  const AccountSettingsPage({super.key});

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final authState = ref.watch(authControllerProvider);
    final email = authState is AuthAuthenticated
        ? authState.account.email
        : '—';

    return Scaffold(
      appBar: AppBar(title: const Text('账户')),
      body: ListView(
        children: [
          ListTile(title: const Text('邮箱'), subtitle: Text(email)),
          ListTile(title: const Text('版本'), subtitle: const Text(_appVersion)),
          const Divider(),
          ListTile(
            leading: const Icon(Icons.logout, color: Colors.red),
            title: const Text('退出登录', style: TextStyle(color: Colors.red)),
            onTap: () => _logout(context, ref),
          ),
        ],
      ),
    );
  }

  Future<void> _logout(BuildContext context, WidgetRef ref) async {
    await ref.read(authControllerProvider.notifier).logout();
    if (context.mounted) {
      Navigator.of(context).pop();
    }
  }
}
