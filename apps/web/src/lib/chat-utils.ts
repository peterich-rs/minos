import { type UiEventMessage, type MessageRole } from './minos'

export type TranscriptToolCall = {
  toolCallId: string
  name: string
  argsJson: string
  output: string
  isError: boolean
  completed: boolean
}

export type TranscriptMessage = {
  messageId: string
  role: MessageRole
  text: string
  reasoning: string
  startedAtMs: number | null
  finishedAtMs: number | null
  toolCalls: TranscriptToolCall[]
}

export function transcriptFromEvents(events: UiEventMessage[]) {
  const messageOrder: string[] = []
  const messages = new Map<string, TranscriptMessage>()
  const errors: Array<{ code: string; message: string }> = []
  let title: string | null = null
  let endReason: string | null = null

  for (const event of events) {
    if (event.kind === 'thread_opened') {
      title = event.title ?? title
      continue
    }
    if (event.kind === 'thread_title_updated') {
      title = event.title
      continue
    }
    if (event.kind === 'thread_closed') {
      endReason =
        event.reason.kind === 'crashed'
          ? event.reason.message ?? 'thread crashed'
          : event.reason.kind.replaceAll('_', ' ')
      continue
    }
    if (event.kind === 'message_started') {
      messageOrder.push(event.message_id)
      messages.set(event.message_id, {
        messageId: event.message_id,
        role: event.role,
        text: '',
        reasoning: '',
        startedAtMs: event.started_at_ms,
        finishedAtMs: null,
        toolCalls: [],
      })
      continue
    }
    if (event.kind === 'message_completed') {
      const message = messages.get(event.message_id)
      if (message) {
        message.finishedAtMs = event.finished_at_ms
      }
      continue
    }
    if (event.kind === 'text_delta') {
      const message = messages.get(event.message_id)
      if (message) {
        message.text += event.text
      }
      continue
    }
    if (event.kind === 'reasoning_delta') {
      const message = messages.get(event.message_id)
      if (message) {
        message.reasoning += event.text
      }
      continue
    }
    if (event.kind === 'tool_call_placed') {
      const message = messages.get(event.message_id)
      if (message) {
        message.toolCalls.push({
          toolCallId: event.tool_call_id,
          name: event.name,
          argsJson: event.args_json,
          output: '',
          isError: false,
          completed: false,
        })
      }
      continue
    }
    if (event.kind === 'tool_call_completed') {
      for (const message of messages.values()) {
        const toolCall = message.toolCalls.find(
          (candidate) => candidate.toolCallId === event.tool_call_id,
        )
        if (toolCall) {
          toolCall.output = event.output
          toolCall.isError = event.is_error
          toolCall.completed = true
        }
      }
      continue
    }
    if (event.kind === 'error') {
      errors.push({
        code: event.code,
        message: event.message,
      })
    }
  }

  return {
    title,
    endReason,
    errors,
    messages: messageOrder
      .map((messageId) => messages.get(messageId))
      .filter((message): message is TranscriptMessage => Boolean(message)),
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
