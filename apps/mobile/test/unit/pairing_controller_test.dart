import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:mocktail/mocktail.dart';

import 'package:minos/application/minos_providers.dart';
import 'package:minos/domain/minos_core_protocol.dart';
import 'package:minos/src/rust/api/minos.dart';

class _FakeMinosCore extends Mock implements MinosCoreProtocol {}

void main() {
  setUpAll(() {
    registerFallbackValue('');
  });

  ProviderContainer buildContainer(MinosCoreProtocol core) {
    final container = ProviderContainer(
      overrides: [minosCoreProvider.overrideWithValue(core)],
    );
    addTearDown(container.dispose);
    return container;
  }

  test('submit(validJson) transitions idle -> loading -> data(true)', () async {
    final fake = _FakeMinosCore();
    when(() => fake.pairWithQrJson(any())).thenAnswer((_) async {});

    final container = buildContainer(fake);
    final snapshots = <AsyncValue<bool>>[];
    container.listen<AsyncValue<bool>>(
      pairingControllerProvider,
      (_, next) => snapshots.add(next),
      fireImmediately: true,
    );

    expect(snapshots.first, const AsyncData<bool>(false));

    await container.read(pairingControllerProvider.notifier).submit('{"v":2}');

    expect(snapshots.length, greaterThanOrEqualTo(3));
    expect(snapshots[1].isLoading, isTrue);
    expect(snapshots.last, const AsyncData<bool>(true));
    verify(() => fake.pairWithQrJson('{"v":2}')).called(1);
  });

  test('submit(invalidJson) surfaces MinosError as AsyncError', () async {
    final fake = _FakeMinosCore();
    const err = MinosError.storeCorrupt(
      path: 'qr_payload',
      message: 'bad utf8',
    );
    when(() => fake.pairWithQrJson(any())).thenThrow(err);

    final container = buildContainer(fake);
    final snapshots = <AsyncValue<bool>>[];
    container.listen<AsyncValue<bool>>(
      pairingControllerProvider,
      (_, next) => snapshots.add(next),
      fireImmediately: true,
    );

    await container.read(pairingControllerProvider.notifier).submit('garbage');

    final last = snapshots.last;
    expect(last, isA<AsyncError<bool>>());
    expect(last.error, isA<MinosError_StoreCorrupt>());
    expect(last.error, equals(err));
  });

  test(
    'second submit after an error clears the error and lands on data(true)',
    () async {
      final fake = _FakeMinosCore();
      const err = MinosError.storeCorrupt(
        path: 'qr_payload',
        message: 'bad utf8',
      );

      var callIndex = 0;
      when(() => fake.pairWithQrJson(any())).thenAnswer((invocation) async {
        callIndex += 1;
        if (callIndex == 1) {
          throw err;
        }
      });

      final container = buildContainer(fake);
      final snapshots = <AsyncValue<bool>>[];
      container.listen<AsyncValue<bool>>(
        pairingControllerProvider,
        (_, next) => snapshots.add(next),
        fireImmediately: true,
      );

      await container.read(pairingControllerProvider.notifier).submit('bad');
      expect(snapshots.last, isA<AsyncError<bool>>());

      await container.read(pairingControllerProvider.notifier).submit('good');
      expect(snapshots.last, const AsyncData<bool>(true));
    },
  );

  test('submit() rejects v1 payload via underlying MinosError', () async {
    // The Rust side classifies a v1 payload as
    // PairingQrVersionUnsupported; the controller should surface whatever
    // the core throws. We emulate this with the concrete Dart-side variant.
    final fake = _FakeMinosCore();
    const err = MinosError.pairingQrVersionUnsupported(version: 1);
    when(() => fake.pairWithQrJson(any())).thenThrow(err);

    final container = buildContainer(fake);
    final snapshots = <AsyncValue<bool>>[];
    container.listen<AsyncValue<bool>>(
      pairingControllerProvider,
      (_, next) => snapshots.add(next),
      fireImmediately: true,
    );

    await container
        .read(pairingControllerProvider.notifier)
        .submit('{"v":1,"old":"format"}');

    final last = snapshots.last;
    expect(last, isA<AsyncError<bool>>());
    expect(last.error, isA<MinosError_PairingQrVersionUnsupported>());
  });
}
