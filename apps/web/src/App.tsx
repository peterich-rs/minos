import { useEffect, useRef } from 'react'
import { Toaster } from 'sonner'

import { useAppStore } from './lib/store'
import {
  type AgentDescriptor,
  type ThreadSummary,
  backendWsBase,
  createWsTicket,
  listHosts,
  listThreads,
  readThread,
  runWithSessionRefresh,
} from './lib/minos'
import { RelaySocket } from './lib/relay-socket'

import { AuthScreen } from './components/auth-screen'
import { AppShell } from './components/app-shell'
import { ThemeProvider } from './components/theme-provider'

async function fetchConsoleSnapshot(deviceId: string, accessToken: string) {
  const [hostResponse, threadResponse] = await Promise.all([
    listHosts(deviceId, accessToken),
    listThreads(deviceId, accessToken, { limit: 48 }),
  ])
  return {
    hosts: hostResponse.hosts,
    threads: threadResponse.threads,
  }
}

function RelayManager() {
  const {
    deviceId,
    session,
    setSession,
    setConnectionState,
    setRelaySocket,
    setHosts,
    setThreads,
    setSelectedThreadId,
    setActiveHost,
    activeHost,
    selectedThreadId,
    connectionState,
    setLatestSocialEvent,
    setRuntimeAgents,
  } = useAppStore()

  const socketRef = useRef<RelaySocket | null>(null)

  // 1. WebSocket lifecycle
  useEffect(() => {
    if (!session) {
      socketRef.current?.close()
      socketRef.current = null
      return
    }

    let cancelled = false
    const socket = new RelaySocket({
      wsBaseUrl: backendWsBase(),
      onUiEvent: (frame) => {
        useAppStore.getState().updateThreadRecord(frame.thread_id, (current) => ({
          ...current,
          ui_events: [...current.ui_events, frame.ui],
        }))

        const currentThreads = useAppStore.getState().threads
        const existingIndex = currentThreads.findIndex(
          (thread) => thread.thread_id === frame.thread_id,
        )

        let nextThreads = [...currentThreads]

        if (frame.ui.kind === 'thread_opened') {
          const openedEvent = frame.ui
          const openedThread: ThreadSummary = {
            thread_id: openedEvent.thread_id,
            agent: openedEvent.agent,
            title: openedEvent.title,
            first_ts_ms: frame.ts_ms,
            last_ts_ms: frame.ts_ms,
            message_count: 0,
            ended_at_ms: null,
            end_reason: null,
          }
          if (existingIndex === -1) {
            nextThreads = [openedThread, ...currentThreads]
          } else {
            nextThreads[existingIndex] = {
              ...currentThreads[existingIndex],
              title: openedEvent.title ?? currentThreads[existingIndex].title,
              last_ts_ms: frame.ts_ms,
            }
          }
        } else if (existingIndex !== -1) {
          const thread = { ...currentThreads[existingIndex] }
          if (frame.ui.kind === 'thread_title_updated') {
            thread.title = frame.ui.title
            thread.last_ts_ms = frame.ts_ms
          } else if (frame.ui.kind === 'thread_closed') {
            thread.ended_at_ms = frame.ui.closed_at_ms
            thread.end_reason = frame.ui.reason
            thread.last_ts_ms = frame.ts_ms
          } else if (frame.ui.kind === 'message_started') {
            thread.message_count += 1
            thread.last_ts_ms = frame.ts_ms
          } else {
            thread.last_ts_ms = frame.ts_ms
          }
          nextThreads[existingIndex] = thread
        }

        setThreads(nextThreads)
      },
      onSocialMessage: (frame) => {
        setLatestSocialEvent(frame)
      },
      onConnectionState: (state, message) => {
        if (cancelled) return
        setConnectionState(state)
        if (message && state !== 'connected') {
          console.warn(message)
        }
      },
      onServerNotice: (message) => {
        if (!cancelled) console.info(message)
      },
      autoReconnect: {
        ticketProvider: async () => {
          const ticketResponse = await runWithSessionRefresh(
            session,
            deviceId,
            setSession,
            (current) => createWsTicket(deviceId, current.accessToken),
          )
          return ticketResponse.ticket
        },
        attempts: [1000, 2000, 5000],
      },
    })
    socketRef.current = socket

    void (async () => {
      try {
        setRelaySocket(socket)
        const ticketResponse = await runWithSessionRefresh(
          session,
          deviceId,
          setSession,
          (current) => createWsTicket(deviceId, current.accessToken),
        )
        if (cancelled) return
        await socket.connect(ticketResponse.ticket)
        const snapshot = await runWithSessionRefresh(
          session,
          deviceId,
          setSession,
          (current) => fetchConsoleSnapshot(deviceId, current.accessToken),
          false,
        )
        if (cancelled) return

        setHosts(snapshot.hosts)
        setThreads(snapshot.threads)

        const currentSelected = useAppStore.getState().selectedThreadId
        if (!currentSelected || !snapshot.threads.some((t) => t.thread_id === currentSelected)) {
          setSelectedThreadId(snapshot.threads[0]?.thread_id ?? null)
        }

        const currentHost = useAppStore.getState().activeHost
        if (!currentHost || !snapshot.hosts.some((h) => h.host_device_id === currentHost)) {
          setActiveHost(snapshot.hosts[0]?.host_device_id ?? null)
        }
      } catch (error) {
        if (!cancelled) console.error(error instanceof Error ? error.message : String(error))
      }
    })()

    return () => {
      cancelled = true
      socket.close()
      setRelaySocket(null)
      if (socketRef.current === socket) {
        socketRef.current = null
      }
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [deviceId, session])

  // 2. Selected thread history
  useEffect(() => {
    if (!selectedThreadId || !session) return

    let cancelled = false
    void runWithSessionRefresh(session, deviceId, setSession, (current) =>
      readThread(deviceId, current.accessToken, selectedThreadId),
    )
      .then((response) => {
        if (!cancelled) {
          useAppStore.getState().setThreadRecords({
            ...useAppStore.getState().threadRecords,
            [selectedThreadId]: response,
          })
        }
      })
      .catch((error) => {
        if (!cancelled) console.error(error instanceof Error ? error.message : String(error))
      })

    return () => {
      cancelled = true
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [deviceId, selectedThreadId, session])

  // 3. Runtime agent listing
  useEffect(() => {
    if (!activeHost || connectionState !== 'connected') return
    const socket = socketRef.current
    if (!socket) return

    socket
      .sendRpc<AgentDescriptor[]>(activeHost, 'minos_list_clis', null)
      .then((agents) => {
        setRuntimeAgents(agents)
      })
      .catch((error) => {
        console.error(error instanceof Error ? error.message : String(error))
      })
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [activeHost, connectionState])

  // 4. Visibility-based reconnect
  useEffect(() => {
    if (!session) return

    let hiddenTimer: number | null = null

    function handleVisibilityChange() {
      if (document.hidden) {
        hiddenTimer = window.setTimeout(() => {
          hiddenTimer = null
          socketRef.current?.close()
        }, 30_000)
      } else {
        if (hiddenTimer !== null) {
          window.clearTimeout(hiddenTimer)
          hiddenTimer = null
        }
        const currentSocket = socketRef.current
        if (!currentSocket || currentSocket.state === 'closed' || currentSocket.state === 'error') {
          if (session) {
            setSession({
              accountId: session.accountId,
              email: session.email,
              accessToken: session.accessToken,
              refreshToken: session.refreshToken,
            })
          }
        }
      }
    }

    document.addEventListener('visibilitychange', handleVisibilityChange)
    return () => {
      document.removeEventListener('visibilitychange', handleVisibilityChange)
      if (hiddenTimer !== null) window.clearTimeout(hiddenTimer)
    }
  }, [session, setSession])

  return null
}

export default function App() {
  const session = useAppStore((state) => state.session)

  return (
    <ThemeProvider>
      <RelayManager />
      {session ? <AppShell /> : <AuthScreen />}
      <Toaster
        richColors
        position="top-center"
        toastOptions={{
          className: 'rounded-xl',
        }}
      />
    </ThemeProvider>
  )
}
