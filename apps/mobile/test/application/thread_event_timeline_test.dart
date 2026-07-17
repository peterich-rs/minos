import 'package:flutter_test/flutter_test.dart';
import 'package:minos/application/thread_event_timeline.dart';
import 'package:minos/infrastructure/platform_int64.dart';
import 'package:minos/src/rust/api/minos.dart';

void main() {
  group('buildThreadEventTimeline', () {
    test('places late reasoning after tools, not under the user message', () {
      final rows = buildThreadEventTimeline(
        <UiEventMessage>[
          UiEventMessage.messageStarted(
            messageId: 'user-1',
            role: MessageRole.user,
            startedAtMs: platformInt64FromInt(1),
          ),
          const UiEventMessage.textDelta(
            messageId: 'user-1',
            text: DisplayPayload.inline(text: 'do the task'),
          ),
          UiEventMessage.messageStarted(
            messageId: 'assistant-1',
            role: MessageRole.assistant,
            startedAtMs: platformInt64FromInt(2),
          ),
          const UiEventMessage.toolCallPlaced(
            messageId: 'assistant-1',
            toolCallId: 'tc-1',
            name: 'read_file',
            argsJson: DisplayPayload.inline(text: '{"path":"a.rs"}'),
          ),
          const UiEventMessage.toolCallCompleted(
            toolCallId: 'tc-1',
            output: DisplayPayload.inline(text: 'ok'),
            isError: false,
          ),
          const UiEventMessage.reasoningDelta(
            messageId: 'assistant-1',
            text: DisplayPayload.inline(text: 'after tools'),
          ),
        ],
      );

      expect(rows, hasLength(3));
      expect(rows[0], isA<TimelineUserMessage>());
      expect(rows[1], isA<TimelineToolCall>());
      expect(rows[2], isA<TimelineReasoning>());
      expect((rows[2] as TimelineReasoning).text, 'after tools');
      expect(rows[2].eventIndex, greaterThan(rows[1].eventIndex));
    });

    test('opens a new reasoning segment after intervening tools', () {
      final rows = buildThreadEventTimeline(
        <UiEventMessage>[
          UiEventMessage.messageStarted(
            messageId: 'assistant-1',
            role: MessageRole.assistant,
            startedAtMs: platformInt64FromInt(1),
          ),
          const UiEventMessage.reasoningDelta(
            messageId: 'assistant-1',
            text: DisplayPayload.inline(text: 'first'),
          ),
          const UiEventMessage.toolCallPlaced(
            messageId: 'assistant-1',
            toolCallId: 'tc-1',
            name: 'grep',
            argsJson: DisplayPayload.inline(text: '{}'),
          ),
          const UiEventMessage.reasoningDelta(
            messageId: 'assistant-1',
            text: DisplayPayload.inline(text: 'second'),
          ),
          const UiEventMessage.textDelta(
            messageId: 'assistant-1',
            text: DisplayPayload.inline(text: 'done'),
          ),
        ],
      );

      expect(rows.map((row) => row.runtimeType).toList(), <Type>[
        TimelineReasoning,
        TimelineToolCall,
        TimelineReasoning,
        TimelineAssistantText,
      ]);
      expect((rows[0] as TimelineReasoning).text, 'first');
      expect((rows[2] as TimelineReasoning).text, 'second');
      expect((rows[3] as TimelineAssistantText).text, 'done');
    });

    test('streams contiguous reasoning deltas onto the same item', () {
      final rows = buildThreadEventTimeline(
        <UiEventMessage>[
          UiEventMessage.messageStarted(
            messageId: 'assistant-1',
            role: MessageRole.assistant,
            startedAtMs: platformInt64FromInt(1),
          ),
          const UiEventMessage.reasoningDelta(
            messageId: 'assistant-1',
            text: DisplayPayload.inline(text: 'think'),
          ),
          const UiEventMessage.reasoningDelta(
            messageId: 'assistant-1',
            text: DisplayPayload.inline(text: ' more'),
          ),
        ],
      );

      expect(rows, hasLength(1));
      expect((rows.single as TimelineReasoning).text, 'think more');
    });

    test('shows live placeholder only before first content arrives', () {
      final waiting = buildThreadEventTimeline(
        <UiEventMessage>[
          UiEventMessage.messageStarted(
            messageId: 'assistant-1',
            role: MessageRole.assistant,
            startedAtMs: platformInt64FromInt(1),
          ),
        ],
        showLiveAssistantState: true,
      );
      expect(waiting, hasLength(1));
      expect(waiting.single, isA<TimelineAssistantPlaceholder>());

      final withTool = buildThreadEventTimeline(
        <UiEventMessage>[
          UiEventMessage.messageStarted(
            messageId: 'assistant-1',
            role: MessageRole.assistant,
            startedAtMs: platformInt64FromInt(1),
          ),
          const UiEventMessage.toolCallPlaced(
            messageId: 'assistant-1',
            toolCallId: 'tc-1',
            name: 'read_file',
            argsJson: DisplayPayload.inline(text: '{}'),
          ),
        ],
        showLiveAssistantState: true,
      );
      expect(withTool, hasLength(1));
      expect(withTool.single, isA<TimelineToolCall>());
    });

    test(
      'splits intermediate and final assistant text across tools (ACP/Grok)',
      () {
        final rows = buildThreadEventTimeline(
          <UiEventMessage>[
            UiEventMessage.messageStarted(
              messageId: 'user-1',
              role: MessageRole.user,
              startedAtMs: platformInt64FromInt(1),
            ),
            const UiEventMessage.textDelta(
              messageId: 'user-1',
              text: DisplayPayload.inline(text: 'fix the bug'),
            ),
            UiEventMessage.messageStarted(
              messageId: 'assistant-1',
              role: MessageRole.assistant,
              startedAtMs: platformInt64FromInt(2),
            ),
            const UiEventMessage.textDelta(
              messageId: 'assistant-1',
              text: DisplayPayload.inline(text: 'Looking into it.'),
            ),
            const UiEventMessage.toolCallPlaced(
              messageId: 'assistant-1',
              toolCallId: 'tc-1',
              name: 'read_file',
              argsJson: DisplayPayload.inline(text: '{}'),
            ),
            const UiEventMessage.toolCallCompleted(
              toolCallId: 'tc-1',
              output: DisplayPayload.inline(text: 'ok'),
              isError: false,
            ),
            const UiEventMessage.reasoningDelta(
              messageId: 'assistant-1',
              text: DisplayPayload.inline(text: 'found the issue'),
            ),
            const UiEventMessage.textDelta(
              messageId: 'assistant-1',
              text: DisplayPayload.inline(text: 'Here is the fix.'),
            ),
            UiEventMessage.messageCompleted(
              messageId: 'assistant-1',
              finishedAtMs: platformInt64FromInt(3),
            ),
          ],
          showLiveAssistantState: true,
        );

        expect(rows.map((row) => row.runtimeType).toList(), <Type>[
          TimelineUserMessage,
          TimelineAssistantText,
          TimelineToolCall,
          TimelineReasoning,
          TimelineAssistantText,
        ]);
        expect((rows[1] as TimelineAssistantText).text, 'Looking into it.');
        expect((rows[1] as TimelineAssistantText).showCursor, isFalse);
        expect((rows[3] as TimelineReasoning).text, 'found the issue');
        expect((rows[3] as TimelineReasoning).isLive, isFalse);
        expect((rows[4] as TimelineAssistantText).text, 'Here is the fix.');
        expect((rows[4] as TimelineAssistantText).showCursor, isFalse);
        expect(rows[4].eventIndex, greaterThan(rows[3].eventIndex));
      },
    );

    test('streams contiguous assistant text onto the same item', () {
      final rows = buildThreadEventTimeline(
        <UiEventMessage>[
          UiEventMessage.messageStarted(
            messageId: 'assistant-1',
            role: MessageRole.assistant,
            startedAtMs: platformInt64FromInt(1),
          ),
          const UiEventMessage.textDelta(
            messageId: 'assistant-1',
            text: DisplayPayload.inline(text: 'Hel'),
          ),
          const UiEventMessage.textDelta(
            messageId: 'assistant-1',
            text: DisplayPayload.inline(text: 'lo'),
          ),
        ],
        showLiveAssistantState: true,
      );

      expect(rows, hasLength(1));
      final text = rows.single as TimelineAssistantText;
      expect(text.text, 'Hello');
      expect(text.showCursor, isTrue);
    });

    test('only open reasoning segment is live while turn continues', () {
      final rows = buildThreadEventTimeline(
        <UiEventMessage>[
          UiEventMessage.messageStarted(
            messageId: 'assistant-1',
            role: MessageRole.assistant,
            startedAtMs: platformInt64FromInt(1),
          ),
          const UiEventMessage.reasoningDelta(
            messageId: 'assistant-1',
            text: DisplayPayload.inline(text: 'first thought'),
          ),
          const UiEventMessage.toolCallPlaced(
            messageId: 'assistant-1',
            toolCallId: 'tc-1',
            name: 'grep',
            argsJson: DisplayPayload.inline(text: '{}'),
          ),
          const UiEventMessage.reasoningDelta(
            messageId: 'assistant-1',
            text: DisplayPayload.inline(text: 'second thought'),
          ),
        ],
        showLiveAssistantState: true,
      );

      expect(rows.map((row) => row.runtimeType).toList(), <Type>[
        TimelineReasoning,
        TimelineToolCall,
        TimelineReasoning,
      ]);
      expect((rows[0] as TimelineReasoning).isLive, isFalse);
      expect((rows[2] as TimelineReasoning).isLive, isTrue);
    });

    test('intermediate text loses cursor once tools begin', () {
      final rows = buildThreadEventTimeline(
        <UiEventMessage>[
          UiEventMessage.messageStarted(
            messageId: 'assistant-1',
            role: MessageRole.assistant,
            startedAtMs: platformInt64FromInt(1),
          ),
          const UiEventMessage.textDelta(
            messageId: 'assistant-1',
            text: DisplayPayload.inline(text: 'starting'),
          ),
          const UiEventMessage.toolCallPlaced(
            messageId: 'assistant-1',
            toolCallId: 'tc-1',
            name: 'read_file',
            argsJson: DisplayPayload.inline(text: '{}'),
          ),
        ],
        showLiveAssistantState: true,
      );

      expect(rows.map((row) => row.runtimeType).toList(), <Type>[
        TimelineAssistantText,
        TimelineToolCall,
      ]);
      expect((rows[0] as TimelineAssistantText).showCursor, isFalse);
    });
  });
}
