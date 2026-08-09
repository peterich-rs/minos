/**
 * Hub conversation digests — server-authoritative list SSOT (InboxSync).
 *
 * Read contract:
 * 1. First loadConversations in Hub IM mode: if empty → one POST query → hydrate
 * 2. Subsequent per-project loads: read cache only + merge with daemon rows
 * 3. Live Account* durables / mark-read: patchOne only
 * 4. SnapshotRequired on account topic / explicit refresh: invalidate then hydrate
 *
 * Focus semantics (C3/C4):
 * - focusedConversationId is set only by open/select (Timeline mount →
 *   markConversationRead) — never by loadTimeline (quiet or full).
 * - Focused live inbound: local unread stays 0; Hub mark-read is debounced
 *   (im-hub-bridge scheduleFocusedMarkRead, 400ms).
 */

export type HubConversationDigest = {
  conversationId: string;
  title: string;
  preview: string | null;
  lastMessageAtMs: number;
  unreadCount: number;
  unreadMentionCount: number;
  kind: string;
  memberCount: number;
};

type DigestDelta = Partial<
  Pick<
    HubConversationDigest,
    | "title"
    | "preview"
    | "lastMessageAtMs"
    | "unreadCount"
    | "unreadMentionCount"
  >
>;

let digestsById: Map<string, HubConversationDigest> = new Map();
let hydrated = false;

export const hubDigestCache = {
  /** Sole HTTP fill path. Replaces all digests. */
  hydrate(digests: readonly HubConversationDigest[]): void {
    digestsById = new Map(
      digests
        .filter((d) => d.conversationId.trim())
        .map((d) => [d.conversationId, { ...d }]),
    );
    hydrated = true;
  },

  /** Live Account* / mark-read writer after hydrate. */
  patchOne(conversationId: string, delta: DigestDelta): void {
    const id = conversationId.trim();
    if (!id) return;
    const prev = digestsById.get(id);
    if (!prev) {
      // Unknown conversation: insert minimal row so rail can show Hub-only.
      // Empty title (not "Conversation") so list merge can keep a real daemon title.
      digestsById.set(id, {
        conversationId: id,
        title: (delta.title ?? "").trim(),
        preview: delta.preview ?? null,
        lastMessageAtMs: delta.lastMessageAtMs ?? 0,
        unreadCount: delta.unreadCount ?? 0,
        unreadMentionCount: delta.unreadMentionCount ?? 0,
        kind: "group",
        memberCount: 0,
      });
      return;
    }
    // Do not let empty / placeholder titles wipe a real name already cached.
    const nextTitleRaw = delta.title;
    const nextTitle =
      typeof nextTitleRaw === "string" ? nextTitleRaw.trim() : undefined;
    const placeholder =
      !nextTitle ||
      nextTitle.toLowerCase() === "conversation" ||
      nextTitle.toLowerCase() === "group chat" ||
      nextTitle.toLowerCase() === "direct agent sessions";
    digestsById.set(id, {
      ...prev,
      ...delta,
      conversationId: id,
      title: placeholder ? prev.title : (nextTitle as string),
    });
  },

  invalidate(): void {
    digestsById = new Map();
    hydrated = false;
  },

  isHydrated(): boolean {
    return hydrated;
  },

  get(conversationId: string): HubConversationDigest | undefined {
    return digestsById.get(conversationId);
  },

  getAll(): HubConversationDigest[] {
    return [...digestsById.values()];
  },

  /** Test helper */
  _resetForTests(): void {
    digestsById = new Map();
    hydrated = false;
  },
};
