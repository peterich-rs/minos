import 'dart:io';

import 'package:flutter_rust_bridge/flutter_rust_bridge_for_generated.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:minos/domain/minos_error_display.dart';
import 'package:minos/src/rust/api/minos.dart';
import 'package:minos/src/rust/frb_generated.dart';

/// Resolve the workspace-root cargo artifact for host-based unit tests.
///
/// The frb-generated loader assumes an iOS/Android framework layout; when
/// running `flutter test` on the host we instead point it at the cdylib
/// emitted by `cargo build -p minos-ffi-frb`.
String _hostDylibPath() {
  // `flutter test` runs with the package as cwd (apps/mobile).
  final workspaceRoot = Directory.current.parent.parent.path;
  // Prefer a release build if both exist; fall back to debug.
  final release = File('$workspaceRoot/target/release/libminos_ffi_frb.dylib');
  if (release.existsSync()) return release.path;
  return '$workspaceRoot/target/debug/libminos_ffi_frb.dylib';
}

void main() {
  // `kindMessage` is a pure-Rust call via frb. It does not touch UI plugins,
  // but we still need the Rust isolate running with the host dylib loaded.
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

  final cases = <({MinosError error, ErrorKind kind})>[
    (
      error: const MinosError.bindFailed(
        addr: '127.0.0.1:42',
        message: 'eaddrinuse',
      ),
      kind: ErrorKind.bindFailed,
    ),
    (
      error: const MinosError.connectFailed(
        url: 'https://example',
        message: 'timeout',
      ),
      kind: ErrorKind.connectFailed,
    ),
    (
      error: const MinosError.disconnected(reason: 'peer gone'),
      kind: ErrorKind.disconnected,
    ),
    (
      error: const MinosError.pairingTokenInvalid(),
      kind: ErrorKind.pairingTokenInvalid,
    ),
    (
      error: MinosError.pairingStateMismatch(actual: PairingState.unpaired),
      kind: ErrorKind.pairingStateMismatch,
    ),
    (
      error: const MinosError.deviceNotTrusted(deviceId: 'dev-1'),
      kind: ErrorKind.deviceNotTrusted,
    ),
    (
      error: const MinosError.storeIo(path: '/tmp', message: 'eio'),
      kind: ErrorKind.storeIo,
    ),
    (
      error: const MinosError.storeCorrupt(path: '/tmp', message: 'bad'),
      kind: ErrorKind.storeCorrupt,
    ),
    (
      error: MinosError.cliProbeTimeout(
        bin: 'claude',
        timeoutMs: BigInt.from(5000),
      ),
      kind: ErrorKind.cliProbeTimeout,
    ),
    (
      error: const MinosError.cliProbeFailed(bin: 'claude', message: 'nope'),
      kind: ErrorKind.cliProbeFailed,
    ),
    (
      error: const MinosError.rpcCallFailed(method: 'foo', message: 'err'),
      kind: ErrorKind.rpcCallFailed,
    ),
  ];

  for (final c in cases) {
    test('${c.error.runtimeType}.kind maps to ${c.kind.name}', () {
      expect(c.error.kind, c.kind);
    });

    test('${c.error.runtimeType}.userMessage(zh) is non-empty', () {
      expect(c.error.userMessage(Lang.zh), isNotEmpty);
    });

    test('${c.error.runtimeType}.userMessage(en) is non-empty', () {
      expect(c.error.userMessage(Lang.en), isNotEmpty);
    });
  }

  test('zh and en produce different copy for at least one variant', () {
    // Pick one variant and assert zh != en so the Lang parameter is proven
    // to actually wire through to the Rust side.
    const err = MinosError.pairingTokenInvalid();
    expect(err.userMessage(Lang.zh), isNot(equals(err.userMessage(Lang.en))));
  });
}
