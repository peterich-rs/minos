/**
 * Conversation / project mutation use-cases (create, rename, priority, progress).
 */
import type { WorkspaceGet, WorkspaceSet, WorkspaceState } from "./types";
import {
  patchLocalConversation,
  toUiConversation,
  toUiProject,
  toUiProjects,
} from "./helpers";
import { quietHydrateAllConversationLists } from "./projection";
import { daemonApi } from "@/shared/lib/daemon";
import { minosQueryClient } from "@/shared/api/queryClient";
import { queryKeys } from "@/shared/api/queryKeys";
import {
  nextPriority,
  nextProgress,
  parsePriority,
  parseProgress,
  progressForBoardColumn,
} from "@/shared/lib/conversation-meta";
import type { Conversation, Project } from "@/shared/lib/mock-data";
import { syncConversationToCloud } from "@/shared/lib/im-cloud-sync";

export function createConversationMutationActions(
  set: WorkspaceSet,
  get: WorkspaceGet,
): Pick<
  WorkspaceState,
  | "createConversation"
  | "refreshConversationGitStatus"
  | "updateConversationTitle"
  | "cycleConversationPriority"
  | "cycleConversationProgress"
  | "setConversationProgress"
  | "moveConversationToBoardColumn"
  | "createProject"
> {
  return {
    createConversation: async (projectId, input) => {
      const title = input.title.trim();
      if (!title) {
        throw new Error("title cannot be empty");
      }
      const priority = input.priority ?? null;
      const gitMode =
        input.gitMode === "inherit" || input.gitMode === "worktree"
          ? input.gitMode
          : "worktree";
      // Membership roster — who may be @mentioned / started. No eager session start.
      const agents: Array<{ agent: string; brief?: string }> = [];
      const seen = new Set<string>();
      for (const spec of input.agents ?? []) {
        const agent = (spec.agent ?? "").trim().toLowerCase();
        if (!agent || seen.has(agent)) continue;
        seen.add(agent);
        const brief = spec.brief?.trim() || undefined;
        agents.push(brief ? { agent, brief } : { agent });
      }

      if (get().source !== "daemon") {
        // Mock: append a local conversation for browser-only preview.
        const id = `mock-conv-${Date.now()}`;
        const conv: Conversation = {
          id,
          projectId,
          title,
          preview: "No messages yet",
          updatedAt: "now",
          updatedAtMs: Date.now(),
          messageCount: 0,
          boardColumn: "backlog",
          agentSessionCount: 0,
          participatingAgents: agents.map((a) => a.agent),
          runningCount: 0,
          approvalCount: 0,
          progress: "todo",
          priority: priority ?? undefined,
          gitMode,
          gitDirty: false,
          branch: gitMode === "worktree" ? "minos/mock-branch" : "main",
          worktree:
            gitMode === "worktree" ? "/tmp/.minos-worktrees/mock" : undefined,
        };
        set((s) => ({ conversations: [conv, ...s.conversations] }));
        return id;
      }

      const created = await daemonApi.createConversation(projectId, title, {
        priority: priority ?? null,
        agents,
        gitMode,
      });
      set({ actionError: null });

      // Multi-end IM: project shell + agent roster (local runtimes → cloud ids).
      void syncConversationToCloud({
        conversationId: created.id,
        title: created.title || title,
        agentRuntimes: agents.map((a) => a.agent),
      });

      await minosQueryClient.invalidateQueries({
        queryKey: queryKeys.conversations(projectId),
      });
      await get().loadConversations(projectId);
      return created.id;
    },

    refreshConversationGitStatus: async (conversationId) => {
      if (get().source !== "daemon" || !conversationId) return;
      try {
        const status = await daemonApi.gitGetStatus(conversationId, {
          refreshConversation: true,
        });
        const fromDaemon = status.conversation;
        set((s) => ({
          conversations: patchLocalConversation(
            s.conversations,
            conversationId,
            {
              branch:
                fromDaemon?.branch ?? status.branch ?? undefined,
              worktree:
                fromDaemon?.worktree ??
                (status.isLinkedWorktree
                  ? status.path
                  : s.conversations.find((c) => c.id === conversationId)
                      ?.worktree),
              gitMode: fromDaemon?.gitMode ?? undefined,
              gitDirty: fromDaemon?.gitDirty ?? status.dirty,
              gitHead:
                fromDaemon?.gitHead ??
                status.shortHead ??
                status.head ??
                undefined,
            },
          ),
        }));
      } catch {
        // Best-effort: non-git workspaces or offline git should not block open.
      }
    },

    updateConversationTitle: async (conversationId, title) => {
      const trimmed = title.trim();
      if (!trimmed) {
        throw new Error("title cannot be empty");
      }
      if (get().source !== "daemon") {
        set((s) => ({
          conversations: patchLocalConversation(
            s.conversations,
            conversationId,
            {
              title: trimmed,
            },
          ),
        }));
        return;
      }
      const updated = await daemonApi.updateConversation(conversationId, {
        title: trimmed,
      });
      void syncConversationToCloud({
        conversationId,
        title: trimmed,
        agentRuntimes: get().conversations.find((c) => c.id === conversationId)
          ?.participatingAgents,
      });
      set((s) => ({
        conversations: patchLocalConversation(s.conversations, conversationId, {
          ...toUiConversation(
            updated,
            s.readMessageCountById,
            s.focusedConversationId,
          ),
          // Preserve local attention fields that RPC may not recompute yet.
          unread: s.conversations.find((c) => c.id === conversationId)?.unread,
        }),
      }));
    },

    cycleConversationPriority: async (conversationId) => {
      const current = get().conversations.find((c) => c.id === conversationId);
      if (!current) return;
      const next = nextPriority(current.priority);
      const priorityValue = next ?? "";
      if (get().source !== "daemon") {
        set((s) => ({
          conversations: patchLocalConversation(
            s.conversations,
            conversationId,
            {
              priority: next ?? undefined,
            },
          ),
        }));
        return;
      }
      const updated = await daemonApi.updateConversation(conversationId, {
        priority: priorityValue,
      });
      set((s) => ({
        conversations: patchLocalConversation(s.conversations, conversationId, {
          priority: parsePriority(updated.priority),
        }),
      }));
    },

    cycleConversationProgress: async (conversationId) => {
      const current = get().conversations.find((c) => c.id === conversationId);
      if (!current) return;
      const next = nextProgress(current.progress);
      if (get().source !== "daemon") {
        set((s) => ({
          conversations: patchLocalConversation(
            s.conversations,
            conversationId,
            {
              progress: next,
            },
          ),
        }));
        return;
      }
      const updated = await daemonApi.updateConversation(conversationId, {
        progress: next,
      });
      set((s) => ({
        conversations: patchLocalConversation(s.conversations, conversationId, {
          progress: parseProgress(updated.progress),
        }),
      }));
    },

    setConversationProgress: async (conversationId, progress) => {
      if (get().source !== "daemon") {
        set((s) => ({
          conversations: patchLocalConversation(
            s.conversations,
            conversationId,
            {
              progress,
            },
          ),
        }));
        return;
      }
      const updated = await daemonApi.updateConversation(conversationId, {
        progress,
      });
      set((s) => ({
        conversations: patchLocalConversation(s.conversations, conversationId, {
          progress: parseProgress(updated.progress),
        }),
      }));
    },

    moveConversationToBoardColumn: async (conversationId, column) => {
      const progress = progressForBoardColumn(column);
      await get().setConversationProgress(conversationId, progress);
    },

    createProject: async (workspacePath) => {
      const trimmed = workspacePath.trim();
      if (!trimmed) {
        throw new Error("workspace path is required");
      }

      if (get().source !== "daemon" || !get().connection?.connected) {
        const base =
          trimmed.split(/[/\\]/).filter(Boolean).pop() || "project";
        const project: Project = {
          id: `mock-proj-${Date.now()}`,
          name: base,
          workspacePath: trimmed,
          conversationCount: 0,
          runningAgents: 0,
          needsAttention: 0,
          updatedAtMs: Date.now(),
          hasUnread: false,
          lastAttentionMs: 0,
        };
        set((s) => ({
          projects: [...s.projects, project],
          error: null,
          source: "mock",
        }));
        return project.id;
      }

      try {
        const created = toUiProject(await daemonApi.createProject(trimmed));
        const projects = toUiProjects(await daemonApi.listProjects());
        set({ projects, actionError: null });
        void quietHydrateAllConversationLists(get);
        return created.id;
      } catch (e) {
        const message = e instanceof Error ? e.message : String(e);
        set({ actionError: message });
        throw e;
      }
    },
  };
}
