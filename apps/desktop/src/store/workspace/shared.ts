/**
 * Cross-slice helpers used by multiple workspace action modules.
 */
import type { WorkspaceGet } from "./types";

/**
 * After send/retry: always quiet-reload Timeline; quiet-reload Inspector only
 * when that working set already exists (never invent list_sessions on open).
 */
export async function quietRefreshConversationSlices(
  get: WorkspaceGet,
  conversationId: string,
): Promise<void> {
  await get().loadTimeline(conversationId, { quiet: true });
  const st = get();
  if (
    Object.prototype.hasOwnProperty.call(
      st.sessionsByConversation,
      conversationId,
    ) ||
    Object.prototype.hasOwnProperty.call(
      st.inspectorStatusByConversation,
      conversationId,
    )
  ) {
    await get().loadInspector(conversationId, { quiet: true });
  }
}
