import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:mocktail/mocktail.dart';

import 'package:minos/application/minos_providers.dart';
import 'package:minos/domain/minos_core_protocol.dart';
import 'package:minos/src/rust/api/minos.dart';

class _FakeMinosCore extends Mock implements MinosCoreProtocol {}

void main() {
  setUpAll(() {
    // mocktail requires a fallback for any reference-type arguments used in
    // matchers like `any()`. We only match strings, so a string fallback is
    // enough, but register PairResponse too for symmetry.
    registerFallbackValue('');
  });

  ProviderContainer buildContainer(MinosCoreProtocol core) {
    final container = ProviderContainer(
      overrides: [minosCoreProvider.overrideWithValue(core)],
    );
    addTearDown(container.dispose);
    return container;
  }

  test('submit(validJson) transitions idle -> loading -> data', () async {
    final fake = _FakeMinosCore();
    const response = PairResponse(ok: true, macName: 'MacTest');
    when(() => fake.pairWithJson(any())).thenAnswer((_) async => response);

    final container = buildContainer(fake);
    final snapshots = <AsyncValue<PairResponse?>>[];
    container.listen<AsyncValue<PairResponse?>>(
      pairingControllerProvider,
      (_, next) => snapshots.add(next),
      fireImmediately: true,
    );

    expect(snapshots.first, const AsyncData<PairResponse?>(null));

    await container.read(pairingControllerProvider.notifier).submit('{...}');

    // Expect: initial idle, then loading, then data.
    expect(snapshots.length, greaterThanOrEqualTo(3));
    expect(snapshots[1].isLoading, isTrue);
    expect(snapshots.last, const AsyncData<PairResponse?>(response));
    verify(() => fake.pairWithJson('{...}')).called(1);
  });

  test('submit(invalidJson) surfaces MinosError as AsyncError', () async {
    final fake = _FakeMinosCore();
    const err = MinosError.storeCorrupt(
      path: 'qr_payload',
      message: 'bad utf8',
    );
    when(() => fake.pairWithJson(any())).thenThrow(err);

    final container = buildContainer(fake);
    final snapshots = <AsyncValue<PairResponse?>>[];
    container.listen<AsyncValue<PairResponse?>>(
      pairingControllerProvider,
      (_, next) => snapshots.add(next),
      fireImmediately: true,
    );

    await container.read(pairingControllerProvider.notifier).submit('garbage');

    final last = snapshots.last;
    expect(last, isA<AsyncError<PairResponse?>>());
    expect(last.error, isA<MinosError_StoreCorrupt>());
    expect(last.error, equals(err));
  });

  test(
    'second submit after an error clears the error and lands on data',
    () async {
      final fake = _FakeMinosCore();
      const goodResponse = PairResponse(ok: true, macName: 'MacTest');
      const err = MinosError.storeCorrupt(
        path: 'qr_payload',
        message: 'bad utf8',
      );

      var callIndex = 0;
      when(() => fake.pairWithJson(any())).thenAnswer((invocation) async {
        callIndex += 1;
        if (callIndex == 1) {
          throw err;
        }
        return goodResponse;
      });

      final container = buildContainer(fake);
      final snapshots = <AsyncValue<PairResponse?>>[];
      container.listen<AsyncValue<PairResponse?>>(
        pairingControllerProvider,
        (_, next) => snapshots.add(next),
        fireImmediately: true,
      );

      await container.read(pairingControllerProvider.notifier).submit('bad');
      expect(snapshots.last, isA<AsyncError<PairResponse?>>());

      await container.read(pairingControllerProvider.notifier).submit('good');
      expect(snapshots.last, const AsyncData<PairResponse?>(goodResponse));
    },
  );
}
