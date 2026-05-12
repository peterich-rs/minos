import 'dart:async';

import 'package:flutter_riverpod/flutter_riverpod.dart';

import 'package:minos/application/group_agent_provider.dart';
import 'package:minos/application/minos_providers.dart';
import 'package:minos/application/social_providers.dart';
import 'package:minos/domain/agent_profile.dart';
import 'package:minos/src/rust/api/minos.dart';

/// Dispatches agent tasks when an agent is @mentioned in a group chat.
///
/// When a message containing @agentId is sent in a group conversation,
/// this dispatcher:
/// 1. Detects which agents are mentioned
/// 2. Starts an agent session for each mentioned agent
/// 3. Sends the message text as the agent's prompt
/// 4. Streams the agent's response back into the group chat
///
/// NOTE: Currently uses `sendChatMessage` to relay agent output. Once the
/// FRB bridge exposes `sendAgentMessage`, this should be updated to use
/// the backend's `/conversations/:id/agents/message` endpoint so messages
/// are properly attributed to the agent (sender_type = 'agent').
class GroupAgentDispatcher {
  GroupAgentDispatcher(this._ref);

  final Ref _ref;

  /// Process a sent message and dispatch to any mentioned agents.
  /// Called after a message is successfully sent in a group conversation.
  Future<void> dispatchIfMentioned({
    required String conversationId,
    required String messageText,
  }) async {
    final groupAgents = _ref.read(groupAgentsProvider(conversationId));
    if (groupAgents.isEmpty) return;

    // Extract mentioned agent IDs from the message text
    final mentionedAgents = _extractMentionedAgents(messageText, groupAgents);
    if (mentionedAgents.isEmpty) return;

    // Strip the @mentions from the text to get the actual prompt
    final prompt = _stripAgentMentions(messageText, mentionedAgents);
    if (prompt.trim().isEmpty) return;

    // Dispatch to each mentioned agent
    for (final agent in mentionedAgents) {
      unawaited(
        _dispatchToAgent(
          conversationId: conversationId,
          agent: agent,
          prompt: prompt.trim(),
        ),
      );
    }
  }

  /// Start an agent session and relay its output to the group chat.
  Future<void> _dispatchToAgent({
    required String conversationId,
    required AgentProfile agent,
    required String prompt,
  }) async {
    try {
      final core = _ref.read(minosCoreProvider);

      // Start the agent session
      final response = await core.startAgent(
        agent: agent.runtimeAgent,
        prompt: prompt,
        workspace: '.',
      );

      // Send a status message to the group indicating the agent is working
      await core.sendChatMessage(
        conversationId: conversationId,
        text: '🤖 [${agent.name}] 正在处理任务...',
      );

      // Subscribe to UI events for this agent session and relay results
      _relayAgentOutput(
        conversationId: conversationId,
        agent: agent,
        sessionId: response.sessionId,
      );
    } catch (error) {
      // Send error notification to the group
      try {
        await _ref
            .read(minosCoreProvider)
            .sendChatMessage(
              conversationId: conversationId,
              text: '🤖 [${agent.name}] 启动失败: $error',
            );
      } catch (_) {
        // Best-effort error reporting
      }
    }
  }

  /// Listen to the agent's UI event stream and relay text output to the group.
  void _relayAgentOutput({
    required String conversationId,
    required AgentProfile agent,
    required String sessionId,
  }) {
    final core = _ref.read(minosCoreProvider);
    final buffer = StringBuffer();
    StreamSubscription<UiEventFrame>? subscription;

    subscription = core.uiEvents.listen((frame) {
      if (frame.threadId != sessionId) return;

      final ui = frame.ui;
      switch (ui) {
        case UiEventMessage_TextDelta(:final text):
          buffer.write(text);
        case UiEventMessage_ToolCallPlaced(:final name):
          buffer.write('\n[调用工具: $name]\n');
        case UiEventMessage_Error(:final message):
          buffer.write('\n⚠️ 错误: $message\n');
        case UiEventMessage_ThreadClosed():
          subscription?.cancel();
          final output = buffer.toString().trim();
          if (output.isNotEmpty) {
            unawaited(
              core.sendChatMessage(
                conversationId: conversationId,
                text: '🤖 [${agent.name}] 完成:\n\n$output',
              ),
            );
          } else {
            unawaited(
              core.sendChatMessage(
                conversationId: conversationId,
                text: '🤖 [${agent.name}] 任务已完成',
              ),
            );
          }
          unawaited(
            _ref
                .read(socialConversationProvider(conversationId).notifier)
                .refresh(),
          );
        default:
          break;
      }
    });

    // Safety timeout: cancel after 5 minutes
    unawaited(
      Future<void>.delayed(const Duration(minutes: 5)).then((_) {
        if (subscription != null) {
          subscription.cancel();
          final output = buffer.toString().trim();
          if (output.isNotEmpty) {
            unawaited(
              core.sendChatMessage(
                conversationId: conversationId,
                text: '🤖 [${agent.name}] 超时，部分输出:\n\n$output',
              ),
            );
          }
        }
      }),
    );
  }

  /// Extract agents that are mentioned in the message text.
  List<AgentProfile> _extractMentionedAgents(
    String text,
    List<AgentProfile> groupAgents,
  ) {
    final mentioned = <AgentProfile>[];
    for (final agent in groupAgents) {
      if (text.contains('@${agent.agentId}')) {
        mentioned.add(agent);
      }
    }
    return mentioned;
  }

  /// Remove @agentId mentions from the text to get the clean prompt.
  String _stripAgentMentions(String text, List<AgentProfile> agents) {
    var result = text;
    for (final agent in agents) {
      result = result.replaceAll('@${agent.agentId}', '').trim();
    }
    return result;
  }
}

/// Provider for the group agent dispatcher.
final groupAgentDispatcherProvider = Provider<GroupAgentDispatcher>((ref) {
  return GroupAgentDispatcher(ref);
});
