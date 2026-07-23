/** Stable TanStack Query keys for daemon catalog / index lists. */
export const queryKeys = {
  projects: ["projects"] as const,
  conversations: (projectId: string) =>
    ["projects", projectId, "conversations"] as const,
  clis: ["clis"] as const,
  agentProfiles: ["agentProfiles"] as const,
  models: (runtime: string) => ["models", runtime] as const,
  projectSessions: (projectId: string) =>
    ["projects", projectId, "sessions"] as const,
  inspectorSessions: (conversationId: string) =>
    ["conversations", conversationId, "sessions"] as const,
} as const;
