import { create } from 'zustand'
import { persist } from 'zustand/middleware'
import {
  type AgentDescriptor,
  type HostSummary,
  type StoredSession,
  type ThreadSummary,
  type SocialMessageFrame,
  type ReadThreadResponse,
  ensureBrowserDeviceId,
  loadStoredSession,
  saveStoredSession,
  clearStoredSession,
  loadActiveHost,
  saveActiveHost,
} from './minos'

import { RelaySocket, type RelayConnectionState } from './relay-socket'

type AuthMode = 'login' | 'register'

/** Sidebar routes — the single source of truth for the app layout. */
export type RouteKey = 'chat' | 'tasks' | 'friends' | 'devices' | 'profile' | 'settings'

export interface AppState {
  // Auth & Session
  deviceId: string
  session: StoredSession | null
  authMode: AuthMode
  setSession: (session: StoredSession | null) => void
  setAuthMode: (mode: AuthMode) => void
  logout: () => void

  // Navigation
  route: RouteKey
  setRoute: (route: RouteKey) => void

  // Hosts & Relay
  hosts: HostSummary[]
  activeHost: string | null
  connectionState: RelayConnectionState
  runtimeAgents: AgentDescriptor[]
  relaySocket: RelaySocket | null
  setHosts: (hosts: HostSummary[]) => void
  setActiveHost: (hostId: string | null) => void
  setConnectionState: (state: RelayConnectionState) => void
  setRuntimeAgents: (agents: AgentDescriptor[]) => void
  setRelaySocket: (socket: RelaySocket | null) => void

  // Threads & Chat
  threads: ThreadSummary[]
  selectedThreadId: string | null
  threadRecords: Record<string, ReadThreadResponse>
  latestSocialEvent: SocialMessageFrame | null
  setThreads: (threads: ThreadSummary[]) => void
  setSelectedThreadId: (id: string | null) => void
  setThreadRecords: (records: Record<string, ReadThreadResponse>) => void
  updateThreadRecord: (id: string, updateFn: (prev: ReadThreadResponse) => ReadThreadResponse) => void
  setLatestSocialEvent: (event: SocialMessageFrame | null) => void

  // Composer
  composerText: string
  setComposerText: (text: string) => void
}

export const useAppStore = create<AppState>()(
  persist(
    (set) => ({
      deviceId: ensureBrowserDeviceId(),
      session: loadStoredSession(),
      authMode: 'login',
      setSession: (session) => {
        if (session) saveStoredSession(session)
        else clearStoredSession()
        set({ session })
      },
      setAuthMode: (authMode) => set({ authMode }),
      logout: () => {
        clearStoredSession()
        set({ session: null })
      },

      route: 'chat',
      setRoute: (route) => set({ route }),

      hosts: [],
      activeHost: loadActiveHost(),
      connectionState: 'idle',
      runtimeAgents: [],
      relaySocket: null,
      setHosts: (hosts) => set({ hosts }),
      setActiveHost: (activeHost) => {
        saveActiveHost(activeHost)
        set({ activeHost })
      },
      setConnectionState: (connectionState) => set({ connectionState }),
      setRuntimeAgents: (runtimeAgents) => set({ runtimeAgents }),
      setRelaySocket: (relaySocket) => set({ relaySocket }),

      threads: [],
      selectedThreadId: null,
      threadRecords: {},
      latestSocialEvent: null,
      setThreads: (threads) => set({ threads }),
      setSelectedThreadId: (selectedThreadId) => set({ selectedThreadId }),
      setThreadRecords: (threadRecords) => set({ threadRecords }),
      updateThreadRecord: (id, updateFn) =>
        set((state) => ({
          threadRecords: {
            ...state.threadRecords,
            [id]: updateFn(
              state.threadRecords[id] || {
                ui_events: [],
                next_seq: null,
                thread_end_reason: null,
              },
            ),
          },
        })),
      setLatestSocialEvent: (latestSocialEvent) => set({ latestSocialEvent }),

      composerText: '',
      setComposerText: (composerText) => set({ composerText }),
    }),
    {
      name: 'minos-app-store',
      partialize: (state) => ({
        route: state.route,
      }),
    },
  ),
)
