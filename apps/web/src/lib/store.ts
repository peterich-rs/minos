import { create } from 'zustand'
import { persist } from 'zustand/middleware'
import {
  type AgentDescriptor,
  type HostSummary,
  type StoredSession,
  type SessionSummary,
  type SocialMessageFrame,
  type ReadSessionResponse,
  ensureBrowserDeviceId,
  loadStoredSession,
  saveStoredSession,
  clearStoredSession,
  loadActiveHost,
  saveActiveHost,
} from './minos'

import { RelaySocket, type RelayConnectionState } from './relay-socket'

type AuthMode = 'login' | 'register'

/** Legacy demo routes (pre-Desktop chrome). Prefer `primaryNav`. */
export type RouteKey = 'chat' | 'tasks' | 'friends' | 'devices' | 'profile' | 'settings'

/** Desktop-family primary nav for CloudShell. */
export type PrimaryNav = 'work' | 'attention' | 'hosts' | 'settings'

export interface AppState {
  // Auth & Session
  deviceId: string
  session: StoredSession | null
  authMode: AuthMode
  setSession: (session: StoredSession | null) => void
  setAuthMode: (mode: AuthMode) => void
  logout: () => void

  // Navigation (Desktop-aligned cloud shell)
  primaryNav: PrimaryNav
  setPrimaryNav: (nav: PrimaryNav) => void
  mockProjectId: string
  setMockProjectId: (id: string) => void
  mockSessionId: string | null
  setMockSessionId: (id: string | null) => void

  // Legacy Navigation
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
  sessions: SessionSummary[]
  selectedThreadId: string | null
  threadRecords: Record<string, ReadSessionResponse>
  latestSocialEvent: SocialMessageFrame | null
  setThreads: (sessions: SessionSummary[]) => void
  setSelectedThreadId: (id: string | null) => void
  setThreadRecords: (records: Record<string, ReadSessionResponse>) => void
  updateThreadRecord: (id: string, updateFn: (prev: ReadSessionResponse) => ReadSessionResponse) => void
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

      primaryNav: 'work',
      setPrimaryNav: (primaryNav) => set({ primaryNav }),
      mockProjectId: 'proj-minos',
      setMockProjectId: (mockProjectId) => set({ mockProjectId }),
      mockSessionId: 'sess-1',
      setMockSessionId: (mockSessionId) => set({ mockSessionId }),

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

      sessions: [],
      selectedThreadId: null,
      threadRecords: {},
      latestSocialEvent: null,
      setThreads: (sessions) => set({ sessions }),
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
                session_end_reason: null,
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
