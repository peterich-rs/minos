import { useEffect } from "react";
import { AlertTriangle } from "lucide-react";
import { agentMeta } from "@/shared/lib/mock-data";
import {
  AttentionListCard,
  AttentionPrimaryButton,
  AttentionSecondaryButton,
} from "@/shared/ui/AttentionChrome";
import {
  PageHeader,
  PageHeaderPrimaryButton,
} from "@/shared/ui/PageHeader";
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
    <div className="flex min-h-0 flex-1 flex-col bg-canvas-soft/40">
      <PageHeader
        title={
          <span className="inline-flex items-center gap-2">
            <AlertTriangle className="h-6 w-6 text-status-approval" />
            Attention
          </span>
        }
        description="Approvals, failures, and suspended agent sessions that need follow-up."
        badge={
          items.length > 0 ? (
            <span className="rounded-full bg-status-approval/15 px-2 py-0.5 text-2xs font-semibold tabular-nums text-status-approval">
              {items.length}
            </span>
          ) : null
        }
      />
      <div className="scrollbar-thin flex-1 space-y-3 overflow-y-auto p-5 sm:p-6">
        {phase === "loading" && items.length === 0 ? (
          <p className="py-12 text-center text-sm text-ink-muted">
            Scanning sessions…
          </p>
        ) : null}
        {phase === "error" && items.length === 0 ? (
          <div className="flex flex-col items-center gap-3 py-12 text-center">
            <p className="text-sm text-status-failed">
              {status.error ?? "Failed to load attention queue"}
            </p>
            <PageHeaderPrimaryButton
              onClick={() => void loadAttentionSessions()}
            >
              Retry
            </PageHeaderPrimaryButton>
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
          const tone = isApproval
            ? "approval"
            : isFailed
              ? "failed"
              : "suspended";
          return (
            <AttentionListCard
              key={session.id}
              tone={tone}
              title={
                isApproval
                  ? "Approval required"
                  : isFailed
                    ? "Session failed"
                    : "Session paused"
              }
              badge={
                <span
                  className={cn(
                    "rounded-md px-1.5 py-0.5 text-2xs font-medium",
                    meta?.color ?? "bg-ink/10 text-ink-secondary",
                  )}
                >
                  {meta?.label ?? session.agent} #{session.shortId}
                </span>
              }
              body={session.summary}
              meta={`${project?.name ?? "—"} / ${conv?.title ?? session.conversationTitle ?? "—"}`}
              actions={
                <>
                  {conv && project ? (
                    <AttentionSecondaryButton
                      onClick={() => {
                        selectProject(project.id);
                        selectConversation(conv.id);
                      }}
                    >
                      Open conversation
                    </AttentionSecondaryButton>
                  ) : null}
                  <AttentionPrimaryButton
                    onClick={() => {
                      if (project) selectProject(project.id);
                      selectSession(session.id);
                      openSessionTranscript(
                        session.id,
                        session.conversationId,
                      );
                    }}
                  >
                    {isApproval ? "Review / approve" : "Open transcript"}
                  </AttentionPrimaryButton>
                </>
              }
            />
          );
        })}
      </div>
    </div>
  );
}
