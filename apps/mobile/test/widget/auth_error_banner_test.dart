@Tags(['ffi'])
library;

import 'dart:io';

import 'package:flutter/material.dart';
import 'package:flutter_rust_bridge/flutter_rust_bridge_for_generated.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:shadcn_ui/shadcn_ui.dart';

import 'package:minos/domain/minos_error_display.dart';
import 'package:minos/presentation/widgets/auth/auth_error_banner.dart';
import 'package:minos/src/rust/api/minos.dart';
import 'package:minos/src/rust/frb_generated.dart';

/// See `test/unit/minos_error_display_test.dart` for the rationale —
/// AuthErrorBanner renders `MinosError.userMessage()` which calls into the
/// Rust `kindMessage` function, so the host cdylib must be loaded first.
String _hostDylibPath() {
  final workspaceRoot = Directory.current.parent.parent.path;
  final String suffix;
  if (Platform.isMacOS) {
    suffix = 'dylib';
  } else if (Platform.isWindows) {
    suffix = 'dll';
  } else {
    suffix = 'so';
  }
  final release =
      File('$workspaceRoot/target/release/libminos_ffi_frb.$suffix');
  if (release.existsSync()) return release.path;
  return '$workspaceRoot/target/debug/libminos_ffi_frb.$suffix';
}

Widget _harness(MinosError? error) {
  return ShadApp(home: Scaffold(body: AuthErrorBanner(error: error)));
}

void main() {
  setUpAll(() async {
    final path = _hostDylibPath();
    if (!File(path).existsSync()) {
      fail(
        'Missing host dylib at $path. Build it first via: '
        'cargo build -p minos-ffi-frb',
      );
    }
    await RustLib.init(externalLibrary: ExternalLibrary.open(path));
  });

  testWidgets('Renders the localized userMessage as title', (tester) async {
    const err = MinosError.invalidCredentials();
    await tester.pumpWidget(_harness(err));
    await tester.pump();

    expect(find.byType(ShadAlert), findsOneWidget);
    expect(find.text(err.userMessage()), findsOneWidget);
  });

  testWidgets('Renders detail in description when present', (tester) async {
    const err = MinosError.rateLimited(retryAfterS: 30);
    await tester.pumpWidget(_harness(err));
    await tester.pump();

    expect(find.text('retry after 30s'), findsOneWidget);
  });

  testWidgets('Auto-dismisses after 6 seconds', (tester) async {
    const err = MinosError.invalidCredentials();
    await tester.pumpWidget(_harness(err));
    await tester.pump();
    expect(find.byType(ShadAlert), findsOneWidget);

    await tester.pump(const Duration(seconds: 6));
    expect(find.byType(ShadAlert), findsNothing);
  });

  testWidgets('Null error renders nothing', (tester) async {
    await tester.pumpWidget(_harness(null));
    await tester.pump();
    expect(find.byType(ShadAlert), findsNothing);
  });
}
