import { useEffect, useMemo, useState } from "react";
import { Command } from "cmdk";
import {
  AlertTriangle,
  Bot,
  FolderGit2,
  LayoutDashboard,
  MessageSquare,
  Monitor,
  Search,
} from "lucide-react";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogTitle,
} from "@/shared/ui/dialog";
import { useUiStore, type PrimaryNav } from "@/store/ui-store";
import { useWorkspaceStore } from "@/store/workspace-store";
import { projectSessionFromEntity } from "@/shared/lib/session-entity";
import { sortByAttentionThenTime } from "@/shared/lib/list-sort";
import { cn } from "@/shared/lib/utils";

type Props = {
  open: boolean;
  onOpenChange: (open: boolean) => void;
};

/**
 * Global ⌘K / Ctrl+K jump palette: projects, conversations, sessions, nav.
 */
export function CommandPalette({ open, onOpenChange }: Props) {
  const [query, setQuery] = useState("");
  const setPrimaryNav = useUiStore((s) => s.setPrimaryNav);
  const selectProject = useUiStore((s) => s.selectProject);
  const openConversation = useUiStore((s) => s.openConversation);
  const openSessionTranscript = useUiStore((s) => s.openSessionTranscript);
  const projects = useWorkspaceStore((s) => s.projects);
  const conversations = useWorkspaceStore((s) => s.conversations);
  // Project sessions from L4 Entity (no deprecated projectSessions mirror).
  const sessionsById = useWorkspaceStore((s) => s.sessionsById);
  const projectSessions = useMemo(
    () =>
      Object.values(sessionsById)
        .filter((e) => !e.parentId)
        .map((e) => projectSessionFromEntity(e)),
    [sessionsById],
  );

  useEffect(() => {
    if (!open) setQuery("");
  }, [open]);

  const sortedProjects = useMemo(
    () => [...projects].sort(sortByAttentionThenTime),
    [projects],
  );

  const navItems: { id: PrimaryNav; label: string; icon: typeof LayoutDashboard }[] =
    [
      { id: "work", label: "Work", icon: LayoutDashboard },
      { id: "attention", label: "Attention", icon: AlertTriangle },
      { id: "agents", label: "Agents", icon: Bot },
      { id: "host", label: "Host", icon: Monitor },
    ];

  const run = (fn: () => void) => {
    fn();
    onOpenChange(false);
  };

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent
        hideClose
        className={cn(
          "max-w-xl overflow-hidden p-0 sm:rounded-2xl",
          "top-[20%] translate-y-0 data-[state=open]:slide-in-from-top-2",
        )}
        onOpenAutoFocus={(e) => {
          // cmdk input receives focus via autoFocus
          e.preventDefault();
          const el = document.querySelector<HTMLInputElement>(
            "[cmdk-input]",
          );
          el?.focus();
        }}
      >
        <DialogTitle className="sr-only">Command palette</DialogTitle>
        <DialogDescription className="sr-only">
          Jump to a project, conversation, session, or navigation section.
        </DialogDescription>
        <Command
          className="flex max-h-[min(70vh,520px)] flex-col"
          label="Global command palette"
          shouldFilter
        >
          <div className="flex items-center gap-2 border-b border-ink/10 px-3">
            <Search className="h-4 w-4 shrink-0 text-ink-muted" />
            <Command.Input
              value={query}
              onValueChange={setQuery}
              placeholder="Jump to project, conversation, session…"
              className="h-11 w-full bg-transparent text-sm text-ink outline-none placeholder:text-ink-muted"
              autoFocus
            />
            <kbd className="hidden shrink-0 rounded border border-ink/10 bg-surface-muted px-1.5 py-0.5 text-3xs font-medium text-ink-muted sm:inline">
              esc
            </kbd>
          </div>
          <Command.List className="scrollbar-thin min-h-0 flex-1 overflow-y-auto p-2">
            <Command.Empty className="px-3 py-8 text-center text-sm text-ink-muted">
              No matches.
            </Command.Empty>

            <Command.Group
              heading="Navigate"
              className="[&_[cmdk-group-heading]]:px-2 [&_[cmdk-group-heading]]:py-1.5 [&_[cmdk-group-heading]]:text-2xs [&_[cmdk-group-heading]]:font-semibold [&_[cmdk-group-heading]]:uppercase [&_[cmdk-group-heading]]:tracking-wide [&_[cmdk-group-heading]]:text-ink-muted"
            >
              {navItems.map((item) => {
                const Icon = item.icon;
                return (
                  <Command.Item
                    key={item.id}
                    value={`nav ${item.label}`}
                    onSelect={() => run(() => setPrimaryNav(item.id))}
                    className={itemClass}
                  >
                    <Icon className="h-4 w-4 shrink-0 opacity-80" />
                    <span>{item.label}</span>
                  </Command.Item>
                );
              })}
            </Command.Group>

            {sortedProjects.length > 0 ? (
              <Command.Group
                heading="Projects"
                className="[&_[cmdk-group-heading]]:px-2 [&_[cmdk-group-heading]]:py-1.5 [&_[cmdk-group-heading]]:text-2xs [&_[cmdk-group-heading]]:font-semibold [&_[cmdk-group-heading]]:uppercase [&_[cmdk-group-heading]]:tracking-wide [&_[cmdk-group-heading]]:text-ink-muted"
              >
                {sortedProjects.map((p) => (
                  <Command.Item
                    key={p.id}
                    value={`project ${p.name} ${p.workspacePath ?? ""}`}
                    onSelect={() => run(() => selectProject(p.id))}
                    className={itemClass}
                  >
                    <FolderGit2 className="h-4 w-4 shrink-0 text-ink-muted" />
                    <span className="min-w-0 flex-1 truncate">{p.name}</span>
                    {p.workspacePath ? (
                      <span className="max-w-[40%] truncate text-2xs text-ink-muted">
                        {p.workspacePath}
                      </span>
                    ) : null}
                  </Command.Item>
                ))}
              </Command.Group>
            ) : null}

            {conversations.length > 0 ? (
              <Command.Group
                heading="Conversations"
                className="[&_[cmdk-group-heading]]:px-2 [&_[cmdk-group-heading]]:py-1.5 [&_[cmdk-group-heading]]:text-2xs [&_[cmdk-group-heading]]:font-semibold [&_[cmdk-group-heading]]:uppercase [&_[cmdk-group-heading]]:tracking-wide [&_[cmdk-group-heading]]:text-ink-muted"
              >
                {conversations.slice(0, 40).map((c) => {
                  const projectName =
                    projects.find((p) => p.id === c.projectId)?.name ?? "";
                  return (
                    <Command.Item
                      key={c.id}
                      value={`conversation ${c.title} ${projectName}`}
                      onSelect={() =>
                        run(() => {
                          if (c.projectId) selectProject(c.projectId);
                          openConversation(c.id);
                        })
                      }
                      className={itemClass}
                    >
                      <MessageSquare className="h-4 w-4 shrink-0 text-ink-muted" />
                      <span className="min-w-0 flex-1 truncate">{c.title}</span>
                      {projectName ? (
                        <span className="max-w-[30%] truncate text-2xs text-ink-muted">
                          {projectName}
                        </span>
                      ) : null}
                    </Command.Item>
                  );
                })}
              </Command.Group>
            ) : null}

            {projectSessions.length > 0 ? (
              <Command.Group
                heading="Sessions"
                className="[&_[cmdk-group-heading]]:px-2 [&_[cmdk-group-heading]]:py-1.5 [&_[cmdk-group-heading]]:text-2xs [&_[cmdk-group-heading]]:font-semibold [&_[cmdk-group-heading]]:uppercase [&_[cmdk-group-heading]]:tracking-wide [&_[cmdk-group-heading]]:text-ink-muted"
              >
                {projectSessions.slice(0, 40).map((s) => {
                  const projectId =
                    conversations.find((c) => c.id === s.conversationId)
                      ?.projectId ?? "";
                  return (
                    <Command.Item
                      key={s.id}
                      value={`session ${s.agent} ${s.shortId} ${s.conversationTitle ?? ""}`}
                      onSelect={() =>
                        run(() => {
                          if (projectId) selectProject(projectId);
                          openSessionTranscript(s.id, s.conversationId);
                        })
                      }
                      className={itemClass}
                    >
                      <Bot className="h-4 w-4 shrink-0 text-ink-muted" />
                      <span className="min-w-0 flex-1 truncate font-mono text-xs">
                        {s.agent}#{s.shortId}
                      </span>
                      {s.conversationTitle ? (
                        <span className="max-w-[40%] truncate text-2xs text-ink-muted">
                          {s.conversationTitle}
                        </span>
                      ) : null}
                    </Command.Item>
                  );
                })}
              </Command.Group>
            ) : null}
          </Command.List>
          <div className="flex items-center justify-between border-t border-ink/5 px-3 py-2 text-2xs text-ink-muted">
            <span>↑↓ navigate · ↵ open</span>
            <span>⌘K</span>
          </div>
        </Command>
      </DialogContent>
    </Dialog>
  );
}

const itemClass = cn(
  "flex cursor-pointer items-center gap-2.5 rounded-lg px-2.5 py-2 text-sm text-ink outline-none",
  "data-[selected=true]:bg-accent-soft data-[selected=true]:text-ink",
  "aria-selected:bg-accent-soft",
);
