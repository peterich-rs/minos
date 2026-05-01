import 'dart:async';

import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:minos/application/active_session_provider.dart';
import 'package:minos/application/minos_providers.dart';
import 'package:minos/domain/active_session.dart';
import 'package:minos/domain/minos_core_protocol.dart';
import 'package:minos/src/rust/api/minos.dart';

class _FakeCore implements MinosCoreProtocol {
  final _uiCtl = StreamController<UiEventFrame>.broadcast();

  StartAgentResponse? startResponse;
  MinosError? startError;
  MinosError? sendError;
  int startCount = 0;
  int sendCount = 0;
  int stopCount = 0;
  String? lastSendThreadId;
  String? lastSendText;

  void emit(UiEventFrame frame) => _uiCtl.add(frame);

  @override
  Stream<UiEventFrame> get uiEvents => _uiCtl.stream;

  @override
  Future<StartAgentResponse> startAgent({
    required AgentName agent,
    required String prompt,
  }) async {
    if (startError != null) throw startError!;
    startCount += 1;
    return startResponse ??
        const StartAgentResponse(sessionId: 'thr-1', cwd: '/tmp');
  }

  @override
  Future<void> sendUserMessage({
    required String sessionId,
    required String text,
  }) async {
    if (sendError != null) throw sendError!;
    sendCount += 1;
    lastSendThreadId = sessionId;
    lastSendText = text;
  }

  @override
  Future<void> stopAgent() async {
    stopCount += 1;
  }

  @override
  Future<List<AgentDescriptor>> listClis() async => const [];

  // --- MinosCoreProtocol stubs we don't exercise ---

  @override
  Stream<AuthStateFrame> get authStates => const Stream<AuthStateFrame>.empty();

  @override
  Stream<ConnectionState> get connectionStates =>
      const Stream<ConnectionState>.empty();

  @override
  ConnectionState get currentConnectionState =>
      const ConnectionState.disconnected();

  @override
  Future<AuthSummary> register({
    required String email,
    required String password,
  }) async => throw UnimplementedError();

  @override
  Future<AuthSummary> login({
    required String email,
    required String password,
  }) async => throw UnimplementedError();

  @override
  Future<void> refreshSession() async {}

  @override
  Future<void> logout() async {}

  @override
  Future<void> pairWithQrJson(String qrJson) async {}

  @override
  Future<void> forgetMac(String macDeviceId) async {}

  @override
  Future<List<MacSummaryDto>> listPairedMacs() async => const <MacSummaryDto>[];

  @override
  Future<String?> activeMac() async => null;

  @override
  Future<void> setActiveMac(String macDeviceId) async {}

  @override
  Future<bool> hasPersistedPairing() async => false;

  @override
  Future<String?> peerDisplayName() async => null;

  @override
  Future<void> setPeerDisplayName(String? name) async {}

  @override
  Future<ListThreadsResponse> listThreads(ListThreadsParams params) async =>
      const ListThreadsResponse(threads: []);

  @override
  Future<ReadThreadResponse> readThread(ReadThreadParams params) async =>
      const ReadThreadResponse(uiEvents: []);

  @override
  void notifyForegrounded() {}

  @override
  void notifyBackgrounded() {}

  @override
  Future<void> resumePersistedSession() async {}
}

ProviderContainer _container(_FakeCore core) {
  final c = ProviderContainer(
    overrides: [minosCoreProvider.overrideWithValue(core)],
  );
  addTearDown(c.dispose);
  return c;
}

void main() {
  test('initial state is SessionIdle', () {
    final core = _FakeCore();
    final c = _container(core);
    expect(c.read(activeSessionControllerProvider), const SessionIdle());
  });

  test('start() drives Idle -> Starting -> Streaming', () async {
    final core = _FakeCore()
      ..startResponse = const StartAgentResponse(sessionId: 'thr-A', cwd: '/w');
    final c = _container(core);
    final notifier = c.read(activeSessionControllerProvider.notifier);

    final done = notifier.start(agent: AgentName.codex, prompt: 'hello');
    // After the synchronous setState we should already be in Starting.
    expect(c.read(activeSessionControllerProvider), isA<SessionStarting>());
    await done;
    expect(c.read(activeSessionControllerProvider), isA<SessionStreaming>());
    expect(
      (c.read(activeSessionControllerProvider) as SessionStreaming).threadId,
      'thr-A',
    );
  });

  test('start() error transitions to SessionError with no threadId', () async {
    final core = _FakeCore()
      ..startError = const MinosError.agentStartFailed(reason: 'no daemon');
    final c = _container(core);
    final notifier = c.read(activeSessionControllerProvider.notifier);

    final error = await notifier.start(agent: AgentName.codex, prompt: 'hi');
    final st = c.read(activeSessionControllerProvider);
    expect(st, isA<SessionError>());
    expect((st as SessionError).threadId, isNull);
    expect(st.error, isA<MinosError_AgentStartFailed>());
    expect(error, isA<MinosError_AgentStartFailed>());
  });

  test('startAndSend() sends initial prompt before Streaming', () async {
    final core = _FakeCore()
      ..startResponse = const StartAgentResponse(
        sessionId: 'thr-initial',
        cwd: '/w',
      );
    final c = _container(core);
    final notifier = c.read(activeSessionControllerProvider.notifier);

    final done = notifier.startAndSend(agent: AgentName.codex, prompt: 'hello');
    expect(c.read(activeSessionControllerProvider), isA<SessionStarting>());
    final error = await done;

    expect(error, isNull);
    expect(core.sendCount, 1);
    expect(core.lastSendThreadId, 'thr-initial');
    expect(core.lastSendText, 'hello');
    expect(
      c.read(activeSessionControllerProvider),
      const SessionStreaming(threadId: 'thr-initial', agent: AgentName.codex),
    );
  });

  test(
    'startAndSend() send failure keeps the minted threadId on error',
    () async {
      final core = _FakeCore()
        ..startResponse = const StartAgentResponse(
          sessionId: 'thr-initial-fail',
          cwd: '/w',
        )
        ..sendError = const MinosError.timeout();
      final c = _container(core);
      final notifier = c.read(activeSessionControllerProvider.notifier);

      final error = await notifier.startAndSend(
        agent: AgentName.codex,
        prompt: 'hello',
      );

      expect(error, isA<MinosError_Timeout>());
      expect(
        c.read(activeSessionControllerProvider),
        const SessionError(
          threadId: 'thr-initial-fail',
          error: MinosError.timeout(),
        ),
      );
    },
  );

  test(
    'reset() clears a stale thread-bound error back to SessionIdle',
    () async {
      final core = _FakeCore()
        ..startResponse = const StartAgentResponse(
          sessionId: 'thr-reset',
          cwd: '/w',
        );
      final c = _container(core);
      final notifier = c.read(activeSessionControllerProvider.notifier);

      await notifier.start(agent: AgentName.codex, prompt: 'p');
      core.emit(
        UiEventFrame(
          threadId: 'thr-reset',
          seq: BigInt.zero,
          ui: UiEventMessage.error(code: 'agent_crash', message: 'boom'),
          tsMs: 1,
        ),
      );
      await pumpEventQueue();
      expect(c.read(activeSessionControllerProvider), isA<SessionError>());

      notifier.reset();

      expect(c.read(activeSessionControllerProvider), const SessionIdle());
    },
  );

  test('MessageCompleted on matching thread -> SessionAwaitingInput', () async {
    final core = _FakeCore()
      ..startResponse = const StartAgentResponse(sessionId: 'thr-B', cwd: '/w');
    final c = _container(core);
    final notifier = c.read(activeSessionControllerProvider.notifier);
    await notifier.start(agent: AgentName.codex, prompt: 'p');

    core.emit(
      UiEventFrame(
        threadId: 'thr-B',
        seq: BigInt.zero,
        ui: UiEventMessage.messageCompleted(messageId: 'm1', finishedAtMs: 1),
        tsMs: 1,
      ),
    );
    await pumpEventQueue();
    expect(
      c.read(activeSessionControllerProvider),
      isA<SessionAwaitingInput>(),
    );
  });

  test('UiEvent on a different thread is ignored', () async {
    final core = _FakeCore()
      ..startResponse = const StartAgentResponse(sessionId: 'thr-B', cwd: '/w');
    final c = _container(core);
    final notifier = c.read(activeSessionControllerProvider.notifier);
    await notifier.start(agent: AgentName.codex, prompt: 'p');

    core.emit(
      UiEventFrame(
        threadId: 'thr-OTHER',
        seq: BigInt.zero,
        ui: UiEventMessage.messageCompleted(messageId: 'mx', finishedAtMs: 1),
        tsMs: 1,
      ),
    );
    await pumpEventQueue();
    expect(c.read(activeSessionControllerProvider), isA<SessionStreaming>());
  });

  test('ThreadClosed on matching thread -> SessionStopped', () async {
    final core = _FakeCore()
      ..startResponse = const StartAgentResponse(sessionId: 'thr-C', cwd: '/w');
    final c = _container(core);
    final notifier = c.read(activeSessionControllerProvider.notifier);
    await notifier.start(agent: AgentName.codex, prompt: 'p');

    core.emit(
      UiEventFrame(
        threadId: 'thr-C',
        seq: BigInt.zero,
        ui: UiEventMessage.threadClosed(
          threadId: 'thr-C',
          reason: ThreadEndReason.agentDone(),
          closedAtMs: 1,
        ),
        tsMs: 1,
      ),
    );
    await pumpEventQueue();
    final st = c.read(activeSessionControllerProvider);
    expect(st, isA<SessionStopped>());
    expect((st as SessionStopped).threadId, 'thr-C');
  });

  test(
    'Error frame on matching thread -> SessionError with threadId',
    () async {
      final core = _FakeCore()
        ..startResponse = const StartAgentResponse(
          sessionId: 'thr-D',
          cwd: '/w',
        );
      final c = _container(core);
      final notifier = c.read(activeSessionControllerProvider.notifier);
      await notifier.start(agent: AgentName.codex, prompt: 'p');

      core.emit(
        UiEventFrame(
          threadId: 'thr-D',
          seq: BigInt.zero,
          ui: UiEventMessage.error(code: 'agent_crash', message: 'boom'),
          tsMs: 1,
        ),
      );
      await pumpEventQueue();
      final st = c.read(activeSessionControllerProvider);
      expect(st, isA<SessionError>());
      expect((st as SessionError).threadId, 'thr-D');
    },
  );

  test(
    'send() in AwaitingInput forwards to core and re-enters Streaming',
    () async {
      final core = _FakeCore()
        ..startResponse = const StartAgentResponse(
          sessionId: 'thr-E',
          cwd: '/w',
        );
      final c = _container(core);
      final notifier = c.read(activeSessionControllerProvider.notifier);
      await notifier.start(agent: AgentName.codex, prompt: 'p');
      core.emit(
        UiEventFrame(
          threadId: 'thr-E',
          seq: BigInt.zero,
          ui: UiEventMessage.messageCompleted(messageId: 'm1', finishedAtMs: 1),
          tsMs: 1,
        ),
      );
      await pumpEventQueue();

      final error = await notifier.send('follow-up');
      expect(core.sendCount, 1);
      expect(core.lastSendThreadId, 'thr-E');
      expect(core.lastSendText, 'follow-up');
      expect(c.read(activeSessionControllerProvider), isA<SessionStreaming>());
      expect(error, isNull);
    },
  );

  test(
    'send() failure restores AwaitingInput instead of poisoning session',
    () async {
      final core = _FakeCore()
        ..startResponse = const StartAgentResponse(
          sessionId: 'thr-send-fail',
          cwd: '/w',
        )
        ..sendError = const MinosError.timeout();
      final c = _container(core);
      final notifier = c.read(activeSessionControllerProvider.notifier);

      await notifier.start(agent: AgentName.codex, prompt: 'p');
      core.emit(
        UiEventFrame(
          threadId: 'thr-send-fail',
          seq: BigInt.zero,
          ui: UiEventMessage.messageCompleted(messageId: 'm1', finishedAtMs: 1),
          tsMs: 1,
        ),
      );
      await pumpEventQueue();

      final error = await notifier.send('follow-up');

      expect(error, isA<MinosError_Timeout>());
      expect(
        c.read(activeSessionControllerProvider),
        const SessionAwaitingInput(
          threadId: 'thr-send-fail',
          agent: AgentName.codex,
        ),
      );
    },
  );

  test('send() in Idle is a no-op', () async {
    final core = _FakeCore();
    final c = _container(core);
    await c.read(activeSessionControllerProvider.notifier).send('lost');
    expect(core.sendCount, 0);
    expect(c.read(activeSessionControllerProvider), const SessionIdle());
  });

  test(
    'sendToThread() in Idle resumes known thread without startAgent',
    () async {
      final core = _FakeCore();
      final c = _container(core);

      final error = await c
          .read(activeSessionControllerProvider.notifier)
          .sendToThread(
            threadId: 'thr-existing',
            agent: AgentName.codex,
            text: 'resume',
          );

      expect(error, isNull);
      expect(core.startCount, 0);
      expect(core.sendCount, 1);
      expect(core.lastSendThreadId, 'thr-existing');
      expect(core.lastSendText, 'resume');
      expect(
        c.read(activeSessionControllerProvider),
        const SessionStreaming(
          threadId: 'thr-existing',
          agent: AgentName.codex,
        ),
      );
    },
  );

  test('stop() in Streaming calls core and transitions to Stopped', () async {
    final core = _FakeCore()
      ..startResponse = const StartAgentResponse(sessionId: 'thr-F', cwd: '/w');
    final c = _container(core);
    final notifier = c.read(activeSessionControllerProvider.notifier);
    await notifier.start(agent: AgentName.codex, prompt: 'p');

    await notifier.stop();
    expect(core.stopCount, 1);
    final st = c.read(activeSessionControllerProvider);
    expect(st, isA<SessionStopped>());
    expect((st as SessionStopped).threadId, 'thr-F');
  });

  test('stop() in Idle is a no-op', () async {
    final core = _FakeCore();
    final c = _container(core);
    await c.read(activeSessionControllerProvider.notifier).stop();
    expect(core.stopCount, 0);
    expect(c.read(activeSessionControllerProvider), const SessionIdle());
  });
}
