import {
  useEffect,
  useMemo,
  useRef,
  useState,
  type FormEvent,
} from "react";
import { Check } from "lucide-react";
import {
  runtimeOptionsFromClis,
  type RuntimeCliDescriptor,
} from "@/features/agents/lib/agentConfigProjection";
import {
  buildCreateConversationInput,
  canSubmitCreateConversation,
  CREATE_CONVERSATION_GIT_MODES,
  CREATE_CONVERSATION_PRIORITIES,
  defaultCreateConversationForm,
  toggleSelectedAgent,
  type ConversationGitMode,
  type CreateConversationFormInput,
} from "@/features/work/lib/create-conversation-form";
import { agentMeta, type AgentRuntime } from "@/shared/lib/mock-data";
import { cn } from "@/shared/lib/utils";
import { Button } from "@/shared/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/shared/ui/dialog";

const FORM_ID = "create-conversation-form";

const fieldShellClass =
  "rounded-xl border border-ink/10 bg-surface-muted/50 transition-colors duration-150 hover:border-ink/20 focus-within:border-ink/30 focus-within:bg-surface-raised";

type Props = {
  open: boolean;
  isCreating: boolean;
  projectName: string;
  clis: RuntimeCliDescriptor[];
  onOpenChange: (open: boolean) => void;
  onCreate: (input: CreateConversationFormInput) => Promise<void>;
};

export function CreateConversationDialog({
  open,
  isCreating,
  projectName,
  clis,
  onOpenChange,
  onCreate,
}: Props) {
  const [title, setTitle] = useState("");
  const [priority, setPriority] = useState<
    ReturnType<typeof defaultCreateConversationForm>["priority"]
  >(null);
  const [gitMode, setGitMode] = useState<ConversationGitMode>("worktree");
  const [selectedAgents, setSelectedAgents] = useState<string[]>([]);
  const [errorMessage, setErrorMessage] = useState<string | null>(null);
  const titleInputRef = useRef<HTMLInputElement>(null);
  const submitInFlightRef = useRef(false);

  const agentOptions = useMemo(() => {
    const runtimes = runtimeOptionsFromClis(clis);
    // Prefer installed; still list known runtimes so host inventory is honest.
    return runtimes.map((r) => ({
      id: r.id,
      displayName: r.displayName,
      installed: r.installed,
    }));
  }, [clis]);

  useEffect(() => {
    if (!open) return;
    const defaults = defaultCreateConversationForm();
    setTitle(defaults.title);
    setPriority(defaults.priority);
    setGitMode(defaults.gitMode);
    setSelectedAgents(defaults.selectedAgents);
    setErrorMessage(null);
    submitInFlightRef.current = false;
    const timerId = window.setTimeout(() => {
      titleInputRef.current?.focus();
      titleInputRef.current?.select();
    }, 50);
    return () => window.clearTimeout(timerId);
  }, [open]);

  const canSubmit =
    canSubmitCreateConversation(title, selectedAgents) && !isCreating;

  const handleSubmit = (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    if (!canSubmit || submitInFlightRef.current) return;
    const input = buildCreateConversationInput({
      title,
      priority,
      selectedAgents,
      gitMode,
    });
    if (!input) return;
    submitInFlightRef.current = true;
    setErrorMessage(null);
    void (async () => {
      try {
        await onCreate(input);
        onOpenChange(false);
      } catch (error) {
        setErrorMessage(
          error instanceof Error
            ? error.message
            : "Failed to create conversation.",
        );
      } finally {
        submitInFlightRef.current = false;
      }
    })();
  };

  return (
    <Dialog
      open={open}
      onOpenChange={(next) => {
        if (!next && isCreating) return;
        onOpenChange(next);
      }}
    >
      <DialogContent
        className="flex max-h-[min(85vh,640px)] w-full max-w-lg flex-col gap-0 overflow-hidden p-0 sm:rounded-2xl"
        data-testid="create-conversation-dialog"
      >
        <DialogHeader className="shrink-0">
          <DialogTitle>New conversation</DialogTitle>
          <DialogDescription>
            Create a conversation in{" "}
            <span className="font-medium text-ink-secondary">{projectName}</span>
            . Agents you add become the roster — only they can be @mentioned
            later.
          </DialogDescription>
        </DialogHeader>

        <form
          id={FORM_ID}
          className="scrollbar-thin min-h-0 flex-1 space-y-5 overflow-y-auto px-5 py-4"
          onSubmit={handleSubmit}
        >
          <div className="space-y-1.5">
            <label
              className="text-sm font-medium text-ink"
              htmlFor="create-conversation-title"
            >
              Title
            </label>
            <div className={cn("flex min-h-11 items-center px-3", fieldShellClass)}>
              <input
                ref={titleInputRef}
                id="create-conversation-title"
                data-testid="create-conversation-title"
                type="text"
                value={title}
                disabled={isCreating}
                autoComplete="off"
                spellCheck={false}
                maxLength={200}
                placeholder="e.g. Auth refactor"
                onChange={(e) => {
                  setTitle(e.target.value);
                  setErrorMessage(null);
                }}
                className="h-8 w-full border-0 bg-transparent px-0 text-sm text-ink outline-none placeholder:text-ink-muted/70 disabled:cursor-not-allowed disabled:opacity-60"
              />
            </div>
          </div>

          <div className="space-y-1.5">
            <div className="text-sm font-medium text-ink">Priority</div>
            <div
              className="flex flex-wrap gap-1.5"
              role="radiogroup"
              aria-label="Priority"
              data-testid="create-conversation-priority"
            >
              {CREATE_CONVERSATION_PRIORITIES.map((opt) => {
                const active = priority === opt.value;
                return (
                  <button
                    key={opt.label}
                    type="button"
                    role="radio"
                    aria-checked={active}
                    disabled={isCreating}
                    onClick={() => setPriority(opt.value)}
                    className={cn(
                      "inline-flex h-8 items-center rounded-lg px-2.5 text-xs font-medium ring-1 ring-inset transition-colors duration-150",
                      active
                        ? "bg-ink text-surface ring-ink"
                        : "bg-surface-muted/60 text-ink-secondary ring-ink/10 hover:bg-surface-hover hover:text-ink",
                      isCreating && "opacity-60",
                    )}
                  >
                    {opt.label}
                  </button>
                );
              })}
            </div>
          </div>

          <div className="space-y-1.5">
            <div className="flex items-baseline justify-between gap-2">
              <div className="text-sm font-medium text-ink">Git workspace</div>
              <span className="text-2xs text-ink-muted">
                Isolation for agent edits
              </span>
            </div>
            <div
              className="grid gap-1.5"
              role="radiogroup"
              aria-label="Git workspace mode"
              data-testid="create-conversation-git-mode"
            >
              {CREATE_CONVERSATION_GIT_MODES.map((opt) => {
                const active = gitMode === opt.value;
                return (
                  <button
                    key={opt.value}
                    type="button"
                    role="radio"
                    aria-checked={active}
                    disabled={isCreating}
                    onClick={() => setGitMode(opt.value)}
                    className={cn(
                      "flex min-h-11 flex-col items-start gap-0.5 rounded-xl border px-3 py-2.5 text-left transition-colors duration-150",
                      active
                        ? "border-ink/25 bg-surface-raised shadow-sm"
                        : "border-ink/10 bg-surface-muted/40 hover:border-ink/20 hover:bg-surface-hover",
                      isCreating && "opacity-60",
                    )}
                  >
                    <span className="text-sm font-medium text-ink">
                      {opt.label}
                    </span>
                    <span className="text-2xs leading-snug text-ink-muted">
                      {opt.description}
                    </span>
                  </button>
                );
              })}
            </div>
          </div>

          <div className="space-y-1.5">
            <div className="flex items-baseline justify-between gap-2">
              <div className="text-sm font-medium text-ink">Agents</div>
              <span className="text-2xs text-ink-muted">
                Roster · required for @mention
              </span>
            </div>
            {agentOptions.length === 0 ? (
              <p className="rounded-xl border border-dashed border-ink/10 bg-surface-muted/40 px-3 py-3 text-xs text-ink-muted">
                No agent runtimes detected yet. Install a CLI, then recreate
                this conversation with members selected.
              </p>
            ) : (
              <div
                className="grid gap-1.5"
                data-testid="create-conversation-agents"
              >
                {agentOptions.map((agent) => {
                  const selected = selectedAgents.includes(agent.id);
                  const meta =
                    agentMeta[agent.id as AgentRuntime] ??
                    ({
                      label: agent.displayName,
                      color: "bg-ink/10 text-ink-secondary",
                    } as const);
                  return (
                    <button
                      key={agent.id}
                      type="button"
                      disabled={isCreating || !agent.installed}
                      aria-pressed={selected}
                      onClick={() => {
                        if (!agent.installed) return;
                        setSelectedAgents((prev) =>
                          toggleSelectedAgent(prev, agent.id),
                        );
                        setErrorMessage(null);
                      }}
                      className={cn(
                        "flex min-h-11 items-center gap-3 rounded-xl border px-3 text-left transition-colors duration-150",
                        selected
                          ? "border-ink/25 bg-surface-raised shadow-sm"
                          : "border-ink/10 bg-surface-muted/40 hover:border-ink/20 hover:bg-surface-hover",
                        (!agent.installed || isCreating) &&
                          "cursor-not-allowed opacity-50",
                      )}
                    >
                      <span
                        className={cn(
                          "flex h-8 w-8 shrink-0 items-center justify-center rounded-lg text-2xs font-semibold",
                          meta.color,
                        )}
                      >
                        {agent.displayName.slice(0, 2).toUpperCase()}
                      </span>
                      <span className="min-w-0 flex-1">
                        <span className="block truncate text-sm font-medium text-ink">
                          {agent.displayName}
                        </span>
                        <span className="block truncate text-2xs text-ink-muted">
                          {agent.installed
                            ? selected
                              ? "Member · can be @mentioned"
                              : "Installed · tap to add to roster"
                            : "Not installed"}
                        </span>
                      </span>
                      <span
                        className={cn(
                          "flex h-5 w-5 shrink-0 items-center justify-center rounded-md border transition-colors",
                          selected
                            ? "border-ink bg-ink text-surface"
                            : "border-ink/15 bg-surface text-transparent",
                        )}
                      >
                        <Check className="h-3 w-3" strokeWidth={2.5} />
                      </span>
                    </button>
                  );
                })}
              </div>
            )}
          </div>

          {errorMessage ? (
            <p className="text-sm text-rose-600" role="alert">
              {errorMessage}
            </p>
          ) : null}
        </form>

        <DialogFooter className="shrink-0">
          <Button
            type="button"
            variant="secondary"
            disabled={isCreating}
            onClick={() => onOpenChange(false)}
          >
            Cancel
          </Button>
          <Button
            type="submit"
            form={FORM_ID}
            disabled={!canSubmit}
            data-testid="create-conversation-submit"
          >
            {isCreating ? "Creating…" : "Create conversation"}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
