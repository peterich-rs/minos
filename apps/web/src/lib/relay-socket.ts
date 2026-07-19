import type {
  ChatMessageSummary,
  SocialMessageFrame,
  UiEventFrame,
  UiEventMessage,
} from './minos'

export type RelayConnectionState = 'idle' | 'connecting' | 'connected' | 'closed' | 'error'

type PendingRequest = {
  resolve: (value: unknown) => void
  reject: (reason?: unknown) => void
  timeout: number
}

type EnvelopeEvent =
  | { kind: 'event'; v: number; type: 'ui_event_message'; thread_id: string; seq: number; ui: UiEventMessage; ts_ms: number }
  | { kind: 'event'; v: number; type: 'social_message'; conversation_id: string; message: ChatMessageSummary }
  | { kind: 'event'; v: number; type: 'unpaired' | 'server_shutdown' }
  | { kind: 'event'; v: number; type: 'peer_online' | 'peer_offline'; peer_device_id: string }
  | { kind: 'forwarded'; v: number; from: string; payload: Record<string, unknown> }

export type AutoReconnectOptions = {
  ticketProvider: () => Promise<string>
  attempts?: number[]
}

export type RelaySocketOptions = {
  wsBaseUrl: string
  onUiEvent: (frame: UiEventFrame) => void
  onSocialMessage?: (frame: SocialMessageFrame) => void
  onConnectionState: (state: RelayConnectionState, message?: string) => void
  onServerNotice: (message: string) => void
  autoReconnect?: AutoReconnectOptions
}

export class RelaySocket {
  private readonly options: RelaySocketOptions
  private socket: WebSocket | null = null
  private nextId = 1
  private pending = new Map<number, PendingRequest>()
  private explicitClose = false
  private reconnectIndex = 0
  private reconnectTimer: number | null = null

  constructor(options: RelaySocketOptions) {
    this.options = options
  }

  get state(): RelayConnectionState {
    if (!this.socket) return 'idle'
    switch (this.socket.readyState) {
      case WebSocket.CONNECTING:
        return 'connecting'
      case WebSocket.OPEN:
        return 'connected'
      default:
        return 'closed'
    }
  }

  async connect(ticket: string): Promise<void> {
    this.explicitClose = false
    this.closeInternal()
    this.options.onConnectionState('connecting')

    await new Promise<void>((resolve, reject) => {
      const ws = new WebSocket(
        `${this.options.wsBaseUrl}/ws/client?ticket=${encodeURIComponent(ticket)}`,
      )
      this.socket = ws

      ws.onopen = () => {
        this.reconnectIndex = 0
        this.options.onConnectionState('connected')
        resolve()
      }

      ws.onerror = () => {
        const error = new Error('websocket connection failed')
        this.options.onConnectionState('error', error.message)
        reject(error)
      }

      ws.onclose = (event) => {
        const reason = event.reason || `socket closed (${event.code})`
        this.rejectAll(reason)
        this.options.onConnectionState('closed', reason)
        if (!this.explicitClose) {
          this.scheduleReconnect()
        }
      }

      ws.onmessage = (event) => {
        this.handleMessage(event.data)
      }
    })
  }

  close(): void {
    this.explicitClose = true
    this.cancelReconnect()
    this.closeInternal()
  }

  async sendRpc<T>(
    targetDeviceId: string,
    method: string,
    params: unknown,
  ): Promise<T> {
    const ws = this.socket
    if (!ws || ws.readyState !== WebSocket.OPEN) {
      throw new Error('socket is not connected')
    }

    const requestId = this.nextId++
    const envelope = {
      kind: 'forward',
      v: 1,
      target_device_id: targetDeviceId,
      payload: {
        jsonrpc: '2.0',
        id: requestId,
        method,
        params,
      },
    }

    return new Promise<T>((resolve, reject) => {
      const timeout = window.setTimeout(() => {
        this.pending.delete(requestId)
        reject(new Error(`${method} timed out`))
      }, 15000)

      this.pending.set(requestId, {
        resolve: (value) => resolve(value as T),
        reject,
        timeout,
      })

      ws.send(JSON.stringify(envelope))
    })
  }

  private scheduleReconnect(): void {
    const reconnect = this.options.autoReconnect
    if (!reconnect) return

    const backoffs = reconnect.attempts ?? [1000, 2000, 5000]
    if (this.reconnectIndex >= backoffs.length) {
      this.options.onConnectionState('error', 'relay offline')
      return
    }

    const delay = backoffs[this.reconnectIndex]
    this.reconnectIndex++

    this.reconnectTimer = window.setTimeout(() => {
      this.reconnectTimer = null
      void this.attemptReconnect(reconnect)
    }, delay)
  }

  private async attemptReconnect(reconnect: AutoReconnectOptions): Promise<void> {
    try {
      const ticket = await reconnect.ticketProvider()
      await this.connect(ticket)
    } catch {
      // connect() already emits state; scheduleReconnect will be called from onclose
    }
  }

  private cancelReconnect(): void {
    if (this.reconnectTimer !== null) {
      window.clearTimeout(this.reconnectTimer)
      this.reconnectTimer = null
    }
  }

  private closeInternal(): void {
    this.rejectAll('socket replaced')
    if (this.socket) {
      this.socket.onclose = null
      this.socket.onerror = null
      this.socket.onmessage = null
      this.socket.onopen = null
      this.socket.close()
      this.socket = null
    }
  }

  private handleMessage(raw: unknown): void {
    if (typeof raw !== 'string') {
      return
    }

    let envelope: EnvelopeEvent
    try {
      envelope = JSON.parse(raw) as EnvelopeEvent
    } catch {
      return
    }

    if (envelope.kind === 'forwarded') {
      const id = Number(envelope.payload.id)
      const pending = this.pending.get(id)
      if (!pending) {
        return
      }
      window.clearTimeout(pending.timeout)
      this.pending.delete(id)

      if ('error' in envelope.payload && envelope.payload.error) {
        const errorRecord = envelope.payload.error as Record<string, unknown>
        pending.reject(new Error(String(errorRecord.message ?? 'rpc failed')))
        return
      }
      pending.resolve(envelope.payload.result)
      return
    }

    if (envelope.kind !== 'event') {
      return
    }

    if (envelope.type === 'ui_event_message') {
      this.options.onUiEvent({
        thread_id: envelope.thread_id,
        seq: envelope.seq,
        ui: envelope.ui,
        ts_ms: envelope.ts_ms,
      })
      return
    }

    if (envelope.type === 'social_message') {
      this.options.onSocialMessage?.({
        conversation_id: envelope.conversation_id,
        message: envelope.message,
      })
      return
    }

    if (envelope.type === 'server_shutdown') {
      this.options.onServerNotice('backend requested a reconnect')
      return
    }

    if (envelope.type === 'peer_offline') {
      this.options.onServerNotice(`host ${envelope.peer_device_id} went offline`)
    }
  }

  private rejectAll(message: string): void {
    for (const pending of this.pending.values()) {
      window.clearTimeout(pending.timeout)
      pending.reject(new Error(message))
    }
    this.pending.clear()
  }
}
