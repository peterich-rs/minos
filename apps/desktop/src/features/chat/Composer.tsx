import { useEffect, useMemo, useRef, useState } from "react";
import { AtSign, Bold, Paperclip, X } from "lucide-react";
import {
  buildAgentMentionOptions,
  buildHumanMentionOptions,
  mentionQueryAtCursor,
  type MentionHuman,
  type MentionProfile,
} from "@/shared/lib/agent-route";
import type { TimelineMessage } from "@/shared/lib/mock-data";
import { hasPrimaryShortcutModifier } from "@/shared/lib/platform";
import { cn } from "@/shared/lib/utils";
import { toast } from "@/shared/lib/toast";
import { daemonApi, isTauriRuntime } from "@/shared/lib/daemon";
import { listConversationParticipants } from "@/shared/lib/minos-cloud";
import {
  ComposerChrome,
  ComposerToolBtn,
} from "@/shared/ui/ComposerChrome";
import { useAccountStore } from "@/store/account-store";
import { useUiStore } from "@/store/ui-store";
import {
  useWorkspaceStore,
  type ProjectSession,
} from "@/store/workspace-store";
import { EmojiPicker } from "./EmojiPicker";
import { replyAuthorLabel, replyPreviewBody } from "./lib/format";

const EMPTY_SESSIONS: ProjectSession[] = [];
const EMPTY_MESSAGES: TimelineMessage[] = [];
const EMPTY_PROFILES: MentionProfile[] = [];
const EMPTY_HUMANS: MentionHuman[] = [];

export function Composer({ conversationId }: { conversationId: string }) {
  const draft = useUiStore(
    (s) => s.draftByConversationId[conversationId] ?? "",
  );
  const setDraftGlobal = useUiStore((s) => s.setDraft);
  const setDraft = (value: string) => setDraftGlobal(conversationId, value);
  const replyToMessageId = useUiStore(
    (s) => s.replyToMessageIdByConversation[conversationId] ?? null,
  );
  const clearReplyTo = useUiStore((s) => s.clearReplyTo);
  const messages = useWorkspaceStore(
    (s) => s.messagesByConversation[conversationId] ?? EMPTY_MESSAGES,
  );
  const replyParent = useMemo(
    () =>
      replyToMessageId
        ? messages.find((m) => m.id === replyToMessageId)
        : undefined,
    [messages, replyToMessageId],
  );

  const sendMessage = useWorkspaceStore((s) => s.sendMessage);
  const loadInspector = useWorkspaceStore((s) => s.loadInspector);
  const source = useWorkspaceStore((s) => s.source);
  const clis = useWorkspaceStore((s) => s.clis);
  const participatingAgents = useWorkspaceStore(
    (s) =>
      s.conversations.find((c) => c.id === conversationId)
        ?.participatingAgents ?? [],
  );
  const sessions = useWorkspaceStore(
    (s) => s.sessionsByConversation[conversationId] ?? EMPTY_SESSIONS,
  );
  const timelineStatus = useWorkspaceStore(
    (s) => s.timelineStatusByConversation[conversationId],
  );
  const hasCachedMessages = useWorkspaceStore(
    (s) => conversationId in s.messagesByConversation,
  );
  const accountSyncStatus = useAccountStore((s) => s.accountSyncStatus);
  const session = useAccountStore((s) => s.session);
  const deviceId = useAccountStore((s) => s.deviceId);

  const phase = timelineStatus?.phase ?? "idle";
  const detailError = timelineStatus?.error;
  // Account-primary send gate: when signed in, only fully online can send.
  // connecting/unknown/offline must not present send-ready chat (ADR 0021 §6).
  const accountLinked = Boolean(session?.accessToken?.trim());
  const accountSendReady = !accountLinked || accountSyncStatus === "online";
  const accountBlocked = accountLinked && accountSyncStatus !== "online";

  const [sending, setSending] = useState(false);
  const [sendError, setSendError] = useState<string | null>(null);
  const [cursor, setCursor] = useState(0);
  const [mentionIndex, setMentionIndex] = useState(0);
  const [mentionProfiles, setMentionProfiles] =
    useState<MentionProfile[]>(EMPTY_PROFILES);
  const [mentionHumans, setMentionHumans] =
    useState<MentionHuman[]>(EMPTY_HUMANS);
  const [participantAgents, setParticipantAgents] = useState<string[] | null>(
    null,
  );
  const textareaRef = useRef<HTMLTextAreaElement>(null);

  const mention = mentionQueryAtCursor(draft, cursor);
  const memberAgents = participantAgents ?? participatingAgents;
  const mentionOptions = useMemo(() => {
    if (!mention) return [];
    const humans = buildHumanMentionOptions(mention.query, mentionHumans, {
      selfAccountId: session?.accountId,
      limit: 8,
    });
    // Membership-gated bots: participants API agents preferred, else roster.
    const agents = buildAgentMentionOptions({
      query: mention.query,
      clis,
      sessions,
      profiles: mentionProfiles,
      memberAgents,
    });
    return [...humans, ...agents];
  }, [
    mention,
    mentionHumans,
    clis,
    sessions,
    mentionProfiles,
    memberAgents,
    session?.accountId,
  ]);

  // When @-mention UI opens and sessions are empty, ensure Inspector working set
  // so @agent#short options can list existing sessions without opening the rail.
  const mentionActive = mention != null;
  useEffect(() => {
    if (source !== "daemon" || !mentionActive) return;
    if (sessions.length > 0) return;
    const hasKey =
      conversationId in useWorkspaceStore.getState().sessionsByConversation;
    void loadInspector(conversationId, { quiet: hasKey });
  }, [mentionActive, sessions.length, conversationId, source, loadInspector]);

  // Unified participants (humans ∪ agents) for @ picker — ADR 0021 / global-bot-identity.
  // @ targets = conversation roster only (Hub participants preferred). Never mix
  // unjoined Host profiles into collab send targets.
  useEffect(() => {
    if (!mentionActive) return;
    const token = session?.accessToken?.trim();
    let cancelled = false;
    void (async () => {
      // Prefer Hub participants when account is online.
      if (token && conversationId) {
        try {
          const parts = await listConversationParticipants(
            deviceId,
            token,
            conversationId,
          );
          if (cancelled) return;
          setMentionHumans(
            parts.humans.map((h) => ({
              accountId: h.accountId,
              minosId: h.minosId,
              displayName: h.displayName,
            })),
          );
          // Membership tokens: runtime + bot name + agent_id (all roster-scoped).
          // Disabled bots stay on participants for UI, but are not @-targets.
          const memberTokens = new Set<string>();
          const rosterProfiles: MentionProfile[] = [];
          for (const a of parts.agents) {
            if ((a.status || "active").toLowerCase() === "disabled") continue;
            const runtime = a.runtimeAgent.trim().toLowerCase();
            if (runtime) memberTokens.add(runtime);
            const name = (a.displayName || a.name).trim();
            if (name) memberTokens.add(name.toLowerCase());
            if (a.agentId) memberTokens.add(a.agentId.toLowerCase());
            rosterProfiles.push({
              id: a.agentId,
              name: name || a.agentId,
              runtimeAgent: a.runtimeAgent,
            });
          }
          setParticipantAgents([...memberTokens]);
          setMentionProfiles(rosterProfiles);
          return;
        } catch {
          if (cancelled) return;
          // Fall through to offline roster.
        }
      }

      // Offline / no Hub: gate by local conversation roster only (no full profile dir).
      if (source === "daemon" && isTauriRuntime()) {
        try {
          const res = await daemonApi.listAgentProfiles();
          if (cancelled) return;
          const memberSet = new Set(
            participatingAgents.map((a) => a.trim().toLowerCase()).filter(Boolean),
          );
          const rosterOnly = (res.profiles ?? [])
            .filter((p) => memberSet.has(p.runtime_agent.trim().toLowerCase()))
            .map((p) => ({
              id: p.id,
              name: p.name,
              runtimeAgent: p.runtime_agent,
            }));
          setMentionProfiles(rosterOnly);
          setParticipantAgents(participatingAgents.map((a) => a.toLowerCase()));
          setMentionHumans(EMPTY_HUMANS);
          return;
        } catch {
          /* fall through */
        }
      }
      if (!cancelled) {
        setMentionHumans(EMPTY_HUMANS);
        setMentionProfiles(EMPTY_PROFILES);
        setParticipantAgents(null);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [
    mentionActive,
    conversationId,
    deviceId,
    session?.accessToken,
    source,
    participatingAgents,
  ]);

  useEffect(() => {
    setMentionIndex(0);
  }, [mention?.query, mentionOptions.length]);

  // Focus composer when user picks Reply on a message.
  useEffect(() => {
    if (!replyToMessageId) return;
    requestAnimationFrame(() => {
      textareaRef.current?.focus();
    });
  }, [replyToMessageId]);

  const applyMention = (insert: string) => {
    if (!mention) return;
    const next = draft.slice(0, mention.start) + insert + draft.slice(cursor);
    setDraft(next);
    const nextCursor = mention.start + insert.length;
    setCursor(nextCursor);
    requestAnimationFrame(() => {
      const el = textareaRef.current;
      if (el) {
        el.focus();
        el.setSelectionRange(nextCursor, nextCursor);
      }
    });
  };

  const insertAtCursor = (text: string) => {
    const el = textareaRef.current;
    const pos = el?.selectionStart ?? draft.length;
    const next = draft.slice(0, pos) + text + draft.slice(pos);
    setDraft(next);
    const c = pos + text.length;
    setCursor(c);
    requestAnimationFrame(() => {
      el?.focus();
      el?.setSelectionRange(c, c);
    });
  };

  const onSend = async () => {
    const text = draft.trim();
    if (!text || !conversationId) return;
    if (accountBlocked) {
      const detail =
        accountSyncStatus === "connecting"
          ? "Account is still connecting — wait until Online to send."
          : accountSyncStatus === "unknown"
            ? "Account sync not ready — wait until Online to send."
            : "Account sync is disconnected — reconnect to send chat.";
      toast.error("Cannot send", detail);
      setSendError(detail);
      return;
    }
    // WeChat-style: empty the composer immediately. The message body is
    // already captured in `text`; the optimistic `sending` row (inserted by
    // sendMessage before any throwing step) carries it. On failure the row
    // becomes a failed bubble with a red `!`, so the draft is never refilled.
    const pendingReplyTo = replyToMessageId ?? undefined;
    setDraft("");
    setCursor(0);
    clearReplyTo(conversationId);
    setSending(true);
    setSendError(null);
    try {
      await sendMessage(conversationId, text, undefined, {
        replyToMessageId: pendingReplyTo,
      });
    } catch (e) {
      const msg = e instanceof Error ? e.message : String(e);
      setSendError(msg);
      toast.error("Failed to send message", msg);
    } finally {
      setSending(false);
    }
  };

  const accountHint =
    !accountLinked
      ? null
      : accountSyncStatus === "online"
        ? null
        : accountSyncStatus === "connecting"
          ? "Connecting… · cannot send until Account is Online"
          : accountSyncStatus === "unknown"
            ? "Account sync starting… · cannot send yet"
            : "Messages offline · cannot send until Account reconnects";

  const hint = (
    <>
      {accountHint
        ? accountHint
        : source === "daemon"
          ? "Connected · @member or @bot · ⌘/Ctrl+Enter send"
          : "Mock mode"}
      {phase === "loading" && hasCachedMessages ? " · refreshing…" : ""}
      {sendError || (phase === "error" && detailError) ? (
        <span className="mt-1 block text-xs text-status-failed">
          {sendError || detailError}
        </span>
      ) : null}
    </>
  );

  return (
    // Composer stays outside the scrollport — always visible at the bottom.
    <div className="relative shrink-0">
      {mention && mentionOptions.length > 0 ? (
        <div className="absolute bottom-full left-5 right-5 z-20 mb-2 max-h-52 overflow-y-auto rounded-xl border border-ink/10 bg-surface py-1 shadow-lg">
          <div className="px-3 py-1.5 text-2xs font-semibold uppercase tracking-wide text-ink-muted">
            Participants
          </div>
          {mentionOptions.map((opt, i) => (
            <button
              key={opt.id}
              type="button"
              disabled={opt.disabled}
              onMouseDown={(e) => {
                e.preventDefault();
                if (!opt.disabled) applyMention(opt.insert);
              }}
              className={cn(
                "flex w-full items-center justify-between px-3 py-2 text-left text-sm",
                i === mentionIndex
                  ? "bg-surface-muted"
                  : "hover:bg-surface-hover",
                opt.disabled && "opacity-40",
              )}
            >
              <span className="font-medium text-ink">{opt.label}</span>
              <span className="text-2xs text-ink-muted">{opt.hint}</span>
            </button>
          ))}
        </div>
      ) : null}

      {replyToMessageId ? (
        <div className="absolute bottom-full left-5 right-5 z-10 mb-2 flex items-start gap-2 rounded-xl border border-ink/10 bg-surface-muted/95 px-3 py-2 shadow-sm backdrop-blur-sm">
          <div className="min-w-0 flex-1">
            <div className="text-2xs font-semibold text-ink">
              Replying to{" "}
              {replyParent ? replyAuthorLabel(replyParent) : "message"}
            </div>
            <div className="mt-0.5 line-clamp-2 text-xs text-ink-secondary">
              {replyParent
                ? replyPreviewBody(replyParent.body)
                : `(message ${replyToMessageId})`}
            </div>
          </div>
          <button
            type="button"
            onClick={() => clearReplyTo(conversationId)}
            aria-label="Cancel reply"
            className="inline-flex h-7 w-7 shrink-0 items-center justify-center rounded-md text-ink-muted hover:bg-surface-hover hover:text-ink"
          >
            <X className="h-3.5 w-3.5" />
          </button>
        </div>
      ) : null}

      <ComposerChrome
        textareaProps={{
          ref: textareaRef,
          value: draft,
          onChange: (e) => {
            setDraft(e.target.value);
            setCursor(e.target.selectionStart);
          },
          onSelect: (e) =>
            setCursor((e.target as HTMLTextAreaElement).selectionStart),
          onClick: (e) =>
            setCursor((e.target as HTMLTextAreaElement).selectionStart),
          onKeyDown: (e) => {
            if (mention && mentionOptions.length > 0) {
              if (e.key === "ArrowDown") {
                e.preventDefault();
                setMentionIndex((i) =>
                  Math.min(i + 1, mentionOptions.length - 1),
                );
                return;
              }
              if (e.key === "ArrowUp") {
                e.preventDefault();
                setMentionIndex((i) => Math.max(i - 1, 0));
                return;
              }
              if (e.key === "Enter" || e.key === "Tab") {
                const opt = mentionOptions[mentionIndex];
                if (opt && !opt.disabled) {
                  e.preventDefault();
                  applyMention(opt.insert);
                  return;
                }
              }
              if (e.key === "Escape") {
                e.preventDefault();
                setCursor(mention.start);
                return;
              }
            }
            if (e.key === "Escape" && replyToMessageId) {
              e.preventDefault();
              clearReplyTo(conversationId);
              return;
            }
            if (e.key === "Enter" && hasPrimaryShortcutModifier(e)) {
              e.preventDefault();
              void onSend();
            }
          },
          rows: 3,
          placeholder: accountBlocked
            ? accountSyncStatus === "connecting" ||
              accountSyncStatus === "unknown"
              ? "Connecting… wait until Account is Online to send"
              : "Messages offline — reconnect Account to send"
            : "Message… type @ to mention a member or bot",
          disabled: accountBlocked,
        }}
        toolbarStart={
          <>
            <ComposerToolBtn
              title="@ mention"
              onClick={() => {
                insertAtCursor("@");
              }}
            >
              <AtSign className="h-3.5 w-3.5" />
            </ComposerToolBtn>
            <ComposerToolBtn title="Bold">
              <Bold className="h-3.5 w-3.5" />
            </ComposerToolBtn>
            <EmojiPicker
              onSelect={(emoji) => insertAtCursor(emoji)}
              showQuickStrip={false}
              side="top"
              align="start"
              ariaLabel="Insert emoji"
            />
            <ComposerToolBtn title="Attach">
              <Paperclip className="h-3.5 w-3.5" />
            </ComposerToolBtn>
          </>
        }
        sendLabel={sending ? "Sending…" : "Send"}
        sendDisabled={sending || !draft.trim() || !accountSendReady}
        onSend={() => void onSend()}
        hint={hint}
      />
    </div>
  );
}
