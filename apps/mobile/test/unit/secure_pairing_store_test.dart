import 'package:flutter_secure_storage/flutter_secure_storage.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:mocktail/mocktail.dart';

import 'package:minos/infrastructure/secure_pairing_store.dart';
import 'package:minos/src/rust/api/minos.dart';

class _MockFlutterSecureStorage extends Mock implements FlutterSecureStorage {}

void main() {
  late _MockFlutterSecureStorage storage;
  late Map<String, String> values;

  setUp(() {
    storage = _MockFlutterSecureStorage();
    values = <String, String>{};

    when(() => storage.read(key: any(named: 'key'))).thenAnswer((
      invocation,
    ) async {
      final key = invocation.namedArguments[#key]! as String;
      return values[key];
    });
    when(
      () => storage.write(
        key: any(named: 'key'),
        value: any(named: 'value'),
      ),
    ).thenAnswer((invocation) async {
      final key = invocation.namedArguments[#key]! as String;
      final value = invocation.namedArguments[#value] as String?;
      if (value == null) {
        values.remove(key);
      } else {
        values[key] = value;
      }
    });
    when(() => storage.delete(key: any(named: 'key'))).thenAnswer((
      invocation,
    ) async {
      final key = invocation.namedArguments[#key]! as String;
      values.remove(key);
    });
  });

  test('saveState/loadState round-trips device credentials', () async {
    final store = SecurePairingStore(storage: storage);
    const state = PersistedPairingState(
      deviceId: 'dev-123',
      deviceSecret: 'sec-456',
    );

    await store.saveState(state);

    expect(values, <String, String>{
      'minos.device_id': 'dev-123',
      'minos.device_secret': 'sec-456',
    });
    expect(await store.loadState(), state);
  });

  test('clearAll removes every persisted credential key', () async {
    final store = SecurePairingStore(storage: storage);
    await store.saveState(
      const PersistedPairingState(deviceId: 'dev-123', deviceSecret: 'sec-456'),
    );

    await store.clearAll();

    expect(values, isEmpty);
    expect(await store.loadState(), isNull);
  });

  test('loadState wipes incomplete core resume snapshots', () async {
    final store = SecurePairingStore(storage: storage);
    values.addAll(<String, String>{'minos.device_id': 'dev-123'});

    expect(await store.loadState(), isNull);
    expect(values, isEmpty);
  });

  // ---- Phase 4 auth fields ----

  test('saveState/loadState round-trips full auth tuple', () async {
    final store = SecurePairingStore(storage: storage);
    final state = PersistedPairingState(
      deviceId: 'dev-1',
      deviceSecret: 'sec-1',
      accessToken: 'access-token-xyz',
      accessExpiresAtMs: 1700000000000,
      refreshToken: 'refresh-token-abc',
      accountId: 'acc-uuid',
      accountEmail: 'user@example.com',
    );

    await store.saveState(state);

    expect(values['minos.access_token'], 'access-token-xyz');
    expect(values['minos.access_expires_at_ms'], '1700000000000');
    expect(values['minos.refresh_token'], 'refresh-token-abc');
    expect(values['minos.account_id'], 'acc-uuid');
    expect(values['minos.account_email'], 'user@example.com');

    expect(await store.loadState(), state);
  });

  test('loadState preserves authenticated pre-pair snapshots', () async {
    final store = SecurePairingStore(storage: storage);
    final state = PersistedPairingState(
      deviceId: 'dev-1',
      accessToken: 'access-token-xyz',
      accessExpiresAtMs: 1700000000000,
      refreshToken: 'refresh-token-abc',
      accountId: 'acc-uuid',
      accountEmail: 'user@example.com',
    );

    await store.saveState(state);

    expect(values['minos.device_id'], 'dev-1');
    expect(values.containsKey('minos.device_secret'), isFalse);
    expect(await store.loadState(), state);
  });

  test('saveState skips auth keys when no auth tuple is present', () async {
    final store = SecurePairingStore(storage: storage);
    const state = PersistedPairingState(
      deviceId: 'dev-1',
      deviceSecret: 'sec-1',
    );

    await store.saveState(state);

    expect(values.containsKey('minos.access_token'), isFalse);
    expect(values.containsKey('minos.refresh_token'), isFalse);
    expect(values.containsKey('minos.account_id'), isFalse);
  });

  test('clearAuth wipes only the auth tuple, leaving pairing intact', () async {
    final store = SecurePairingStore(storage: storage);
    final state = PersistedPairingState(
      deviceId: 'dev-1',
      deviceSecret: 'sec-1',
      accessToken: 'access',
      accessExpiresAtMs: 1700000000000,
      refreshToken: 'refresh',
      accountId: 'acc',
      accountEmail: 'u@example.com',
    );
    await store.saveState(state);

    await store.clearAuth();

    expect(values['minos.device_id'], 'dev-1');
    expect(values['minos.device_secret'], 'sec-1');
    expect(values.containsKey('minos.access_token'), isFalse);
    expect(values.containsKey('minos.access_expires_at_ms'), isFalse);
    expect(values.containsKey('minos.refresh_token'), isFalse);
    expect(values.containsKey('minos.account_id'), isFalse);
    expect(values.containsKey('minos.account_email'), isFalse);
  });

  test('loadState wipes a half-set auth tuple', () async {
    final store = SecurePairingStore(storage: storage);
    values.addAll(<String, String>{
      'minos.device_id': 'dev-1',
      'minos.device_secret': 'sec-1',
      // Missing access_expires_at_ms / refresh_token / account_id /
      // account_email — half-set tuple must be treated as corruption.
      'minos.access_token': 'access',
    });

    expect(await store.loadState(), isNull);
    expect(values, isEmpty);
  });

  test('clearAll wipes auth keys too', () async {
    final store = SecurePairingStore(storage: storage);
    values.addAll(<String, String>{
      'minos.access_token': 'access',
      'minos.access_expires_at_ms': '1700000000000',
      'minos.refresh_token': 'refresh',
      'minos.account_id': 'acc',
      'minos.account_email': 'u@example.com',
    });
    await store.clearAll();
    expect(values, isEmpty);
  });
}
