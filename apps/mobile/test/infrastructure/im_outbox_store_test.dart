import 'package:flutter_test/flutter_test.dart';
import 'package:minos/infrastructure/im_outbox_store.dart';

void main() {
  group('classifyOutboxFailure', () {
    test('network and connection are transient', () {
      expect(
        classifyOutboxFailure('network error'),
        OutboxFailureClass.transient,
      );
      expect(
        classifyOutboxFailure('Not signed in — message not synced'),
        OutboxFailureClass.transient,
      );
      expect(
        classifyOutboxFailure('connection refused'),
        OutboxFailureClass.transient,
      );
      expect(
        classifyOutboxFailure('SocketException'),
        OutboxFailureClass.transient,
      );
      expect(classifyOutboxFailure('HTTP 503'), OutboxFailureClass.transient);
    });

    test('client validation and 4xx are permanent', () {
      expect(
        classifyOutboxFailure('invalid_payload_json'),
        OutboxFailureClass.permanent,
      );
      expect(classifyOutboxFailure('empty_text'), OutboxFailureClass.permanent);
      expect(classifyOutboxFailure('HTTP 400'), OutboxFailureClass.permanent);
      expect(classifyOutboxFailure('forbidden'), OutboxFailureClass.permanent);
    });

    test('408 and 429 stay transient', () {
      expect(
        classifyOutboxFailure('HTTP 408 timeout'),
        OutboxFailureClass.transient,
      );
      expect(
        classifyOutboxFailure('HTTP 429 too many'),
        OutboxFailureClass.transient,
      );
    });
  });

  group('resolveOutboxFailure', () {
    test('transient never becomes terminal even after many attempts', () {
      for (var attempts = 1; attempts <= 50; attempts++) {
        final outcome = resolveOutboxFailure(
          attempts: attempts,
          error: 'network down',
          nowMs: 1_000_000,
        );
        expect(outcome.status, ImOutboxStatus.pending);
        expect(outcome.failureClass, OutboxFailureClass.transient);
        expect(outcome.nextAttemptAtMs, greaterThan(1_000_000));
      }
    });

    test('permanent becomes terminal after max attempts', () {
      final early = resolveOutboxFailure(
        attempts: 3,
        error: 'HTTP 400 bad',
        nowMs: 1000,
      );
      expect(early.status, ImOutboxStatus.pending);

      final terminal = resolveOutboxFailure(
        attempts: kImOutboxMaxPermanentAttempts,
        error: 'HTTP 400 bad',
        nowMs: 1000,
      );
      expect(terminal.status, ImOutboxStatus.failedTerminal);
    });
  });

  group('ImOutboxMemory', () {
    late ImOutboxMemory store;
    const t0 = 1_700_000_000_000;

    setUp(() {
      store = ImOutboxMemory();
    });

    test('enqueue is idempotent for same client_op_id', () {
      store.enqueueUserMessage(
        clientOpId: 'op-1',
        conversationId: 'c1',
        payloadJson: '{"text":"a"}',
        nowMs: t0,
      );
      store.enqueueUserMessage(
        clientOpId: 'op-1',
        conversationId: 'c1',
        payloadJson: '{"text":"b"}',
        nowMs: t0 + 10,
      );
      expect(store.snapshot.length, 1);
      expect(store.snapshot.single.payloadJson, '{"text":"b"}');
      expect(store.snapshot.single.status, ImOutboxStatus.pending);
    });

    test('enqueue after acked is a no-op', () {
      store.enqueueUserMessage(
        clientOpId: 'op-ack',
        conversationId: 'c1',
        payloadJson: '{"text":"a"}',
        nowMs: t0,
      );
      store.markInflight('op-ack', t0 + 1);
      store.markAcked('op-ack', t0 + 2);
      store.enqueueUserMessage(
        clientOpId: 'op-ack',
        conversationId: 'c1',
        payloadJson: '{"text":"again"}',
        nowMs: t0 + 3,
      );
      expect(store.snapshot.single.status, ImOutboxStatus.acked);
      expect(store.listDue(t0 + 100).length, 0);
    });

    test('stale inflight reclaim makes row due again', () {
      store.enqueueUserMessage(
        clientOpId: 'op-stale',
        conversationId: 'c1',
        payloadJson: '{"text":"x"}',
        nowMs: t0,
      );
      store.markInflight('op-stale', t0);
      expect(store.listDue(t0 + 1_000).length, 0);

      final reclaimed = store.reclaimStaleInflight(
        t0 + kImOutboxStaleInflightMs + 1,
      );
      expect(reclaimed, 1);
      final due = store.listDue(t0 + kImOutboxStaleInflightMs + 1);
      expect(due.length, 1);
      expect(due.single.clientOpId, 'op-stale');
      expect(due.single.status, ImOutboxStatus.pending);
    });

    test('network failures never terminal; permanent can terminal', () {
      store.enqueueUserMessage(
        clientOpId: 'op-net',
        conversationId: 'c1',
        payloadJson: '{"text":"x"}',
        nowMs: t0,
      );
      for (var i = 0; i < 20; i++) {
        store.markInflight('op-net', t0 + i);
        final status = store.markFailed('op-net', 'network', t0 + i);
        expect(status, ImOutboxStatus.pending);
      }

      store.enqueueUserMessage(
        clientOpId: 'op-perm',
        conversationId: 'c1',
        payloadJson: '{"text":"y"}',
        nowMs: t0,
      );
      ImOutboxStatus last = ImOutboxStatus.pending;
      for (var i = 0; i < kImOutboxMaxPermanentAttempts; i++) {
        store.markInflight('op-perm', t0 + i);
        last = store.markFailed('op-perm', 'HTTP 400', t0 + i);
      }
      expect(last, ImOutboxStatus.failedTerminal);
    });

    test('startup reclaim covers sending rows still in outbox', () {
      store.enqueueUserMessage(
        clientOpId: 'covered',
        conversationId: 'c1',
        payloadJson: '{"text":"x"}',
        nowMs: t0,
      );
      store.markInflight('covered', t0);
      final covered = store.reclaimAllInflightOnStartup(t0 + 100);
      expect(covered.contains('covered'), isTrue);
      expect(store.snapshot.single.status, ImOutboxStatus.pending);

      final stranded = store.strandedSendingLocalIds(
        sendingLocalIds: <String>['covered', 'orphan'],
        coveredOutboxIds: covered,
      );
      expect(stranded, <String>['orphan']);
    });
  });
}
