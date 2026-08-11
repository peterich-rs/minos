import type { AgentName } from './minos'

/**
 * Web local workspace cache for agent launch prefs (model/effort bindings).
 *
 * **Product bot identity SSOT is Hub `agents`** (global bot directory + digital
 * body). This module is a browser-only cache for Web workbench UX until Web
 * fully uses Hub bot CRUD + participants APIs — not a multi-end identity store.
 *
 * Bot identity SSOT: Hub `agents` (see docs/architecture-overview.md).
 */

export type AgentReasoningEffort = 'low' | 'medium' | 'high'

export interface AgentEnvironmentVariable {
  key: string
  value: string
}

export interface AgentProfile {
  id: string
  name: string
  description: string
  runtimeAgent: AgentName
  model: string
  reasoningEffort: AgentReasoningEffort
  environmentVariables: AgentEnvironmentVariable[]
  hostDeviceId?: string | null
  hostDisplayName?: string | null
  createdAtMs: number
  updatedAtMs: number
}

export interface AgentWorkspaceState {
  profiles: AgentProfile[]
  preferredProfileId: string | null
  threadProfileBindings: Record<string, string>
}

const STORAGE_KEY = 'minos.web.agent-workspace'

const DEFAULT_WORKSPACE: AgentWorkspaceState = {
  profiles: [],
  preferredProfileId: null,
  threadProfileBindings: {},
}

function storageAvailable(): boolean {
  return typeof window !== 'undefined' && typeof window.localStorage !== 'undefined'
}

function normalizeEnvironmentVariables(
  entries: AgentEnvironmentVariable[],
): AgentEnvironmentVariable[] {
  return entries
    .map((entry) => ({
      key: entry.key.trim(),
      value: entry.value,
    }))
    .filter((entry) => entry.key.length > 0)
}

function normalizeProfile(profile: AgentProfile): AgentProfile {
  return {
    ...profile,
    name: profile.name.trim() || 'Agent',
    description: profile.description.trim(),
    model: profile.model.trim() || 'GPT-5.5',
    environmentVariables: normalizeEnvironmentVariables(
      profile.environmentVariables,
    ),
    hostDeviceId: profile.hostDeviceId?.trim() || null,
    hostDisplayName: profile.hostDisplayName?.trim() || null,
  }
}

export function normalizeAgentWorkspace(
  workspace: AgentWorkspaceState,
): AgentWorkspaceState {
  const profiles = workspace.profiles.map(normalizeProfile)
  const profileIds = new Set(profiles.map((profile) => profile.id))
  const preferredProfileId =
    workspace.preferredProfileId &&
    profiles.some((profile) => profile.id === workspace.preferredProfileId)
      ? workspace.preferredProfileId
      : profiles[0]?.id ?? null
  const threadProfileBindings = Object.fromEntries(
    Object.entries(workspace.threadProfileBindings ?? {}).filter(([, profileId]) =>
      profileIds.has(profileId),
    ),
  )

  return {
    profiles,
    preferredProfileId,
    threadProfileBindings,
  }
}

export function loadAgentWorkspace(): AgentWorkspaceState {
  if (!storageAvailable()) {
    return DEFAULT_WORKSPACE
  }
  const raw = window.localStorage.getItem(STORAGE_KEY)
  if (!raw) {
    return DEFAULT_WORKSPACE
  }
  try {
    return normalizeAgentWorkspace(
      JSON.parse(raw) as AgentWorkspaceState,
    )
  } catch {
    window.localStorage.removeItem(STORAGE_KEY)
    return DEFAULT_WORKSPACE
  }
}

export function saveAgentWorkspace(workspace: AgentWorkspaceState): void {
  if (!storageAvailable()) {
    return
  }
  window.localStorage.setItem(
    STORAGE_KEY,
    JSON.stringify(normalizeAgentWorkspace(workspace)),
  )
}

export function createAgentProfileId(): string {
  return `agent-${Date.now().toString(36)}`
}

export function profileById(
  workspace: AgentWorkspaceState,
  profileId: string | null | undefined,
): AgentProfile | null {
  if (!profileId) {
    return null
  }
  return workspace.profiles.find((profile) => profile.id === profileId) ?? null
}

export function preferredProfile(
  workspace: AgentWorkspaceState,
): AgentProfile | null {
  return (
    profileById(workspace, workspace.preferredProfileId) ??
    workspace.profiles[0] ??
    null
  )
}
