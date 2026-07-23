import { type UiEventMessage, type MessageRole } from './minos'

export type TranscriptToolCall = {
  toolCallId: string
  name: string
  argsJson: string
  output: string
  isError: boolean
  completed: boolean
}

/** @deprecated Prefer `TranscriptItem` timeline rows. Kept for type-compatible call sites. */
export type TranscriptMessage = {
  messageId: string
  role: MessageRole
  text: string
  reasoning: string
  startedAtMs: number | null
  finishedAtMs: number | null
  toolCalls: TranscriptToolCall[]
}

export type TranscriptItem =
  | {
      kind: 'user'
      id: string
      messageId: string
      text: string
      startedAtMs: number | null
    }
  | {
      kind: 'reasoning'
      id: string
      messageId: string
      text: string
    }
  | {
      kind: 'tool'
      id: string
      toolCall: TranscriptToolCall
    }
  | {
      kind: 'assistant_text'
      id: string
      messageId: string
      text: string
      finishedAtMs: number | null
      showCursor: boolean
    }
  | {
      kind: 'placeholder'
      id: string
      messageId: string
    }
  | {
      kind: 'system'
      id: string
      text: string
    }

type TimedItem = TranscriptItem & { eventIndex: number; rank: number }

/**
 * Project UiEventMessage[] into timeline rows ordered by first appearance.
 * Reasoning/tool/text that share one assistant message_id still get separate
 * items at the event where they first arrived; later deltas update in place.
 */
export function transcriptFromEvents(events: UiEventMessage[]) {
  const roleByMsg = new Map<string, MessageRole>()
  const completedMsgs = new Set<string>()
  const textByMsg = new Map<string, string>()
  const textFirstIndex = new Map<string, number>()
  const messageStartIndex = new Map<string, number>()
  const startedAtByMsg = new Map<string, number | null>()
  const finishedAtByMsg = new Map<string, number | null>()
  const toolById = new Map<string, TranscriptToolCall & { messageId: string }>()
  const toolFirstIndex = new Map<string, number>()
  const reasoningSegments: Array<{
    messageId: string
    eventIndex: number
    text: string
  }> = []
  const errors: Array<{ code: string; message: string }> = []
  let title: string | null = null
  let endReason: string | null = null
  let openReasoningMessageId: string | null = null
  let lastAssistantMessageId: string | null = null

  const closeReasoning = () => {
    openReasoningMessageId = null
  }

  const appendReasoning = (messageId: string, eventIndex: number, text: string) => {
    if (!text) return
    if (
      openReasoningMessageId !== messageId ||
      reasoningSegments.length === 0
    ) {
      reasoningSegments.push({ messageId, eventIndex, text })
      openReasoningMessageId = messageId
      return
    }
    reasoningSegments[reasoningSegments.length - 1].text += text
  }

  const replaceReasoning = (messageId: string, eventIndex: number, text: string) => {
    if (!text) {
      for (let i = reasoningSegments.length - 1; i >= 0; i -= 1) {
        if (reasoningSegments[i].messageId === messageId) {
          reasoningSegments.splice(i, 1)
        }
      }
      if (openReasoningMessageId === messageId) {
        openReasoningMessageId = null
      }
      return
    }
    if (
      openReasoningMessageId === messageId &&
      reasoningSegments.length > 0
    ) {
      reasoningSegments[reasoningSegments.length - 1].text = text
      return
    }
    reasoningSegments.push({ messageId, eventIndex, text })
    openReasoningMessageId = messageId
  }

  events.forEach((event, i) => {
    if (event.kind === 'session_opened') {
      title = event.title ?? title
      return
    }
    if (event.kind === 'thread_title_updated') {
      title = event.title
      return
    }
    if (event.kind === 'thread_closed') {
      endReason =
        event.reason.kind === 'crashed'
          ? event.reason.message ?? 'thread crashed'
          : event.reason.kind.replaceAll('_', ' ')
      return
    }
    if (event.kind === 'message_started') {
      if (!messageStartIndex.has(event.message_id)) {
        messageStartIndex.set(event.message_id, i)
      }
      roleByMsg.set(event.message_id, event.role)
      startedAtByMsg.set(event.message_id, event.started_at_ms)
      if (!textByMsg.has(event.message_id)) {
        textByMsg.set(event.message_id, '')
      }
      if (event.role === 'assistant') {
        lastAssistantMessageId = event.message_id
      }
      closeReasoning()
      return
    }
    if (event.kind === 'message_completed') {
      completedMsgs.add(event.message_id)
      finishedAtByMsg.set(event.message_id, event.finished_at_ms)
      closeReasoning()
      return
    }
    if (event.kind === 'text_delta' || event.kind === 'text_replace') {
      const next =
        event.kind === 'text_replace'
          ? event.text
          : (textByMsg.get(event.message_id) ?? '') + event.text
      textByMsg.set(event.message_id, next)
      if (!textFirstIndex.has(event.message_id) && next.length > 0) {
        textFirstIndex.set(event.message_id, i)
      }
      closeReasoning()
      return
    }
    if (event.kind === 'reasoning_delta') {
      appendReasoning(event.message_id, i, event.text)
      return
    }
    if (event.kind === 'reasoning_replace') {
      replaceReasoning(event.message_id, i, event.text)
      return
    }
    if (event.kind === 'tool_call_placed') {
      toolById.set(event.tool_call_id, {
        messageId: event.message_id,
        toolCallId: event.tool_call_id,
        name: event.name,
        argsJson: event.args_json,
        output: '',
        isError: false,
        completed: false,
      })
      if (!toolFirstIndex.has(event.tool_call_id)) {
        toolFirstIndex.set(event.tool_call_id, i)
      }
      closeReasoning()
      return
    }
    if (event.kind === 'tool_call_completed') {
      const existing = toolById.get(event.tool_call_id)
      if (existing) {
        existing.output = event.output
        existing.isError = event.is_error
        existing.completed = true
      } else {
        const messageId = lastAssistantMessageId ?? ''
        toolById.set(event.tool_call_id, {
          messageId,
          toolCallId: event.tool_call_id,
          name: '(unknown)',
          argsJson: '{}',
          output: event.output,
          isError: event.is_error,
          completed: true,
        })
        if (!toolFirstIndex.has(event.tool_call_id)) {
          toolFirstIndex.set(event.tool_call_id, i)
        }
      }
      return
    }
    if (event.kind === 'error') {
      errors.push({
        code: event.code,
        message: event.message,
      })
    }
  })

  const timed: TimedItem[] = []

  for (const [messageId, role] of roleByMsg.entries()) {
    if (role !== 'user') continue
    const text = textByMsg.get(messageId) ?? ''
    if (!text.trim()) continue
    timed.push({
      kind: 'user',
      id: `user:${messageId}`,
      messageId,
      text,
      startedAtMs: startedAtByMsg.get(messageId) ?? null,
      eventIndex: messageStartIndex.get(messageId) ?? 0,
      rank: 0,
    })
  }

  reasoningSegments.forEach((segment, index) => {
    if (!segment.text) return
    timed.push({
      kind: 'reasoning',
      id: `reasoning:${segment.messageId}:${segment.eventIndex}:${index}`,
      messageId: segment.messageId,
      text: segment.text,
      eventIndex: segment.eventIndex,
      rank: 1,
    })
  })

  for (const [toolCallId, eventIndex] of toolFirstIndex.entries()) {
    const tool = toolById.get(toolCallId)
    if (!tool) continue
    timed.push({
      kind: 'tool',
      id: `tool:${toolCallId}`,
      toolCall: {
        toolCallId: tool.toolCallId,
        name: tool.name,
        argsJson: tool.argsJson,
        output: tool.output,
        isError: tool.isError,
        completed: tool.completed,
      },
      eventIndex,
      rank: 2,
    })
  }

  for (const [messageId, eventIndex] of textFirstIndex.entries()) {
    const role = roleByMsg.get(messageId) ?? 'assistant'
    if (role === 'user') continue
    const text = textByMsg.get(messageId) ?? ''
    if (!text) continue
    const finishedAtMs = finishedAtByMsg.get(messageId) ?? null
    timed.push({
      kind: 'assistant_text',
      id: `assistant:${messageId}`,
      messageId,
      text,
      finishedAtMs,
      showCursor: finishedAtMs == null && messageId === lastAssistantMessageId,
      eventIndex,
      rank: 3,
    })
  }

  if (
    lastAssistantMessageId &&
    !completedMsgs.has(lastAssistantMessageId)
  ) {
    const hasText = Boolean(textByMsg.get(lastAssistantMessageId))
    const hasReasoning = reasoningSegments.some(
      (segment) => segment.messageId === lastAssistantMessageId,
    )
    const hasTool = [...toolById.values()].some(
      (tool) => tool.messageId === lastAssistantMessageId,
    )
    if (!hasText && !hasReasoning && !hasTool) {
      timed.push({
        kind: 'placeholder',
        id: `placeholder:${lastAssistantMessageId}`,
        messageId: lastAssistantMessageId,
        eventIndex: messageStartIndex.get(lastAssistantMessageId) ?? events.length,
        rank: 4,
      })
    }
  }

  for (const [index, error] of errors.entries()) {
    timed.push({
      kind: 'system',
      id: `error:${index}:${error.code}`,
      text: `${error.code}: ${error.message}`,
      eventIndex: events.length + index,
      rank: 5,
    })
  }

  if (endReason) {
    timed.push({
      kind: 'system',
      id: `closed:${endReason}`,
      text: endReason,
      eventIndex: events.length + errors.length,
      rank: 6,
    })
  }

  timed.sort((a, b) => {
    if (a.eventIndex !== b.eventIndex) return a.eventIndex - b.eventIndex
    return a.rank - b.rank
  })

  const items: TranscriptItem[] = timed.map(({ eventIndex: _e, rank: _r, ...item }) => item)

  // Legacy message aggregation for any call site that still expects messages[].
  const messages: TranscriptMessage[] = []
  const messageOrder: string[] = []
  const messagesById = new Map<string, TranscriptMessage>()
  for (const item of items) {
    if (item.kind === 'user') {
      messageOrder.push(item.messageId)
      messagesById.set(item.messageId, {
        messageId: item.messageId,
        role: 'user',
        text: item.text,
        reasoning: '',
        startedAtMs: item.startedAtMs,
        finishedAtMs: null,
        toolCalls: [],
      })
    }
  }
  for (const item of items) {
    if (item.kind === 'assistant_text' || item.kind === 'reasoning' || item.kind === 'placeholder') {
      const messageId = item.messageId
      if (!messagesById.has(messageId)) {
        messageOrder.push(messageId)
        messagesById.set(messageId, {
          messageId,
          role: 'assistant',
          text: '',
          reasoning: '',
          startedAtMs: startedAtByMsg.get(messageId) ?? null,
          finishedAtMs: finishedAtByMsg.get(messageId) ?? null,
          toolCalls: [],
        })
      }
    }
    if (item.kind === 'tool') {
      // associate with last assistant if needed
      const messageId = lastAssistantMessageId
      if (messageId && !messagesById.has(messageId)) {
        messageOrder.push(messageId)
        messagesById.set(messageId, {
          messageId,
          role: 'assistant',
          text: '',
          reasoning: '',
          startedAtMs: startedAtByMsg.get(messageId) ?? null,
          finishedAtMs: finishedAtByMsg.get(messageId) ?? null,
          toolCalls: [],
        })
      }
    }
  }
  for (const item of items) {
    if (item.kind === 'assistant_text') {
      const msg = messagesById.get(item.messageId)
      if (msg) {
        msg.text = item.text
        msg.finishedAtMs = item.finishedAtMs
      }
    } else if (item.kind === 'reasoning') {
      const msg = messagesById.get(item.messageId)
      if (msg) {
        msg.reasoning = msg.reasoning ? `${msg.reasoning}\n${item.text}` : item.text
      }
    } else if (item.kind === 'tool') {
      const messageId =
        [...toolById.entries()].find(([, t]) => t.toolCallId === item.toolCall.toolCallId)?.[1]
          .messageId ?? lastAssistantMessageId
      if (messageId) {
        const msg = messagesById.get(messageId)
        msg?.toolCalls.push(item.toolCall)
      }
    }
  }
  for (const id of messageOrder) {
    const msg = messagesById.get(id)
    if (msg) messages.push(msg)
  }

  return {
    title,
    endReason,
    errors,
    items,
    messages,
  }
}

export function formatClock(timestampMs: number | null): string {
  if (!timestampMs) {
    return ''
  }
  return new Intl.DateTimeFormat('en', {
    hour: '2-digit',
    minute: '2-digit',
  }).format(timestampMs)
}

export function formatRelative(timestampMs: number): string {
  const deltaMinutes = Math.round((Date.now() - timestampMs) / 60000)
  if (deltaMinutes <= 1) {
    return 'just now'
  }
  if (deltaMinutes < 60) {
    return `${deltaMinutes}m ago`
  }
  const deltaHours = Math.round(deltaMinutes / 60)
  if (deltaHours < 24) {
    return `${deltaHours}h ago`
  }
  const deltaDays = Math.round(deltaHours / 24)
  return `${deltaDays}d ago`
}
