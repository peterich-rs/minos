import 'package:flutter_test/flutter_test.dart';

import 'package:minos/application/root_route_decision.dart';
import 'package:minos/src/rust/api/minos.dart';

void main() {
  test('connected always routes to thread list', () {
    expect(
      decideRootRoute(
        connectionState: const ConnectionState.connected(),
        hasPersistedPairing: false,
      ),
      RootRoute.threads,
    );
  });

  test('disconnected with durable pairing stays on thread list', () {
    expect(
      decideRootRoute(
        connectionState: const ConnectionState.disconnected(),
        hasPersistedPairing: true,
      ),
      RootRoute.threads,
    );
  });

  test('disconnected without durable pairing routes to pairing', () {
    expect(
      decideRootRoute(
        connectionState: const ConnectionState.disconnected(),
        hasPersistedPairing: false,
      ),
      RootRoute.pairing,
    );
  });

  test('pairing state keeps the scanner surface visible', () {
    expect(
      decideRootRoute(
        connectionState: const ConnectionState.pairing(),
        hasPersistedPairing: false,
      ),
      RootRoute.pairing,
    );
  });
}
