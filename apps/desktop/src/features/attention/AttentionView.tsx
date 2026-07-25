import { useEffect } from "react";
import { ShieldAlert, XCircle, PauseCircle } from "lucide-react";
import { agentMeta } from "@/shared/lib/mock-data";
import { useUiStore } from "@/store/ui-store";
import { useWorkspaceStore } from "@/store/workspace-store";
import { cn } from "@/shared/lib/utils";

export function AttentionView() {
  const selectConversation = useUiStore((s) => s.selectConversation);
  const selectProject = useUiStore((s) => s.selectProject);
  const selectSession = useUiStore((s) => s.selectSession);
  const openSessionTranscript = useUiStore((s) => s.openSessionTranscript);
  const source = useWorkspaceStore((s) => s.source);
  const items = useWorkspaceStore((s) => s.attentionSessions);
  const status = useWorkspaceStore((s) => s.attentionStatus);
  const loadAttentionSessions = useWorkspaceStore(
    (s) => s.loadAttentionSessions,
  );
  const projects = useWorkspaceStore((s) => s.projects);
  const conversations = useWorkspaceStore((s) => s.conversations);

  useEffect(() => {
    if (source !== "daemon") return;
    void loadAttentionSessions();
  }, [source, loadAttentionSessions]);

  const phase = status.phase;

  return (
    <div className="flex min-h-0 flex-1 flex-col bg-surface">
      <header className="border-b border-ink/5 px-6 py-5">
        <h1 className="text-xl font-semibold tracking-tight text-ink">
          Attention
        </h1>
        <p className="mt-1 text-sm text-ink-muted">
          Approvals, failures, and suspended agent sessions that need follow-up.
        </p>
      </header>
      <div className="scrollbar-thin flex-1 space-y-2 overflow-y-auto p-4">
        {phase === "loading" && items.length === 0 ? (
          <p className="py-12 text-center text-sm text-ink-muted">
            Scanning sessions…
          </p>
        ) : null}
        {phase === "error" && items.length === 0 ? (
          <div className="flex flex-col items-center gap-3 py-12 text-center">
            <p className="text-sm text-rose-600">
              {status.error ?? "Failed to load attention queue"}
            </p>
            <button
              type="button"
              onClick={() => void loadAttentionSessions()}
              className="rounded-lg bg-ink px-3 py-1.5 text-xs font-semibold text-surface"
            >
              Retry
            </button>
          </div>
        ) : null}
        {phase === "ready" && items.length === 0 ? (
          <p className="py-12 text-center text-sm text-ink-muted">
            Nothing needs attention right now.
          </p>
        ) : null}
        {items.map((session) => {
          const conv = conversations.find((c) => c.id === session.conversationId);
          const project = projects.find((p) => p.id === conv?.projectId);
          const meta = agentMeta[session.agent as keyof typeof agentMeta];
          const isApproval = session.status === "needs_approval";
          const isFailed = session.status === "failed";
          return (
            <div
              key={session.id}
              className="rounded-2xl border border-ink/5 bg-surface-raised p-4 shadow-sm"
            >
              <div className="flex items-start gap-3">
                <div
                  className={cn(
                    "flex h-9 w-9 items-center justify-center rounded-xl",
                    isApproval
                      ? "bg-rose-100 text-rose-700"
                      : isFailed
                        ? "bg-red-100 text-red-700"
                        : "bg-status-suspended/15 text-status-suspended",
                  )}
                >
                  {isApproval ? (
                    <ShieldAlert className="h-4 w-4" />
                  ) : isFailed ? (
                    <XCircle className="h-4 w-4" />
                  ) : (
                    <PauseCircle className="h-4 w-4" />
                  )}
                </div>
                <div className="min-w-0 flex-1">
                  <div className="flex flex-wrap items-center gap-2">
                    <span className="text-sm font-semibold text-ink">
                      {isApproval
                        ? "Approval required"
                        : isFailed
                          ? "Session failed"
                          : "Session paused"}
                    </span>
                    <span
                      className={cn(
                        "rounded-md px-1.5 py-0.5 text-2xs font-medium",
                        meta?.color ?? "bg-ink/10 text-ink-secondary",
                      )}
                    >
                      {meta?.label ?? session.agent} #{session.shortId}
                    </span>
                  </div>
                  <p className="mt-1 text-sm text-ink-secondary">
                    {session.summary}
                  </p>
                  <p className="mt-1 text-xs text-ink-muted">
                    {project?.name ?? "—"} / {conv?.title ?? session.conversationTitle ?? "—"}
                  </p>
                  <div className="mt-3 flex flex-wrap gap-2">
                    {conv && project ? (
                      <button
                        type="button"
                        onClick={() => {
                          selectProject(project.id);
                          selectConversation(conv.id);
                        }}
                        className="rounded-lg border border-ink/10 bg-surface-raised px-3 py-1.5 text-xs font-medium text-ink hover:bg-surface-muted"
                      >
                        Open conversation
                      </button>
                    ) : null}
                    <button
                      type="button"
                      onClick={() => {
                        if (project) selectProject(project.id);
                        selectSession(session.id);
                        openSessionTranscript(
                          session.id,
                          session.conversationId,
                        );
                      }}
                      className="rounded-lg bg-ink px-3 py-1.5 text-xs font-semibold text-surface hover:opacity-90"
                    >
                      {isApproval ? "Review / approve" : "Open transcript"}
                    </button>
                  </div>
                </div>
              </div>
            </div>
          );
        })}
      </div>
    </div>
  );
}
