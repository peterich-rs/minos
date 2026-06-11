import 'package:flutter_test/flutter_test.dart';
import 'package:minos/application/agent_activity_provider.dart';
import 'package:minos/domain/agent_profile.dart';
import 'package:minos/infrastructure/platform_int64.dart';
import 'package:minos/src/rust/api/minos.dart';

void main() {
  group('agentActivitySnapshotFromEvents', () {
    test('returns null without concrete assistant activity events', () {
      expect(
        agentActivitySnapshotFromEvents(
          sessionId: 'session-1',
          events: const <UiEventMessage>[],
        ),
        isNull,
      );

      expect(
        agentActivitySnapshotFromEvents(
          sessionId: 'session-1',
          events: <UiEventMessage>[
            UiEventMessage.messageStarted(
              messageId: 'assistant-1',
              role: MessageRole.assistant,
              startedAtMs: platformInt64FromInt(1000),
            ),
          ],
        ),
        isNull,
      );
    });

    test('ignores user-only turns', () {
      final snapshot = agentActivitySnapshotFromEvents(
        sessionId: 'session-1',
        events: <UiEventMessage>[
          UiEventMessage.messageStarted(
            messageId: 'user-1',
            role: MessageRole.user,
            startedAtMs: platformInt64FromInt(1000),
          ),
          const UiEventMessage.textDelta(messageId: 'user-1', text: 'hello'),
          UiEventMessage.messageCompleted(
            messageId: 'user-1',
            finishedAtMs: platformInt64FromInt(1001),
          ),
        ],
      );

      expect(snapshot, isNull);
    });

    test(
      'surfaces concrete reasoning activity while assistant turn is live',
      () {
        final snapshot = agentActivitySnapshotFromEvents(
          sessionId: 'session-1',
          events: <UiEventMessage>[
            UiEventMessage.messageStarted(
              messageId: 'assistant-1',
              role: MessageRole.assistant,
              startedAtMs: platformInt64FromInt(1000),
            ),
            const UiEventMessage.reasoningDelta(
              messageId: 'assistant-1',
              text: 'checking the repository',
            ),
          ],
        );

        expect(snapshot, isNotNull);
        expect(snapshot!.label, '思考中 · checking the repository');
        expect(snapshot.kind, AgentActivityKind.reasoning);
        expect(snapshot.tone, AgentActivityTone.info);
      },
    );

    test('surfaces concrete tool activity while assistant turn is live', () {
      final snapshot = agentActivitySnapshotFromEvents(
        sessionId: 'session-1',
        events: <UiEventMessage>[
          UiEventMessage.messageStarted(
            messageId: 'assistant-1',
            role: MessageRole.assistant,
            startedAtMs: platformInt64FromInt(1000),
          ),
          const UiEventMessage.toolCallPlaced(
            messageId: 'assistant-1',
            toolCallId: 'tool-1',
            name: 'rg',
            argsJson: '{}',
          ),
        ],
      );

      expect(snapshot, isNotNull);
      expect(snapshot!.label, '调用工具 · rg');
      expect(snapshot.kind, AgentActivityKind.tool);
    });

    test('hides activity after the assistant turn completes', () {
      final snapshot = agentActivitySnapshotFromEvents(
        sessionId: 'session-1',
        events: <UiEventMessage>[
          UiEventMessage.messageStarted(
            messageId: 'assistant-1',
            role: MessageRole.assistant,
            startedAtMs: platformInt64FromInt(1000),
          ),
          const UiEventMessage.textDelta(
            messageId: 'assistant-1',
            text: 'done',
          ),
          UiEventMessage.messageCompleted(
            messageId: 'assistant-1',
            finishedAtMs: platformInt64FromInt(1001),
          ),
        ],
      );

      expect(snapshot, isNull);
    });
  });

  group('debugSessionAgentHostOnline', () {
    test('returns false when the matched agent host is offline', () {
      final session = _session(agentId: 'agent-1');
      final agent = _agent(hostDeviceId: 'host-1');
      final host = _host(hostDeviceId: 'host-1', online: false);

      expect(
        debugSessionAgentHostOnline(
          session: session,
          agents: <AgentProfile>[agent],
          hosts: <HostSummaryDto>[host],
          activeHostId: 'host-1',
          connectionState: const ConnectionState.connected(),
        ),
        isFalse,
      );
    });

    test('returns true when the matched agent host is online', () {
      final session = _session(agentId: 'agent-1');
      final agent = _agent(hostDeviceId: 'host-1');
      final host = _host(hostDeviceId: 'host-1', online: true);

      expect(
        debugSessionAgentHostOnline(
          session: session,
          agents: <AgentProfile>[agent],
          hosts: <HostSummaryDto>[host],
          activeHostId: 'host-1',
          connectionState: const ConnectionState.connected(),
        ),
        isTrue,
      );
    });
  });
}

AgentSessionSummaryDto _session({required String agentId}) {
  return AgentSessionSummaryDto(
    sessionId: 'session-1',
    conversationId: 'conversation-1',
    agentId: agentId,
    status: 'running',
    startedAtMs: platformInt64FromInt(1000),
    lastActivityAtMs: platformInt64FromInt(1000),
    messageCount: 1,
  );
}

AgentProfile _agent({required String hostDeviceId}) {
  return AgentProfile(
    id: 'agent-1',
    agentId: 'agent-1',
    name: 'Agent',
    description: '',
    runtimeAgent: AgentName.codex,
    model: 'gpt-5',
    reasoningEffort: AgentReasoningEffort.medium,
    environmentVariables: const <AgentEnvironmentVariable>[],
    hostDeviceId: hostDeviceId,
    createdAtMs: 1000,
    updatedAtMs: 1000,
  );
}

HostSummaryDto _host({required String hostDeviceId, required bool online}) {
  return HostSummaryDto(
    hostDeviceId: hostDeviceId,
    hostDisplayName: 'Mac',
    pairedAtMs: platformInt64FromInt(1000),
    pairedViaDeviceId: 'phone-1',
    online: online,
  );
}
