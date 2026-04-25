import 'package:flutter_secure_storage/flutter_secure_storage.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:mocktail/mocktail.dart';

import 'package:minos/infrastructure/cf_access_config.dart';
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

  test(
    'saveState/loadState injects build-time cf access credentials',
    () async {
      final store = SecurePairingStore(
        storage: storage,
        cfAccessConfig: CfAccessConfig(
          clientId: 'build-cf-id',
          clientSecret: 'build-cf-secret',
        ),
      );
      const state = PersistedPairingState(
        backendUrl: 'ws://127.0.0.1/devices',
        deviceId: 'dev-123',
        deviceSecret: 'sec-456',
        cfAccessClientId: 'runtime-cf-id',
        cfAccessClientSecret: 'runtime-cf-secret',
      );

      await store.saveState(state);

      expect(values, <String, String>{
        'minos.backend_url': 'ws://127.0.0.1/devices',
        'minos.device_id': 'dev-123',
        'minos.device_secret': 'sec-456',
      });
      expect(
        await store.loadState(),
        const PersistedPairingState(
          backendUrl: 'ws://127.0.0.1/devices',
          deviceId: 'dev-123',
          deviceSecret: 'sec-456',
          cfAccessClientId: 'build-cf-id',
          cfAccessClientSecret: 'build-cf-secret',
        ),
      );
    },
  );

  test('clearAll removes every persisted credential key', () async {
    final store = SecurePairingStore(storage: storage);
    await store.saveState(
      const PersistedPairingState(
        backendUrl: 'ws://127.0.0.1/devices',
        deviceId: 'dev-123',
        deviceSecret: 'sec-456',
      ),
    );
    values['minos.cf_access_client_id'] = 'legacy-cf-id';
    values['minos.cf_access_client_secret'] = 'legacy-cf-secret';

    await store.clearAll();

    expect(values, isEmpty);
    expect(await store.loadState(), isNull);
  });

  test('loadState wipes incomplete core resume snapshots', () async {
    final store = SecurePairingStore(storage: storage);
    values.addAll(<String, String>{
      'minos.backend_url': 'ws://127.0.0.1/devices',
      'minos.device_id': 'dev-123',
      'minos.cf_access_client_id': 'cf-id',
      'minos.cf_access_client_secret': 'cf-secret',
    });

    expect(await store.loadState(), isNull);
    expect(values, isEmpty);
  });

  test(
    'loadState removes legacy stored Cloudflare Access credentials',
    () async {
      final store = SecurePairingStore(storage: storage);
      values.addAll(<String, String>{
        'minos.backend_url': 'ws://127.0.0.1/devices',
        'minos.device_id': 'dev-123',
        'minos.device_secret': 'sec-456',
        'minos.cf_access_client_id': 'legacy-cf-id',
        'minos.cf_access_client_secret': 'legacy-cf-secret',
      });

      expect(
        await store.loadState(),
        const PersistedPairingState(
          backendUrl: 'ws://127.0.0.1/devices',
          deviceId: 'dev-123',
          deviceSecret: 'sec-456',
        ),
      );
      expect(values, <String, String>{
        'minos.backend_url': 'ws://127.0.0.1/devices',
        'minos.device_id': 'dev-123',
        'minos.device_secret': 'sec-456',
      });
    },
  );
}
