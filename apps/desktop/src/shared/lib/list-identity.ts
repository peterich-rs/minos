/**
 * Keep array / element object identity across quiet reloads so React.memo rows
 * skip re-render when wire content is unchanged.
 */

/** Reuse previous row objects (by id) when `equal` says content matches. */
export function reuseStableById<T extends { id: string }>(
  prev: readonly T[] | undefined,
  next: readonly T[],
  equal: (a: T, b: T) => boolean,
): T[] {
  if (!prev) return next.slice();
  if (next.length === 0) return prev.length === 0 ? (prev as T[]) : [];
  if (prev.length === 0) return next.slice();

  const prevById = new Map(prev.map((x) => [x.id, x]));
  let changed = prev.length !== next.length;
  const out: T[] = new Array(next.length);

  for (let i = 0; i < next.length; i++) {
    const n = next[i]!;
    const p = prevById.get(n.id);
    if (p && equal(p, n)) {
      out[i] = p;
      if (prev[i] !== p) changed = true;
    } else {
      out[i] = n;
      changed = true;
    }
  }

  return changed ? out : (prev as T[]);
}

/** Conversation rail fields that affect ConversationRow render / sort. */
export function conversationEqual(
  a: {
    id: string;
    projectId?: string;
    title?: string;
    preview?: string;
    updatedAtMs?: number;
    messageCount?: number;
    unread?: number;
    agentSessionCount?: number;
    participatingAgents?: readonly string[];
    runningCount?: number;
    approvalCount?: number;
    priority?: string;
    progress?: string;
    boardColumn?: string;
    branch?: string;
    worktree?: string;
    gitMode?: string;
    gitDirty?: boolean;
    gitHead?: string;
  },
  b: {
    id: string;
    projectId?: string;
    title?: string;
    preview?: string;
    updatedAtMs?: number;
    messageCount?: number;
    unread?: number;
    agentSessionCount?: number;
    participatingAgents?: readonly string[];
    runningCount?: number;
    approvalCount?: number;
    priority?: string;
    progress?: string;
    boardColumn?: string;
    branch?: string;
    worktree?: string;
    gitMode?: string;
    gitDirty?: boolean;
    gitHead?: string;
  },
): boolean {
  if (
    a.id !== b.id ||
    a.projectId !== b.projectId ||
    a.title !== b.title ||
    a.preview !== b.preview ||
    a.updatedAtMs !== b.updatedAtMs ||
    a.messageCount !== b.messageCount ||
    a.unread !== b.unread ||
    a.agentSessionCount !== b.agentSessionCount ||
    a.runningCount !== b.runningCount ||
    a.approvalCount !== b.approvalCount ||
    a.priority !== b.priority ||
    a.progress !== b.progress ||
    a.boardColumn !== b.boardColumn ||
    a.branch !== b.branch ||
    a.worktree !== b.worktree ||
    a.gitMode !== b.gitMode ||
    a.gitDirty !== b.gitDirty ||
    a.gitHead !== b.gitHead
  ) {
    return false;
  }
  const aa = a.participatingAgents ?? [];
  const bb = b.participatingAgents ?? [];
  if (aa.length !== bb.length) return false;
  for (let i = 0; i < aa.length; i++) {
    if (aa[i] !== bb[i]) return false;
  }
  return true;
}

/**
 * Quiet conversation re-list: reuse previous Conversation object identity when
 * content is unchanged so VirtualizedList / memo rows do not remount.
 */
export function reuseStableConversations<
  T extends {
    id: string;
    projectId?: string;
    title?: string;
    preview?: string;
    updatedAtMs?: number;
    messageCount?: number;
    unread?: number;
    agentSessionCount?: number;
    participatingAgents?: readonly string[];
    runningCount?: number;
    approvalCount?: number;
    priority?: string;
    progress?: string;
    boardColumn?: string;
    branch?: string;
    worktree?: string;
    gitMode?: string;
    gitDirty?: boolean;
    gitHead?: string;
  },
>(prev: readonly T[] | undefined, next: readonly T[]): T[] {
  return reuseStableById(prev, next as T[], conversationEqual);
}

/** Timeline message fields that affect row render / order. */
export function timelineMessageEqual(
  a: {
    id: string;
    messageSeq?: number;
    role: string;
    agent?: string;
    sessionId?: string;
    body: string;
    time: string;
    createdAtMs?: number;
    kind?: string;
    replyToMessageId?: string;
    delegationId?: string;
    deliveryStatus?: string;
  },
  b: {
    id: string;
    messageSeq?: number;
    role: string;
    agent?: string;
    sessionId?: string;
    body: string;
    time: string;
    createdAtMs?: number;
    kind?: string;
    replyToMessageId?: string;
    delegationId?: string;
    deliveryStatus?: string;
  },
): boolean {
  return (
    a.id === b.id &&
    a.messageSeq === b.messageSeq &&
    a.role === b.role &&
    a.agent === b.agent &&
    a.sessionId === b.sessionId &&
    a.body === b.body &&
    a.time === b.time &&
    a.createdAtMs === b.createdAtMs &&
    a.kind === b.kind &&
    a.replyToMessageId === b.replyToMessageId &&
    a.delegationId === b.delegationId &&
    a.deliveryStatus === b.deliveryStatus
  );
}

/** Transcript row fields that affect TranscriptItemView render. */
export function transcriptItemEqual(
  a: {
    id: string;
    kind: string;
    role: string | null;
    text: string;
    detail?: string | null;
    title?: string | null;
    tsMs: number;
    seq: number;
    messageId?: string | null;
    requestId?: string | null;
    approvalMethod?: string | null;
    options?: unknown;
    approveResponse?: string | null;
    declineResponse?: string | null;
  },
  b: {
    id: string;
    kind: string;
    role: string | null;
    text: string;
    detail?: string | null;
    title?: string | null;
    tsMs: number;
    seq: number;
    messageId?: string | null;
    requestId?: string | null;
    approvalMethod?: string | null;
    options?: unknown;
    approveResponse?: string | null;
    declineResponse?: string | null;
  },
): boolean {
  return (
    a.id === b.id &&
    a.kind === b.kind &&
    a.role === b.role &&
    a.text === b.text &&
    a.detail === b.detail &&
    a.title === b.title &&
    a.tsMs === b.tsMs &&
    a.seq === b.seq &&
    a.messageId === b.messageId &&
    a.requestId === b.requestId &&
    a.approvalMethod === b.approvalMethod &&
    a.approveResponse === b.approveResponse &&
    a.declineResponse === b.declineResponse &&
    optionsEqual(a.options, b.options)
  );
}

function optionsEqual(a: unknown, b: unknown): boolean {
  if (a === b) return true;
  if (a == null || b == null) return a === b;
  // Options are small; JSON compare is enough for identity reuse.
  try {
    return JSON.stringify(a) === JSON.stringify(b);
  } catch {
    return false;
  }
}
