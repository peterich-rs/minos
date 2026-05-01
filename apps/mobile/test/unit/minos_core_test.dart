import 'package:flutter_test/flutter_test.dart';
import 'package:mocktail/mocktail.dart';

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
    'pairWithQrJson clears local state when keychain persistence fails',
    () async {
      // ADR-0020: with bearer-only auth the rollback can't atomically
      // un-pair on the server (no mac_device_id available from a failed
      // persistedPairingState). The client only wipes its own keychain
      // snapshot and re-throws.
      const qrJson = '{"v":2}';
      const persisted = PersistedPairingState(deviceId: 'dev-123');
      final persistError = StateError('keychain write failed');

      when(
        () => client.pairWithQrJson(qrJson: qrJson),
      ).thenAnswer((_) async {});
      when(
        () => client.persistedPairingState(),
      ).thenAnswer((_) async => persisted);
      when(() => secureStore.saveState(persisted)).thenThrow(persistError);
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
      verify(() => secureStore.clearAll()).called(1);
    },
  );

  test('pairWithQrJson does not clear secure storage on success', () async {
    const qrJson = '{"v":2}';
    const persisted = PersistedPairingState(deviceId: 'dev-123');

    when(() => client.pairWithQrJson(qrJson: qrJson)).thenAnswer((_) async {});
    when(
      () => client.persistedPairingState(),
    ).thenAnswer((_) async => persisted);
    when(() => secureStore.saveState(persisted)).thenAnswer((_) async {});

    final core = MinosCore.forTesting(client: client, secureStore: secureStore);

    await core.pairWithQrJson(qrJson);

    verify(() => secureStore.saveState(persisted)).called(1);
    verifyNever(() => secureStore.clearAll());
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
        deviceId: 'dev-123',
        accessToken: 'access',
        accessExpiresAtMs: 1700000000000,
        refreshToken: 'refresh',
        accountId: 'acc',
        accountEmail: 'u@example.com',
      );
      final rehydrated = _MockMobileClient();
      when(() => secureStore.loadState()).thenAnswer((_) async => persisted);
      when(() => rehydrated.refreshSession()).thenAnswer((_) async {});
      when(
        () => rehydrated.persistedPairingState(),
      ).thenAnswer((_) async => persisted);
      when(() => secureStore.saveState(persisted)).thenAnswer((_) async {});
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
      verify(() => rehydrated.refreshSession()).called(1);
      verify(() => rehydrated.resumePersistedSession()).called(1);
      verify(() => secureStore.saveState(persisted)).called(2);
      verifyNever(() => secureStore.clearAll());
    });

    test(
      'skips resumePersistedSession when persisted snapshot has no auth tuple',
      () async {
        // Phase 8.9: paired-but-logged-out cold launch must not poke the
        // WS — let the AuthController drive resume after the user logs in.
        // Post ADR-0020 a deviceId-only snapshot represents this state.
        const persisted = PersistedPairingState(deviceId: 'dev-paired');
        final rehydrated = _MockMobileClient();
        when(() => secureStore.loadState()).thenAnswer((_) async => persisted);
        when(() => rehydrated.refreshSession()).thenAnswer((_) async {});
        when(
          () => rehydrated.persistedPairingState(),
        ).thenAnswer((_) async => persisted);
        when(() => secureStore.saveState(persisted)).thenAnswer((_) async {});

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

    test('rehydrates auth-only snapshot and resumes the WS', () async {
      // Post ADR-0020: deviceId + full auth tuple is the canonical
      // logged-in shape. resolveClient drives both refreshSession and
      // resumePersistedSession.
      const persisted = PersistedPairingState(
        deviceId: 'dev-auth',
        accessToken: 'access',
        accessExpiresAtMs: 1700000000000,
        refreshToken: 'refresh',
        accountId: 'acc',
        accountEmail: 'u@example.com',
      );
      final rehydrated = _MockMobileClient();
      when(() => secureStore.loadState()).thenAnswer((_) async => persisted);
      when(() => rehydrated.refreshSession()).thenAnswer((_) async {});
      when(
        () => rehydrated.persistedPairingState(),
      ).thenAnswer((_) async => persisted);
      when(() => secureStore.saveState(persisted)).thenAnswer((_) async {});
      when(() => rehydrated.resumePersistedSession()).thenAnswer((_) async {});

      final result = await MinosCore.resolveClient(
        secure: secureStore,
        buildFresh: () => fail('buildFresh must not run with auth snapshot'),
        buildFromPersisted: (state) {
          expect(state, persisted);
          return rehydrated;
        },
      );

      expect(result, same(rehydrated));
      verify(() => rehydrated.refreshSession()).called(1);
      verify(() => rehydrated.resumePersistedSession()).called(1);
      verifyNever(() => secureStore.clearAll());
    });

    test('clears auth when persisted session validation fails', () async {
      const persisted = PersistedPairingState(
        deviceId: 'dev-auth-only',
        accessToken: 'access',
        accessExpiresAtMs: 1700000000000,
        refreshToken: 'refresh',
        accountId: 'acc',
        accountEmail: 'u@example.com',
      );
      final rehydrated = _MockMobileClient();
      when(() => secureStore.loadState()).thenAnswer((_) async => persisted);
      when(() => secureStore.clearAuth()).thenAnswer((_) async {});
      when(() => rehydrated.refreshSession()).thenThrow(
        const MinosError.authRefreshFailed(message: 'invalid refresh'),
      );

      final result = await MinosCore.resolveClient(
        secure: secureStore,
        buildFresh: () => fail('auth failure should keep device identity'),
        buildFromPersisted: (state) {
          expect(state, persisted);
          return rehydrated;
        },
      );

      expect(result, same(rehydrated));
      verify(() => rehydrated.refreshSession()).called(1);
      verify(() => secureStore.clearAuth()).called(1);
      verifyNever(() => rehydrated.resumePersistedSession());
      verifyNever(() => secureStore.clearAll());
    });

    test(
      'wipes secure storage and returns a fresh client when resume is revoked',
      () async {
        const persisted = PersistedPairingState(
          deviceId: 'dev-stale',
          accessToken: 'access',
          accessExpiresAtMs: 1700000000000,
          refreshToken: 'refresh',
          accountId: 'acc',
          accountEmail: 'u@example.com',
        );
        final rehydrated = _MockMobileClient();
        final freshClient = _MockMobileClient();
        when(() => secureStore.loadState()).thenAnswer((_) async => persisted);
        when(() => rehydrated.refreshSession()).thenAnswer((_) async {});
        when(
          () => rehydrated.persistedPairingState(),
        ).thenAnswer((_) async => persisted);
        when(() => secureStore.saveState(persisted)).thenAnswer((_) async {});
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
        verify(() => rehydrated.refreshSession()).called(1);
        verify(() => rehydrated.resumePersistedSession()).called(1);
        verify(() => secureStore.saveState(persisted)).called(1);
        verify(() => secureStore.clearAll()).called(1);
      },
    );

    test(
      'keeps persisted snapshot when resume fails due to transient connection loss',
      () async {
        const persisted = PersistedPairingState(
          deviceId: 'dev-123',
          accessToken: 'access',
          accessExpiresAtMs: 1700000000000,
          refreshToken: 'refresh',
          accountId: 'acc',
          accountEmail: 'u@example.com',
        );
        final rehydrated = _MockMobileClient();
        when(() => secureStore.loadState()).thenAnswer((_) async => persisted);
        when(() => rehydrated.refreshSession()).thenAnswer((_) async {});
        when(
          () => rehydrated.persistedPairingState(),
        ).thenAnswer((_) async => persisted);
        when(() => secureStore.saveState(persisted)).thenAnswer((_) async {});
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
        verify(() => rehydrated.refreshSession()).called(1);
        verify(() => rehydrated.resumePersistedSession()).called(1);
        verify(() => secureStore.saveState(persisted)).called(1);
        verifyNever(() => secureStore.clearAll());
      },
    );
  });

  group('login / register persistence + cross-account migration', () {
    const newAccountSummary = AuthSummary(
      accountId: 'acc-new',
      email: 'new@example.com',
    );

    PersistedPairingState pairedFor(String? accountId) => PersistedPairingState(
      deviceId: 'dev-old',
      accessToken: accountId == null ? null : 'access',
      accessExpiresAtMs: accountId == null ? null : 1700000000000,
      refreshToken: accountId == null ? null : 'refresh',
      accountId: accountId,
      accountEmail: accountId == null ? null : 'old@example.com',
    );

    test(
      'login on the same account preserves pairing and writes the fresh auth tuple',
      () async {
        const fresh = PersistedPairingState(
          deviceId: 'dev-old',
          accessToken: 'access-new',
          accessExpiresAtMs: 1700000099000,
          refreshToken: 'refresh-new',
          accountId: 'acc-new',
          accountEmail: 'new@example.com',
        );
        when(
          () => secureStore.loadState(),
        ).thenAnswer((_) async => pairedFor('acc-new'));
        when(
          () => client.login(email: 'new@example.com', password: 'pw'),
        ).thenAnswer((_) async => newAccountSummary);
        when(
          () => client.persistedPairingState(),
        ).thenAnswer((_) async => fresh);
        when(() => secureStore.saveState(fresh)).thenAnswer((_) async {});

        final core = MinosCore.forTesting(
          client: client,
          secureStore: secureStore,
        );
        final result = await core.login(
          email: 'new@example.com',
          password: 'pw',
        );

        expect(result, newAccountSummary);
        verify(() => secureStore.saveState(fresh)).called(1);
        verifyNever(() => secureStore.savePeerDisplayName(null));
        verifyNever(() => secureStore.clearAll());
      },
    );

    test(
      'login as a different account clears the cached peer display name',
      () async {
        // ADR-0020: account_mac_pairings is account-scoped on the server,
        // so we no longer call forget_peer locally during cross-account
        // login. We just clear the cached display name so the partners
        // list can re-sync from the server without flashing a stale label.
        const fresh = PersistedPairingState(
          deviceId: 'dev-old',
          accessToken: 'access-new',
          accessExpiresAtMs: 1700000099000,
          refreshToken: 'refresh-new',
          accountId: 'acc-new',
          accountEmail: 'new@example.com',
        );
        when(
          () => secureStore.loadState(),
        ).thenAnswer((_) async => pairedFor('acc-prior'));
        when(
          () => secureStore.savePeerDisplayName(null),
        ).thenAnswer((_) async {});
        when(
          () => client.login(email: 'new@example.com', password: 'pw'),
        ).thenAnswer((_) async => newAccountSummary);
        when(
          () => client.persistedPairingState(),
        ).thenAnswer((_) async => fresh);
        when(() => secureStore.saveState(fresh)).thenAnswer((_) async {});

        final core = MinosCore.forTesting(
          client: client,
          secureStore: secureStore,
        );
        await core.login(email: 'new@example.com', password: 'pw');

        verify(() => secureStore.savePeerDisplayName(null)).called(1);
        verify(() => secureStore.saveState(fresh)).called(1);
        verifyNever(() => secureStore.clearAll());
      },
    );

    test(
      'register on a fresh device (no prior accountId) skips the migration branch',
      () async {
        const fresh = PersistedPairingState(
          deviceId: 'dev-fresh',
          accessToken: 'access-new',
          accessExpiresAtMs: 1700000099000,
          refreshToken: 'refresh-new',
          accountId: 'acc-new',
          accountEmail: 'new@example.com',
        );
        // First-launch case: no persisted state at all.
        when(() => secureStore.loadState()).thenAnswer((_) async => null);
        when(
          () => client.register(email: 'new@example.com', password: 'pw'),
        ).thenAnswer((_) async => newAccountSummary);
        when(
          () => client.persistedPairingState(),
        ).thenAnswer((_) async => fresh);
        when(() => secureStore.saveState(fresh)).thenAnswer((_) async {});

        final core = MinosCore.forTesting(
          client: client,
          secureStore: secureStore,
        );
        await core.register(email: 'new@example.com', password: 'pw');

        verifyNever(() => secureStore.savePeerDisplayName(null));
        verifyNever(() => secureStore.clearAll());
        verify(() => secureStore.saveState(fresh)).called(1);
      },
    );

    test(
      'login after a logged-out cold launch (deviceId-only) keeps the device id',
      () async {
        // Post-logout state: device id intact, auth tuple missing → the
        // device is bound to no account in particular, so any new account's
        // login should reuse the device.
        const fresh = PersistedPairingState(
          deviceId: 'dev-old',
          accessToken: 'access-new',
          accessExpiresAtMs: 1700000099000,
          refreshToken: 'refresh-new',
          accountId: 'acc-new',
          accountEmail: 'new@example.com',
        );
        when(
          () => secureStore.loadState(),
        ).thenAnswer((_) async => pairedFor(null));
        when(
          () => client.login(email: 'new@example.com', password: 'pw'),
        ).thenAnswer((_) async => newAccountSummary);
        when(
          () => client.persistedPairingState(),
        ).thenAnswer((_) async => fresh);
        when(() => secureStore.saveState(fresh)).thenAnswer((_) async {});

        final core = MinosCore.forTesting(
          client: client,
          secureStore: secureStore,
        );
        await core.login(email: 'new@example.com', password: 'pw');

        verifyNever(() => secureStore.savePeerDisplayName(null));
        verifyNever(() => secureStore.clearAll());
        verify(() => secureStore.saveState(fresh)).called(1);
      },
    );
  });

  group('hasPersistedPairing', () {
    test(
      'returns true when secure storage has a fully authenticated snapshot',
      () async {
        when(() => secureStore.loadState()).thenAnswer(
          (_) async => const PersistedPairingState(
            deviceId: 'dev-123',
            accessToken: 'access',
            accessExpiresAtMs: 1700000000000,
            refreshToken: 'refresh',
            accountId: 'acc',
            accountEmail: 'u@example.com',
          ),
        );

        final core = MinosCore.forTesting(
          client: client,
          secureStore: secureStore,
        );

        expect(await core.hasPersistedPairing(), isTrue);
      },
    );

    test('returns false when secure storage is empty', () async {
      when(() => secureStore.loadState()).thenAnswer((_) async => null);

      final core = MinosCore.forTesting(
        client: client,
        secureStore: secureStore,
      );

      expect(await core.hasPersistedPairing(), isFalse);
    });

    test('returns false for a deviceId-only (logged-out) snapshot', () async {
      // Post ADR-0020: hasPersistedPairing now means "logged in" — a
      // deviceId-only snapshot represents the post-logout state and
      // should send the user back to login, not the chat surface.
      when(() => secureStore.loadState()).thenAnswer(
        (_) async => const PersistedPairingState(deviceId: 'dev-paired'),
      );

      final core = MinosCore.forTesting(
        client: client,
        secureStore: secureStore,
      );

      expect(await core.hasPersistedPairing(), isFalse);
    });
  });
}
