import type { ReactNode } from "react";
import { ChevronRight, FolderOpen, X } from "lucide-react";
import { agentMeta, type AgentSession } from "@/lib/mock-data";
import { Avatar } from "@/components/Avatar";
import { StatusPill } from "@/components/StatusPill";
import { useUiStore } from "@/store/ui-store";
import { useWorkspaceStore } from "@/store/workspace-store";
import { cn } from "@/lib/utils";

export function SessionInspector({
  conversationId,
}: {
  conversationId: string;
}) {
  const toggleDetails = useUiStore((s) => s.toggleDetails);
  const projectId = useUiStore((s) => s.projectId);
  const selectedSessionId = useUiStore((s) => s.selectedSessionId);
  const selectSession = useUiStore((s) => s.selectSession);
  const conversations = useWorkspaceStore((s) => s.conversations);
  const projects = useWorkspaceStore((s) => s.projects);
  const sessionsByConversation = useWorkspaceStore(
    (s) => s.sessionsByConversation,
  );
  const detailPhase = useWorkspaceStore(
    (s) => s.detailStatusByConversation[conversationId]?.phase,
  );

  const conversation = conversations.find((c) => c.id === conversationId);
  const project = projects.find((p) => p.id === projectId);
  const sessions = sessionsByConversation[conversationId] ?? [];
  const roots = sessions.filter((s) => !s.parentId);
  const selected = sessions.find((s) => s.id === selectedSessionId);

  return (
    <aside className="flex w-[min(280px,30vw)] min-w-[220px] max-w-[340px] shrink-0 flex-col overflow-hidden border-l border-ink/5 bg-surface">
      <header className="flex shrink-0 items-center justify-between border-b border-ink/5 px-4 py-3">
        <div className="min-w-0 truncate text-[13px] font-semibold text-ink">
          {selected ? "Agent session" : "Conversation"}
        </div>
        <button
          type="button"
          onClick={toggleDetails}
          className="rounded-md p-1 text-ink-muted hover:bg-surface-hover"
        >
          <X className="h-4 w-4" />
        </button>
      </header>

      <div className="scrollbar-thin min-h-0 flex-1 space-y-5 overflow-y-auto px-4 py-4 text-[12.5px]">
        {selected ? (
          <SessionDetail
            session={selected}
            onBack={() => selectSession(null)}
          />
        ) : (
          <>
            <section>
              <Label>Title</Label>
              <div
                className="mt-1 truncate font-medium text-ink"
                title={conversation?.title}
              >
                {conversation?.title}
              </div>
            </section>

            <section>
              <Label>Workspace</Label>
              <div className="mt-1 flex min-w-0 items-start gap-2 rounded-lg bg-surface-muted px-2.5 py-2 font-mono text-[11px] text-ink-secondary">
                <FolderOpen className="mt-0.5 h-3.5 w-3.5 shrink-0" />
                <span
                  className="min-w-0 break-all"
                  title={project?.workspacePath}
                >
                  {project?.workspacePath}
                </span>
              </div>
            </section>

            <section>
              <div className="mb-2 flex items-center justify-between">
                <Label>Agent sessions</Label>
                <span className="text-[11px] text-ink-muted">
                  {sessions.length}
                </span>
              </div>
              {roots.length === 0 ? (
                <p className="text-ink-muted">
                  {detailPhase === "loading"
                    ? "Loading sessions…"
                    : "No sessions yet. Use @agent in the input to start one."}
                </p>
              ) : (
                <div className="space-y-1">
                  {roots.map((session) => (
                    <SessionTree
                      key={session.id}
                      session={session}
                      all={sessions}
                      depth={0}
                      selectedId={selectedSessionId}
                      onSelect={selectSession}
                    />
                  ))}
                </div>
              )}
            </section>

            <section>
              <Label>Quick start</Label>
              <p className="mt-1 text-[11px] text-ink-muted">
                Type in the composer, e.g.{" "}
                <span className="font-mono">@grok hello</span>
              </p>
              <div className="mt-2 flex flex-wrap gap-1.5">
                {(["codex", "claude", "gemini", "grok"] as const).map((a) => (
                  <span
                    key={a}
                    className={cn(
                      "rounded-md px-2 py-1 text-[11px] font-medium",
                      agentMeta[a].color,
                    )}
                  >
                    @{a}
                  </span>
                ))}
              </div>
            </section>
          </>
        )}
      </div>
    </aside>
  );
}

function SessionTree({
  session,
  all,
  depth,
  selectedId,
  onSelect,
}: {
  session: AgentSession;
  all: AgentSession[];
  depth: number;
  selectedId: string | null;
  onSelect: (id: string) => void;
}) {
  const children = all.filter((s) => s.parentId === session.id);
  const meta = agentMeta[session.agent as keyof typeof agentMeta];
  const selected = selectedId === session.id;
  const label = meta?.label ?? session.agent ?? "Agent";
  const tone = meta?.tone ?? "slate";

  return (
    <div>
      <button
        type="button"
        onClick={() => onSelect(session.id)}
        style={{ paddingLeft: 8 + depth * 12 }}
        className={cn(
          "flex w-full items-center gap-2 rounded-lg py-2 pr-2 text-left transition-colors",
          selected ? "bg-accent-soft" : "hover:bg-surface-hover",
        )}
      >
        <Avatar name={label} tone={tone} size="sm" />
        <div className="min-w-0 flex-1">
          <div className="flex items-center gap-1.5">
            <span className="truncate text-[12px] font-semibold text-ink">
              {label}
            </span>
            <span className="font-mono text-[10px] text-ink-muted">
              #{session.shortId}
            </span>
          </div>
          <div className="truncate text-[11px] text-ink-muted">
            {session.summary}
          </div>
        </div>
        <StatusPill status={session.status} className="shrink-0 scale-90" />
      </button>
      {children.map((child) => (
        <SessionTree
          key={child.id}
          session={child}
          all={all}
          depth={depth + 1}
          selectedId={selectedId}
          onSelect={onSelect}
        />
      ))}
    </div>
  );
}

function SessionDetail({
  session,
  onBack,
}: {
  session: AgentSession;
  onBack: () => void;
}) {
  const openSessionTranscript = useUiStore((s) => s.openSessionTranscript);
  const meta = agentMeta[session.agent as keyof typeof agentMeta];
  const label = meta?.label ?? session.agent ?? "Agent";
  const tone = meta?.tone ?? "slate";
  return (
    <div className="space-y-4">
      <button
        type="button"
        onClick={onBack}
        className="text-[12px] font-medium text-ink-muted hover:text-ink"
      >
        ← Back to conversation
      </button>
      <div className="flex items-center gap-3">
        <Avatar name={label} tone={tone} size="lg" />
        <div>
          <div className="text-[14px] font-semibold text-ink">
            {label}{" "}
            <span className="font-mono text-[12px] font-normal text-ink-muted">
              #{session.shortId}
            </span>
          </div>
          <StatusPill status={session.status} className="mt-1" />
        </div>
      </div>
      <section>
        <Label>Model</Label>
        <div className="mt-1 font-medium text-ink">{session.model}</div>
      </section>
      <section>
        <Label>Summary</Label>
        <div className="mt-1 text-ink-secondary">{session.summary}</div>
      </section>
      {session.lastTool ? (
        <section>
          <Label>Last tool</Label>
          <div className="mt-1 rounded-lg bg-surface-muted px-2.5 py-2 font-mono text-[11px] text-ink-secondary">
            {session.lastTool}
          </div>
        </section>
      ) : null}
      {session.parentId ? (
        <p className="text-[11px] text-ink-muted">
          Subagent — open parent session for the main run.
        </p>
      ) : null}
      <button
        type="button"
        onClick={() =>
          openSessionTranscript(session.id, session.conversationId)
        }
        className="flex w-full items-center justify-between rounded-xl border border-ink/10 bg-white px-3 py-2.5 text-left text-[12px] font-medium text-ink hover:bg-surface-muted"
      >
        Open full transcript
        <ChevronRight className="h-4 w-4 text-ink-muted" />
      </button>
    </div>
  );
}

function Label({ children }: { children: ReactNode }) {
  return (
    <div className="text-[11px] font-semibold uppercase tracking-[0.06em] text-ink-muted">
      {children}
    </div>
  );
}
