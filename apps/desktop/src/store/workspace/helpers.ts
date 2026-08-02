/**
 * Workspace helpers barrel — prefer importing from the focused modules:
 * - `dto-map`            Daemon DTO → UI mapping
 * - `transcript-merge`   transcript item merge/dedupe
 * - `empty-workspace`    empty caches / bootstrap flight / timers
 * - `mock-bundle`        browser mock seed + CLI fallback inventory
 *
 * Kept so existing `from "./helpers"` imports stay stable.
 */
export {
  conversationRefreshTimers,
  clearConversationRefreshTimers,
  getBootstrapInFlight,
  setBootstrapInFlight,
  idleStatus,
  emptyWorkspace,
} from "./empty-workspace";
export {
  resetWorkspaceModuleState,
  type ResetWorkspaceModuleStateOptions,
  type WorkspaceResetReactions,
} from "./reset-workspace-state";
export {
  KNOWN_AGENTS_FALLBACK,
  mockBundle,
} from "./mock-bundle";
export {
  coerceUiSessionStatus,
  bumpStatus,
  toUiProject,
  toUiProjects,
  normalizeDaemonConversation,
  toUiConversation,
  patchLocalConversation,
  toUiMessage,
  toUiSession,
  patchProjectAggregates,
} from "./dto-map";
export {
  mergeToolLifecycleItems,
  dedupeTranscriptItemsById,
  mergeTranscriptItems,
} from "./transcript-merge";
