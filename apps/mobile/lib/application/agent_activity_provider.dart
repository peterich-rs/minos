import 'dart:async';

import 'package:flutter/foundation.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:minos/application/display_payload_preview.dart';
import 'package:minos/application/group_agent_provider.dart';
import 'package:minos/application/minos_providers.dart';
import 'package:minos/application/thread_events_provider.dart';
import 'package:minos/data/repositories/social_repository.dart';
import 'package:minos/domain/agent_profile.dart';
import 'package:minos/infrastructure/platform_int64.dart';
import 'package:minos/src/rust/api/minos.dart';

enum AgentActivityKind { running, reasoning, tool, success, error }

enum AgentActivityTone { info, success, error }

class AgentActivitySnapshot {
  const AgentActivitySnapshot({
    required this.sessionId,
    required this.label,
    required this.kind,
    required this.tone,
  });

  final String sessionId;
  final String label;
  final AgentActivityKind kind;
  final AgentActivityTone tone;
}

final conversationAgentSessionsProvider = FutureProvider.autoDispose
    .family<List<AgentSessionSummaryDto>, String>((ref, conversationId) {
      return ref
          .watch(socialRepositoryProvider)
          .listAgentSessions(conversationId: conversationId, limit: 5);
    });

final conversationAgentActivityProvider = Provider.autoDispose
    .family<AsyncValue<AgentActivitySnapshot?>, String>((ref, conversationId) {
      final sessionsAsync = ref.watch(
        conversationAgentSessionsProvider(conversationId),
      );
      final sessions = sessionsAsync.asData?.value;
      if (sessions == null) {
        if (sessionsAsync.hasError) {
          return const AsyncValue.data(null);
        }
        return const AsyncValue.loading();
      }

      final session = _latestRunnableSession(sessions);
      if (session == null) {
        return const AsyncValue.data(null);
      }

      final agentOnline = _sessionAgentHostOnline(
        session: session,
        agents: ref.watch(groupAgentsProvider(conversationId)),
        hosts: ref.watch(pairedMacsProvider).asData?.value,
        activeHostId: ref.watch(activeMacProvider).asData?.value,
        connectionState: ref.watch(connectionStateProvider).asData?.value,
      );
      if (!agentOnline) {
        return const AsyncValue.data(null);
      }

      unawaited(
        ref
            .read(socialRepositoryProvider)
            .subscribeAgentSession(sessionId: session.sessionId),
      );

      final eventsAsync = ref.watch(threadEventsProvider(session.sessionId));
      final events = eventsAsync.asData?.value;
      if (events == null) {
        if (eventsAsync.hasError) {
          return const AsyncValue.data(null);
        }
        return const AsyncValue.loading();
      }

      final snapshot = agentActivitySnapshotFromEvents(
        sessionId: session.sessionId,
        events: events,
      );
      if (snapshot != null) {
        return AsyncValue.data(snapshot);
      }

      return const AsyncValue.data(null);
    });

@visibleForTesting
bool debugSessionAgentHostOnline({
  required AgentSessionSummaryDto session,
  required List<AgentProfile> agents,
  required List<HostSummaryDto>? hosts,
  required String? activeHostId,
  required ConnectionState? connectionState,
}) {
  return _sessionAgentHostOnline(
    session: session,
    agents: agents,
    hosts: hosts,
    activeHostId: activeHostId,
    connectionState: connectionState,
  );
}

AgentActivitySnapshot? agentActivitySnapshotFromEvents({
  required String sessionId,
  required List<UiEventMessage> events,
}) {
  final roleByMessage = <String, MessageRole>{};
  final completedMessages = <String>{};
  final statusByMessage = <String, AgentActivitySnapshot>{};
  final toolMessageById = <String, String>{};
  final toolNameById = <String, String>{};
  String? lastAssistantMessageId;
  var threadClosed = false;

  void markAssistantCandidate(String messageId) {
    if (roleByMessage[messageId] == MessageRole.user) return;
    roleByMessage.putIfAbsent(messageId, () => MessageRole.assistant);
    lastAssistantMessageId = messageId;
  }

  void setAssistantStatus(String messageId, AgentActivitySnapshot snapshot) {
    if (roleByMessage[messageId] == MessageRole.user) return;
    markAssistantCandidate(messageId);
    statusByMessage[messageId] = snapshot;
  }

  for (final event in events) {
    switch (event) {
      case UiEventMessage_MessageStarted(:final messageId, :final role):
        roleByMessage[messageId] = role;
        if (role == MessageRole.assistant) {
          lastAssistantMessageId = messageId;
        }
      case UiEventMessage_TextDelta(:final messageId, :final text):
      case UiEventMessage_TextReplace(:final messageId, :final text):
        final preview = _statusPreview(text.renderPreview());
        setAssistantStatus(
          messageId,
          AgentActivitySnapshot(
            sessionId: sessionId,
            label: preview == null ? '生成回复中' : '生成回复中 · $preview',
            kind: AgentActivityKind.running,
            tone: AgentActivityTone.info,
          ),
        );
      case UiEventMessage_ReasoningDelta(:final messageId, :final text):
      case UiEventMessage_ReasoningReplace(:final messageId, :final text):
        final preview = _statusPreview(text.renderPreview());
        setAssistantStatus(
          messageId,
          AgentActivitySnapshot(
            sessionId: sessionId,
            label: preview == null ? '思考中' : '思考中 · $preview',
            kind: AgentActivityKind.reasoning,
            tone: AgentActivityTone.info,
          ),
        );
      case UiEventMessage_ToolCallPlaced(
        :final messageId,
        :final toolCallId,
        :final name,
      ):
        markAssistantCandidate(messageId);
        toolMessageById[toolCallId] = messageId;
        toolNameById[toolCallId] = name;
        setAssistantStatus(
          messageId,
          AgentActivitySnapshot(
            sessionId: sessionId,
            label: '调用工具 · $name',
            kind: AgentActivityKind.tool,
            tone: AgentActivityTone.info,
          ),
        );
      case UiEventMessage_ToolCallCompleted(:final toolCallId, :final isError):
        final messageId = toolMessageById[toolCallId] ?? lastAssistantMessageId;
        if (messageId == null) break;
        markAssistantCandidate(messageId);
        final name = toolNameById[toolCallId] ?? '未知工具';
        setAssistantStatus(
          messageId,
          AgentActivitySnapshot(
            sessionId: sessionId,
            label: isError ? '工具失败 · $name' : '工具完成 · $name',
            kind: isError ? AgentActivityKind.error : AgentActivityKind.success,
            tone: isError ? AgentActivityTone.error : AgentActivityTone.success,
          ),
        );
      case UiEventMessage_MessageCompleted(:final messageId):
        completedMessages.add(messageId);
      case UiEventMessage_Error(:final message):
        return AgentActivitySnapshot(
          sessionId: sessionId,
          label: '执行出错 · ${_statusPreview(message) ?? message}',
          kind: AgentActivityKind.error,
          tone: AgentActivityTone.error,
        );
      case UiEventMessage_ThreadOpened():
      case UiEventMessage_ThreadTitleUpdated():
        break;
      case UiEventMessage_SubagentSpawned(:final agent, :final title):
        final label = title?.trim().isNotEmpty == true
            ? '子 agent · ${agent.name} · ${title!.trim()}'
            : '子 agent · ${agent.name}';
        if (lastAssistantMessageId != null) {
          setAssistantStatus(
            lastAssistantMessageId!,
            AgentActivitySnapshot(
              sessionId: sessionId,
              label: label,
              kind: AgentActivityKind.running,
              tone: AgentActivityTone.info,
            ),
          );
        }
      case UiEventMessage_SubagentStatusUpdated(:final status):
        if (lastAssistantMessageId != null) {
          final done = status == SubagentStatus.completed;
          final failed =
              status == SubagentStatus.failed ||
              status == SubagentStatus.interrupted;
          setAssistantStatus(
            lastAssistantMessageId!,
            AgentActivitySnapshot(
              sessionId: sessionId,
              label: done
                  ? '子 agent 完成'
                  : failed
                  ? '子 agent 失败'
                  : '子 agent 运行中',
              kind: failed
                  ? AgentActivityKind.error
                  : done
                  ? AgentActivityKind.success
                  : AgentActivityKind.running,
              tone: failed
                  ? AgentActivityTone.error
                  : done
                  ? AgentActivityTone.success
                  : AgentActivityTone.info,
            ),
          );
        }
      case UiEventMessage_Raw(:final kind):
        if (lastAssistantMessageId != null) {
          setAssistantStatus(
            lastAssistantMessageId!,
            AgentActivitySnapshot(
              sessionId: sessionId,
              label: '处理事件 · $kind',
              kind: AgentActivityKind.running,
              tone: AgentActivityTone.info,
            ),
          );
        }
        break;
      case UiEventMessage_ThreadClosed():
        threadClosed = true;
    }
  }

  final liveMessageId = lastAssistantMessageId;
  if (threadClosed ||
      liveMessageId == null ||
      completedMessages.contains(liveMessageId)) {
    return null;
  }
  return statusByMessage[liveMessageId];
}

bool _sessionAgentHostOnline({
  required AgentSessionSummaryDto session,
  required List<AgentProfile> agents,
  required List<HostSummaryDto>? hosts,
  required String? activeHostId,
  required ConnectionState? connectionState,
}) {
  final agent = _agentForSession(session, agents);
  if (agent != null) {
    return _agentHostOnline(
      agent: agent,
      hosts: hosts,
      activeHostId: activeHostId,
      connectionState: connectionState,
    );
  }

  final knownHosts = hosts ?? const <HostSummaryDto>[];
  if (knownHosts.isNotEmpty) {
    if (activeHostId != null) {
      for (final host in knownHosts) {
        if (host.hostDeviceId == activeHostId) {
          return host.online;
        }
      }
    }
    return knownHosts.any((host) => host.online);
  }

  return connectionState is ConnectionState_Connected;
}

AgentProfile? _agentForSession(
  AgentSessionSummaryDto session,
  List<AgentProfile> agents,
) {
  final agentId = session.agentId;
  if (agentId != null) {
    for (final agent in agents) {
      if (agent.agentId == agentId || agent.id == agentId) {
        return agent;
      }
    }
  }
  if (agents.length == 1) {
    return agents.first;
  }
  return null;
}

bool _agentHostOnline({
  required AgentProfile agent,
  required List<HostSummaryDto>? hosts,
  required String? activeHostId,
  required ConnectionState? connectionState,
}) {
  final knownHosts = hosts ?? const <HostSummaryDto>[];
  final agentHostId = agent.hostDeviceId;
  if (agentHostId != null) {
    for (final host in knownHosts) {
      if (host.hostDeviceId == agentHostId) {
        return host.online;
      }
    }
    return false;
  }

  if (activeHostId != null) {
    for (final host in knownHosts) {
      if (host.hostDeviceId == activeHostId) {
        return host.online;
      }
    }
  }

  if (knownHosts.length == 1) {
    return knownHosts.first.online;
  }
  if (knownHosts.isNotEmpty) {
    return knownHosts.any((host) => host.online);
  }

  return connectionState is ConnectionState_Connected;
}

AgentSessionSummaryDto? _latestRunnableSession(
  List<AgentSessionSummaryDto> sessions,
) {
  final runnable =
      sessions.where((session) {
        return session.endedAtMs == null && _isRunnableStatus(session.status);
      }).toList()..sort((a, b) {
        return platformInt64ToInt(
          b.lastActivityAtMs,
        ).compareTo(platformInt64ToInt(a.lastActivityAtMs));
      });
  if (runnable.isEmpty) return null;
  return runnable.first;
}

bool _isRunnableStatus(String status) {
  final normalized = status.toLowerCase();
  return !{
    'ended',
    'stopped',
    'failed',
    'completed',
    'cancelled',
    'canceled',
  }.contains(normalized);
}

String? _statusPreview(String raw) {
  final collapsed = raw.replaceAll(RegExp(r'\s+'), ' ').trim();
  if (collapsed.isEmpty) return null;
  const maxChars = 56;
  if (collapsed.length <= maxChars) return collapsed;
  return '${collapsed.substring(0, maxChars - 1)}…';
}
