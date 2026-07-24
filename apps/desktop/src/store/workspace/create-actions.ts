/**
 * Workspace store actions factory (L1–L6) — composes focused modules.
 */
import type { WorkspaceGet, WorkspaceSet, WorkspaceState } from "./types";
import { createConnectionActions } from "./connection";
import { createConversationListActions } from "./conversation-list";
import { createTimelineActions } from "./timeline";
import { createInspectorActions } from "./inspector";
import { createSessionListActions } from "./session-list";
import { createTranscriptActions } from "./transcript";
import { createAttentionActions } from "./attention";
import { createLiveIngressActions } from "./live-ingress";
import { createAgentsHostActions } from "./agents-host";
import { createUseCasesActions } from "./use-cases";

export function createWorkspaceActions(
  set: WorkspaceSet,
  get: WorkspaceGet,
): Pick<
  WorkspaceState,
  | "bootstrap"
  | "refreshProjects"
  | "clearActionError"
  | "loadConversations"
  | "loadTimeline"
  | "loadInspector"
  | "loadOlderMessages"
  | "loadProjectSessions"
  | "loadTranscript"
  | "resumeInterruptedSession"
  | "loadAttentionSessions"
  | "resolveApproval"
  | "respondOpencodePermission"
  | "respondOpencodeQuestion"
  | "markConversationRead"
  | "sendMessage"
  | "retryFailedMessage"
  | "createConversation"
  | "updateConversationTitle"
  | "cycleConversationPriority"
  | "cycleConversationProgress"
  | "setConversationProgress"
  | "moveConversationToBoardColumn"
  | "createProject"
  | "loadClis"
  | "applyIngestEvent"
  | "applyManagerEvent"
  | "applyConversationEvent"
> {
  return {
    ...createConnectionActions(set, get),
    ...createConversationListActions(set, get),
    ...createTimelineActions(set, get),
    ...createInspectorActions(set, get),
    ...createSessionListActions(set, get),
    ...createTranscriptActions(set, get),
    ...createAttentionActions(set, get),
    ...createLiveIngressActions(set, get),
    ...createAgentsHostActions(set, get),
    ...createUseCasesActions(set, get),
  };
}
