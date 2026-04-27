import 'dart:convert';

import 'package:flutter_test/flutter_test.dart';
import 'package:mocktail/mocktail.dart';

import 'package:minos/infrastructure/cf_access_config.dart';
import 'package:minos/infrastructure/minos_core.dart';
import 'package:minos/infrastructure/secure_pairing_store.dart';
import 'package:minos/src/rust/api/minos.dart';

class _MockMobileClient extends Mock implements MobileClient {}

class _MockSecurePairingStore extends Mock implements SecurePairingStore {}

void main() {
  late _MockMobileClient client;
  late _MockSecurePairingStore secureStore;

  setUp(() {
    client = _MockMobileClient();
    secureStore = _MockSecurePairingStore();

    when(
      () => client.subscribeState(),
    ).thenAnswer((_) => const Stream<ConnectionState>.empty());
    when(
      () => client.subscribeUiEvents(),
    ).thenAnswer((_) => const Stream<UiEventFrame>.empty());
    when(
      () => client.currentState(),
    ).thenReturn(const ConnectionState.disconnected());
  });

  test(
    'pairWithQrJson rolls back the live session when keychain persistence fails',
    () async {
      const qrJson = '{"v":2}';
      const persisted = PersistedPairingState(
        backendUrl: 'ws://127.0.0.1/devices',
        deviceId: 'dev-123',
        deviceSecret: 'sec-456',
      );
      final persistError = StateError('keychain write failed');

      when(
        () => client.pairWithQrJson(qrJson: qrJson),
      ).thenAnswer((_) async {});
      when(
        () => client.persistedPairingState(),
      ).thenAnswer((_) async => persisted);
      when(() => secureStore.saveState(persisted)).thenThrow(persistError);
      when(() => client.forgetPeer()).thenAnswer((_) async {});
      when(() => secureStore.clearAll()).thenAnswer((_) async {});

      final core = MinosCore.forTesting(
        client: client,
        secureStore: secureStore,
      );

      await expectLater(
        core.pairWithQrJson(qrJson),
        throwsA(same(persistError)),
      );

      verify(() => client.pairWithQrJson(qrJson: qrJson)).called(1);
      verify(() => client.persistedPairingState()).called(1);
      verify(() => secureStore.saveState(persisted)).called(1);
      verify(() => client.forgetPeer()).called(1);
      verify(() => secureStore.clearAll()).called(1);
    },
  );

  test('pairWithQrJson does not clear secure storage on success', () async {
    const qrJson = '{"v":2}';
    const persisted = PersistedPairingState(
      backendUrl: 'ws://127.0.0.1/devices',
      deviceId: 'dev-123',
      deviceSecret: 'sec-456',
    );

    when(() => client.pairWithQrJson(qrJson: qrJson)).thenAnswer((_) async {});
    when(
      () => client.persistedPairingState(),
    ).thenAnswer((_) async => persisted);
    when(() => secureStore.saveState(persisted)).thenAnswer((_) async {});

    final core = MinosCore.forTesting(client: client, secureStore: secureStore);

    await core.pairWithQrJson(qrJson);

    verify(() => secureStore.saveState(persisted)).called(1);
    verifyNever(() => client.forgetPeer());
    verifyNever(() => secureStore.clearAll());
  });

  test('pairWithQrJson injects build-time cf access credentials', () async {
    const qrJson =
        '{"v":2,"cf_access_client_id":"qr-id","cf_access_client_secret":"qr-secret"}';
    const persisted = PersistedPairingState(
      backendUrl: 'wss://example.com/devices',
      deviceId: 'dev-123',
      deviceSecret: 'sec-456',
    );

    when(
      () => client.pairWithQrJson(qrJson: any(named: 'qrJson')),
    ).thenAnswer((_) async {});
    when(
      () => client.persistedPairingState(),
    ).thenAnswer((_) async => persisted);
    when(() => secureStore.saveState(persisted)).thenAnswer((_) async {});

    final core = MinosCore.forTesting(
      client: client,
      secureStore: secureStore,
      cfAccessConfig: CfAccessConfig(
        clientId: 'build-id',
        clientSecret: 'build-secret',
      ),
    );

    await core.pairWithQrJson(qrJson);

    final captured =
        verify(
              () => client.pairWithQrJson(qrJson: captureAny(named: 'qrJson')),
            ).captured.single
            as String;
    final injected = jsonDecode(captured) as Map<String, Object?>;
    expect(injected['cf_access_client_id'], 'build-id');
    expect(injected['cf_access_client_secret'], 'build-secret');
    verify(() => secureStore.saveState(persisted)).called(1);
  });

  test('pairWithQrJson preserves qr-carried cf access credentials', () async {
    const qrJson =
        '{"v":2,"cf_access_client_id":"qr-id","cf_access_client_secret":"qr-secret"}';
    const persisted = PersistedPairingState(
      backendUrl: 'wss://example.com/devices',
      deviceId: 'dev-123',
      deviceSecret: 'sec-456',
      cfAccessClientId: 'qr-id',
      cfAccessClientSecret: 'qr-secret',
    );

    when(
      () => client.pairWithQrJson(qrJson: any(named: 'qrJson')),
    ).thenAnswer((_) async {});
    when(
      () => client.persistedPairingState(),
    ).thenAnswer((_) async => persisted);
    when(() => secureStore.saveState(persisted)).thenAnswer((_) async {});

    final core = MinosCore.forTesting(client: client, secureStore: secureStore);

    await core.pairWithQrJson(qrJson);

    final captured =
        verify(
              () => client.pairWithQrJson(qrJson: captureAny(named: 'qrJson')),
            ).captured.single
            as String;
    final preserved = jsonDecode(captured) as Map<String, Object?>;
    expect(preserved['cf_access_client_id'], 'qr-id');
    expect(preserved['cf_access_client_secret'], 'qr-secret');
    verify(() => secureStore.saveState(persisted)).called(1);
  });

  group('resolveClient', () {
    test(
      'returns a freshly built client when no persisted state is present',
      () async {
        final freshClient = _MockMobileClient();
        when(() => secureStore.loadState()).thenAnswer((_) async => null);

        final result = await MinosCore.resolveClient(
          secure: secureStore,
          buildFresh: () => freshClient,
          buildFromPersisted: (_) {
            fail('buildFromPersisted must not run when no snapshot is loaded');
          },
        );

        expect(result, same(freshClient));
        verify(() => secureStore.loadState()).called(1);
        verifyNever(() => secureStore.clearAll());
      },
    );

    test('returns the rehydrated client when resume succeeds', () async {
      const persisted = PersistedPairingState(
        backendUrl: 'ws://127.0.0.1/devices',
        deviceId: 'dev-123',
        deviceSecret: 'sec-456',
        accessToken: 'access',
        accessExpiresAtMs: 1700000000000,
        refreshToken: 'refresh',
        accountId: 'acc',
        accountEmail: 'u@example.com',
      );
      final rehydrated = _MockMobileClient();
      when(() => secureStore.loadState()).thenAnswer((_) async => persisted);
      when(() => rehydrated.resumePersistedSession()).thenAnswer((_) async {});

      final result = await MinosCore.resolveClient(
        secure: secureStore,
        buildFresh: () => fail('buildFresh must not run when resume succeeds'),
        buildFromPersisted: (state) {
          expect(state, persisted);
          return rehydrated;
        },
      );

      expect(result, same(rehydrated));
      verify(() => rehydrated.resumePersistedSession()).called(1);
      verifyNever(() => secureStore.clearAll());
    });

    test(
      'skips resumePersistedSession when persisted snapshot has no auth tuple',
      () async {
        // Phase 8.9: paired-but-logged-out cold launch must not poke the
        // WS — let the AuthController drive resume after the user logs in.
        const persisted = PersistedPairingState(
          backendUrl: 'ws://127.0.0.1/devices',
          deviceId: 'dev-paired',
          deviceSecret: 'sec-paired',
        );
        final rehydrated = _MockMobileClient();
        when(() => secureStore.loadState()).thenAnswer((_) async => persisted);

        final result = await MinosCore.resolveClient(
          secure: secureStore,
          buildFresh: () => fail('buildFresh must not run when paired'),
          buildFromPersisted: (state) {
            expect(state, persisted);
            return rehydrated;
          },
        );

        expect(result, same(rehydrated));
        verifyNever(() => rehydrated.resumePersistedSession());
        verifyNever(() => secureStore.clearAll());
      },
    );

    test(
      'wipes secure storage and returns a fresh client when resume is revoked',
      () async {
        const persisted = PersistedPairingState(
          backendUrl: 'ws://127.0.0.1/devices',
          deviceId: 'dev-stale',
          deviceSecret: 'sec-revoked',
          accessToken: 'access',
          accessExpiresAtMs: 1700000000000,
          refreshToken: 'refresh',
          accountId: 'acc',
          accountEmail: 'u@example.com',
        );
        final rehydrated = _MockMobileClient();
        final freshClient = _MockMobileClient();
        when(() => secureStore.loadState()).thenAnswer((_) async => persisted);
        when(() => secureStore.clearAll()).thenAnswer((_) async {});
        when(() => rehydrated.resumePersistedSession()).thenAnswer(
          (_) async =>
              throw const MinosError.deviceNotTrusted(deviceId: 'dev-stale'),
        );

        final result = await MinosCore.resolveClient(
          secure: secureStore,
          buildFresh: () => freshClient,
          buildFromPersisted: (state) {
            expect(state, persisted);
            return rehydrated;
          },
        );

        expect(
          result,
          same(freshClient),
          reason: 'returned client must come from buildFresh after recovery',
        );
        verify(() => rehydrated.resumePersistedSession()).called(1);
        verify(() => secureStore.clearAll()).called(1);
      },
    );

    test(
      'keeps persisted pairing when resume fails due to transient connection loss',
      () async {
        const persisted = PersistedPairingState(
          backendUrl: 'ws://127.0.0.1/devices',
          deviceId: 'dev-123',
          deviceSecret: 'sec-456',
          accessToken: 'access',
          accessExpiresAtMs: 1700000000000,
          refreshToken: 'refresh',
          accountId: 'acc',
          accountEmail: 'u@example.com',
        );
        final rehydrated = _MockMobileClient();
        when(() => secureStore.loadState()).thenAnswer((_) async => persisted);
        when(() => rehydrated.resumePersistedSession()).thenAnswer(
          (_) async => throw const MinosError.connectFailed(
            url: 'ws://127.0.0.1/devices',
            message: 'ConnectionRefused',
          ),
        );

        final result = await MinosCore.resolveClient(
          secure: secureStore,
          buildFresh: () => fail('transient resume failure must stay paired'),
          buildFromPersisted: (state) {
            expect(state, persisted);
            return rehydrated;
          },
        );

        expect(result, same(rehydrated));
        verify(() => rehydrated.resumePersistedSession()).called(1);
        verifyNever(() => secureStore.clearAll());
      },
    );
  });

  group('hasPersistedPairing', () {
    test('returns true when secure storage has a resumable snapshot', () async {
      when(() => secureStore.loadState()).thenAnswer(
        (_) async => const PersistedPairingState(
          backendUrl: 'ws://127.0.0.1/devices',
          deviceId: 'dev-123',
          deviceSecret: 'sec-456',
        ),
      );

      final core = MinosCore.forTesting(
        client: client,
        secureStore: secureStore,
      );

      expect(await core.hasPersistedPairing(), isTrue);
    });

    test('returns false when secure storage is empty', () async {
      when(() => secureStore.loadState()).thenAnswer((_) async => null);

      final core = MinosCore.forTesting(
        client: client,
        secureStore: secureStore,
      );

      expect(await core.hasPersistedPairing(), isFalse);
    });
  });
}
