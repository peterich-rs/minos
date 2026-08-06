/**
 * Empty workspace caches, idle fetch status, bootstrap flight, refresh timers.
 */
import type { Conversation, Project, TimelineMessage } from "@/shared/lib/mock-data";
import type { TranscriptItem } from "@/shared/lib/daemon";
import type { MessageHistoryMeta } from "@/shared/lib/message-history";
import type { TranscriptHistoryMeta } from "@/shared/lib/transcript-history";
import type { SessionEntity } from "@/shared/lib/session-entity";
import type {
  ProjectSession,
  ResourceFetchStatus,
} from "./types";

/** Debounced quiet Timeline re-list timers (conversationId → timeout handle). */
export const conversationRefreshTimers = new Map<
  string,
  ReturnType<typeof setTimeout>
>();

/** Cancel pending quiet re-list timers (workspace boundary / bootstrap wipe). */
export function clearConversationRefreshTimers(): void {
  for (const handle of conversationRefreshTimers.values()) {
    clearTimeout(handle);
  }
  conversationRefreshTimers.clear();
}

/** In-flight bootstrap so React StrictMode double-mount cannot wipe loads. */
let bootstrapInFlight: Promise<void> | null = null;

export function getBootstrapInFlight(): Promise<void> | null {
  return bootstrapInFlight;
}

export function setBootstrapInFlight(p: Promise<void> | null): void {
  bootstrapInFlight = p;
}

export const idleStatus = (): ResourceFetchStatus => ({
  phase: "idle",
  generation: 0,
});

export const emptyWorkspace = {
  projects: [] as Project[],
  conversations: [] as Conversation[],
  conversationsStatusByProject: {} as Record<string, ResourceFetchStatus>,
  messagesByConversation: {} as Record<string, TimelineMessage[]>,
  messageHistoryByConversation: {} as Record<string, MessageHistoryMeta>,
  sessionsByConversation: {} as Record<string, ProjectSession[]>,
  timelineStatusByConversation: {} as Record<string, ResourceFetchStatus>,
  inspectorStatusByConversation: {} as Record<string, ResourceFetchStatus>,
  projectSessionsByProject: {} as Record<string, ProjectSession[]>,
  projectSessionsStatusByProject: {} as Record<string, ResourceFetchStatus>,
  sessionsById: {} as Record<string, SessionEntity>,
  transcriptsBySession: {} as Record<string, TranscriptItem[]>,
  transcriptStatusBySession: {} as Record<string, ResourceFetchStatus>,
  transcriptHistoryBySession: {} as Record<string, TranscriptHistoryMeta>,
  attentionSessions: [] as ProjectSession[],
  attentionStatus: idleStatus(),
  clisStatus: idleStatus(),
};

