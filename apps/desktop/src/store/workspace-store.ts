/**
 * Workspace store — thin composer.
 * App import path stays `@/store/workspace-store`.
 *
 * Module map (consumption layers L0–L6):
 * - workspace/types.ts              shared WorkspaceState types
 * - workspace/helpers.ts            barrel → dto-map / transcript-merge /
 *                                   empty-workspace / mock-bundle
 * - workspace/reset-workspace-state.ts  module singleton teardown on boundary
 * - workspace/projection.ts         SessionEntity commit + list projection
 * - workspace/shared.ts             cross-slice use-case helpers
 * - workspace/connection.ts         L1 bootstrap / livePush / projects
 * - workspace/conversation-list.ts  L3a ConversationList
 * - workspace/timeline.ts           L3a Timeline
 * - workspace/inspector.ts          L3a Inspector
 * - workspace/session-list.ts       L3b SessionList
 * - workspace/transcript.ts         L3b Transcript
 * - workspace/attention.ts          Attention queue (not badge)
 * - workspace/live-ingress.ts       L5 push apply
 * - workspace/agents-host.ts        Agents CLI inventory
 * - workspace/use-cases.ts          L6 send / approvals / mutations
 * - workspace/create-actions.ts     compose factories
 */
import { create } from "zustand";
import { createJSONStorage, persist } from "zustand/middleware";
import type { WorkspaceState } from "./workspace/types";
import {
  emptyWorkspace,
  KNOWN_AGENTS_FALLBACK,
  mergeTranscriptItems,
} from "./workspace/helpers";
import { createWorkspaceActions } from "./workspace/create-actions";

export type {
  DataSource,
  ResourceFetchPhase,
  ResourceFetchStatus,
  ConversationDetailPhase,
  ConversationDetailStatus,
  ProjectSession,
  WorkspaceState,
  SessionEntity,
} from "./workspace/types";

export { mergeTranscriptItems };

export const useWorkspaceStore = create<WorkspaceState>()(
  persist(
    (set, get) => ({
      booting: true,
      bootPhase: "Starting…",
      bootProgress: 5,
      bootEpoch: 0,
      workspaceAccountId: null,
      livePush: false,
      source: "daemon",
      connection: null,
      loading: false,
      error: null,
      actionError: null,
      clis: KNOWN_AGENTS_FALLBACK,
      readMessageCountById: {},
      focusedConversationId: null,
      ...emptyWorkspace,
      ...createWorkspaceActions(set, get),
    }),
    {
      name: "minos.workspace-store.v1",
      storage: createJSONStorage(() => localStorage),
      partialize: (s) => ({
        readMessageCountById: s.readMessageCountById,
      }),
    },
  ),
);
