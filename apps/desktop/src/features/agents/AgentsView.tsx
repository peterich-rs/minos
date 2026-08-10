import {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  type ReactNode,
} from "react";
import { useQueryClient } from "@tanstack/react-query";
import {
  CheckCircle2,
  CircleDashed,
  Loader2,
  Pencil,
  Plus,
  RefreshCw,
  Trash2,
  X,
} from "lucide-react";
import { agentMeta, type AgentRuntime } from "@/shared/lib/mock-data";
import { useWorkspaceStore } from "@/store/workspace-store";
import { useAccountStore } from "@/store/account-store";
import { daemonApi, isTauriRuntime } from "@/shared/lib/daemon";
import {
  createCloudAgent,
  deleteCloudAgent,
  updateCloudAgent,
  type CloudAgentSummary,
} from "@/shared/lib/minos-cloud";
import { validateProfileName } from "@/shared/lib/agent-route";
import { cn } from "@/shared/lib/utils";
import {
  useAgentProfilesQuery,
  useCloudAgentsQuery,
  useModelsQuery,
} from "@/shared/api/hooks";
import { queryKeys } from "@/shared/api/queryKeys";
import {
  defaultEffortForModel,
  defaultRuntimeId,
  effortOptionsForModel,
  runtimeOptionsFromClis,
  shouldShowEffortPicker,
  type ModelCatalogEntry,
  type RuntimeCliDescriptor,
} from "./lib/agentConfigProjection";
import { MODAL_BACKDROP_CLASS } from "@/shared/ui/modalBackdrop";
import {
  PageHeader,
  PageHeaderPrimaryButton,
} from "@/shared/ui/PageHeader";

/**
 * Unified bot row for Agents UI.
 * Hub is identity SSOT when online; daemon profiles are offline cache only.
 */
type BotRow = {
  /** Hub agent_id when source=hub; daemon profile id when source=daemon. */
  id: string;
  name: string;
  displayName: string;
  description: string;
  runtimeAgent: string;
  model: string;
  reasoningEffort: string;
  systemPrompt: string;
  status: string;
  source: "hub" | "daemon";
  hubSource?: string;
  avatarUrl?: string | null;
};

type DaemonProfile = {
  id: string;
  name: string;
  description: string;
  runtime_agent: string;
  model: string;
  reasoning_effort: string;
  instructions?: string;
};

const fieldClass =
  "w-full rounded-xl border border-ink/10 bg-surface-raised px-3.5 py-2.5 text-sm text-ink shadow-sm outline-none transition placeholder:text-ink-muted/70 focus:border-primary/30 focus:ring-2 focus:ring-primary/20";

function cloudAgentToRow(a: CloudAgentSummary): BotRow {
  return {
    id: a.agentId,
    name: a.name,
    displayName: a.displayName || a.name,
    description: a.description,
    runtimeAgent: a.runtimeAgent,
    model: a.model,
    reasoningEffort: a.defaultReasoningEffort,
    systemPrompt: a.systemPrompt,
    status: a.status || "active",
    source: "hub",
    hubSource: a.source,
    avatarUrl: a.avatarUrl,
  };
}

function daemonProfileToRow(p: DaemonProfile): BotRow {
  return {
    id: p.id,
    name: p.name,
    displayName: p.name,
    description: p.description,
    runtimeAgent: p.runtime_agent,
    model: p.model,
    reasoningEffort: p.reasoning_effort,
    systemPrompt: p.instructions ?? "",
    status: "active",
    source: "daemon",
  };
}

export function AgentsView() {
  const clis = useWorkspaceStore((s) => s.clis);
  const clisStatus = useWorkspaceStore((s) => s.clisStatus);
  const loadClis = useWorkspaceStore((s) => s.loadClis);
  const source = useWorkspaceStore((s) => s.source);
  const deviceId = useAccountStore((s) => s.deviceId);
  const accessToken = useAccountStore((s) => s.session?.accessToken);
  const cloudOnline = Boolean(accessToken?.trim());
  const queryClient = useQueryClient();

  // Hub bot directory is SSOT when account is online.
  const cloudAgentsQuery = useCloudAgentsQuery();
  // Daemon profiles: offline cache / Host launch buffer only.
  const profilesQuery = useAgentProfilesQuery();
  const daemonProfiles = (profilesQuery.data ?? []) as DaemonProfile[];

  const [createOpen, setCreateOpen] = useState(false);
  const [editBot, setEditBot] = useState<BotRow | null>(null);
  const [error, setError] = useState<string | null>(null);

  const bots = useMemo((): BotRow[] => {
    if (cloudOnline && cloudAgentsQuery.isSuccess) {
      // Prefer user-configured bots in the product directory; host_runtime is seed-only.
      const hub = (cloudAgentsQuery.data ?? [])
        .filter((a) => (a.source || "user") !== "host_runtime")
        .map(cloudAgentToRow);
      return hub;
    }
    // Offline / not signed in: fall back to daemon cache.
    return daemonProfiles.map(daemonProfileToRow);
  }, [
    cloudOnline,
    cloudAgentsQuery.isSuccess,
    cloudAgentsQuery.data,
    daemonProfiles,
  ]);

  const botsLoading =
    (cloudOnline &&
      (cloudAgentsQuery.isLoading || cloudAgentsQuery.isFetching)) ||
    (!cloudOnline &&
      (profilesQuery.isLoading || profilesQuery.isFetching));

  const loadBots = useCallback(async () => {
    if (cloudOnline) {
      await queryClient.invalidateQueries({ queryKey: queryKeys.cloudAgents });
    }
    await queryClient.invalidateQueries({ queryKey: queryKeys.agentProfiles });
  }, [queryClient, cloudOnline]);

  useEffect(() => {
    if (source !== "daemon") return;
    void loadClis();
  }, [source, loadClis]);

  // One-shot: import offline daemon profile cache into Hub bot directory when
  // Account comes online: import offline daemon profile cache into Hub bot
  // directory. Idempotent by name.
  const importOnceRef = useRef(false);
  useEffect(() => {
    if (!cloudOnline || !accessToken?.trim() || !deviceId) return;
    if (importOnceRef.current) return;
    if (!profilesQuery.isSuccess || !cloudAgentsQuery.isSuccess) return;
    importOnceRef.current = true;
    const hubNames = new Set(
      (cloudAgentsQuery.data ?? []).map((a) =>
        (a.name || a.displayName || "").trim().toLowerCase(),
      ),
    );
    const missing = daemonProfiles.filter((p) => {
      const n = (p.name || "").trim().toLowerCase();
      return n.length > 0 && !hubNames.has(n);
    });
    if (missing.length === 0) return;
    void (async () => {
      let imported = 0;
      for (const p of missing) {
        try {
          await createCloudAgent(deviceId, accessToken, {
            name: p.name,
            displayName: p.name,
            description: p.description ?? "",
            runtimeAgent: p.runtime_agent,
            model: p.model ?? "",
            defaultReasoningEffort: p.reasoning_effort ?? "",
            systemPrompt: p.instructions ?? "",
          });
          imported += 1;
        } catch (e) {
          console.warn("[agents] bulk-import profile failed", p.name, e);
        }
      }
      if (imported > 0) {
        await queryClient.invalidateQueries({ queryKey: queryKeys.cloudAgents });
      }
    })();
  }, [
    cloudOnline,
    accessToken,
    deviceId,
    profilesQuery.isSuccess,
    cloudAgentsQuery.isSuccess,
    cloudAgentsQuery.data,
    daemonProfiles,
    queryClient,
  ]);

  useEffect(() => {
    const err = cloudOnline ? cloudAgentsQuery.error : profilesQuery.error;
    if (err) {
      setError(err instanceof Error ? err.message : String(err));
    } else if (
      (cloudOnline && cloudAgentsQuery.isSuccess) ||
      (!cloudOnline && profilesQuery.isSuccess)
    ) {
      setError(null);
    }
  }, [
    cloudOnline,
    cloudAgentsQuery.error,
    cloudAgentsQuery.isSuccess,
    profilesQuery.error,
    profilesQuery.isSuccess,
  ]);

  const phase = clisStatus.phase;

  return (
    <div className="flex min-h-0 flex-1 flex-col bg-canvas-soft/40">
      <PageHeader
        title="Agents"
        description={
          cloudOnline
            ? "Global bot directory on Hub (identity SSOT). Edit digital body — model, reasoning effort, system prompt — then pull bots into conversations as participants. Local CLI inventory is Host capability only."
            : "Sign in to manage the Hub bot directory (identity SSOT). Offline: Host CLI inventory and local profile cache only — not multi-device identity."
        }
        action={
          <PageHeaderPrimaryButton onClick={() => setCreateOpen(true)}>
            <Plus className="h-3.5 w-3.5" />
            Create agent
          </PageHeaderPrimaryButton>
        }
      />

      {error ? (
        <p className="px-6 pt-3 text-xs text-status-failed">{error}</p>
      ) : null}

      <div className="scrollbar-thin min-h-0 flex-1 space-y-8 overflow-y-auto p-5 sm:p-6">
        <section>
          <div className="mb-3 flex items-center justify-between px-0.5">
            <h2 className="text-2xs font-semibold uppercase tracking-[0.08em] text-ink-muted">
              CLI inventory
            </h2>
            <button
              type="button"
              onClick={() => void loadClis()}
              className="inline-flex items-center gap-1 rounded-lg px-2 py-1 text-2xs font-medium text-ink-muted hover:bg-surface-muted hover:text-ink"
            >
              <RefreshCw className="h-3 w-3" />
              Re-detect
            </button>
          </div>
          {phase === "error" && clis.length === 0 ? (
            <div className="flex flex-col items-center gap-3 px-6 py-10 text-center">
              <p className="text-sm text-rose-600">
                {clisStatus.error ?? "Failed to detect CLIs"}
              </p>
              <button
                type="button"
                onClick={() => void loadClis()}
                className="rounded-xl bg-ink px-3 py-1.5 text-xs font-semibold text-surface"
              >
                Retry detect
              </button>
            </div>
          ) : (
            <div className="grid content-start gap-3 sm:grid-cols-2 lg:grid-cols-3">
              {phase === "loading" && clis.length === 0 ? (
                <p className="col-span-full py-8 text-center text-sm text-ink-muted">
                  Detecting installed CLIs…
                </p>
              ) : null}
              {clis.map((rt) => {
                const agent = rt.agent as AgentRuntime;
                const meta = agentMeta[agent] ?? {
                  label: rt.displayName ?? rt.agent,
                  color: "bg-ink/10 text-ink-secondary",
                };
                return (
                  <div
                    key={rt.agent}
                    className="rounded-2xl border border-ink/5 bg-surface-raised p-4 shadow-sm"
                  >
                    <div className="flex items-start justify-between gap-2">
                      <div>
                        <div
                          className={cn(
                            "inline-flex rounded-lg px-2 py-0.5 text-xs font-semibold",
                            meta.color,
                          )}
                        >
                          {rt.displayName ?? meta.label}
                        </div>
                        <div className="mt-2 text-sm text-ink-secondary">
                          @{rt.agent}
                        </div>
                      </div>
                      {rt.installed ? (
                        <span className="inline-flex items-center gap-1 text-2xs font-medium text-emerald-700">
                          <CheckCircle2 className="h-3.5 w-3.5" />
                          Installed
                        </span>
                      ) : (
                        <span className="inline-flex items-center gap-1 text-2xs font-medium text-ink-muted">
                          <CircleDashed className="h-3.5 w-3.5" />
                          Missing
                        </span>
                      )}
                    </div>
                    {rt.installed ? (
                      <dl className="mt-4 space-y-1.5 text-xs">
                        <div className="flex justify-between gap-2">
                          <dt className="text-ink-muted">Status</dt>
                          <dd className="font-medium text-ink">{rt.status}</dd>
                        </div>
                      </dl>
                    ) : (
                      <p className="mt-4 text-xs text-ink-muted">
                        Install the CLI and re-detect to run @{rt.agent} on this
                        Host. Product bot identity lives on Hub, not as a bare
                        runtime name.
                      </p>
                    )}
                  </div>
                );
              })}
            </div>
          )}
        </section>

        <section>
          <div className="mb-3 flex items-center justify-between px-0.5">
            <h2 className="text-2xs font-semibold uppercase tracking-[0.08em] text-ink-muted">
              {cloudOnline ? "Bot directory (Hub)" : "Local profile cache"}
            </h2>
            <div className="flex items-center gap-2">
              {botsLoading ? (
                <Loader2 className="h-3.5 w-3.5 animate-spin text-ink-muted" />
              ) : null}
              <button
                type="button"
                onClick={() => void loadBots()}
                className="inline-flex items-center gap-1 rounded-lg px-2 py-1 text-2xs font-medium text-ink-muted hover:bg-surface-muted hover:text-ink"
              >
                <RefreshCw className="h-3 w-3" />
                Refresh
              </button>
            </div>
          </div>
          {bots.length === 0 ? (
            <p className="rounded-2xl border border-dashed border-ink/10 bg-surface-raised/70 px-4 py-10 text-center text-sm leading-relaxed text-ink-muted">
              {cloudOnline
                ? "No bots yet. Create one on Hub to pin runtime, model, role brief, and system prompt — then add it as a conversation participant."
                : "No local profile cache. Sign in to create bots on Hub (identity SSOT), or create offline for this Host only."}
            </p>
          ) : (
            <div className="grid content-start gap-3 sm:grid-cols-2 lg:grid-cols-3">
              {bots.map((bot) => {
                const runtime = bot.runtimeAgent as AgentRuntime;
                const meta = agentMeta[runtime] ?? {
                  label: bot.runtimeAgent,
                  color: "bg-ink/10 text-ink-secondary",
                };
                return (
                  <div
                    key={`${bot.source}:${bot.id}`}
                    className="rounded-2xl border border-ink/5 bg-surface-raised p-4 shadow-sm"
                  >
                    <div className="flex items-start justify-between gap-2">
                      <div className="min-w-0">
                        <div className="truncate text-sm font-semibold text-ink">
                          {bot.displayName}
                        </div>
                        <div className="mt-1.5 flex flex-wrap items-center gap-1.5">
                          <span
                            className={cn(
                              "inline-flex rounded-lg px-1.5 py-0.5 text-2xs font-semibold",
                              meta.color,
                            )}
                          >
                            {meta.label}
                          </span>
                          <span className="truncate font-mono text-2xs text-ink-muted">
                            {bot.model}
                          </span>
                          {bot.source === "hub" ? (
                            <span className="rounded-md bg-emerald-50 px-1.5 py-0.5 text-3xs font-medium text-emerald-800">
                              Hub
                            </span>
                          ) : (
                            <span className="rounded-md bg-amber-50 px-1.5 py-0.5 text-3xs font-medium text-amber-800">
                              Local cache
                            </span>
                          )}
                        </div>
                      </div>
                      <div className="flex shrink-0 items-center gap-0.5">
                        <button
                          type="button"
                          title="Edit digital body"
                          onClick={() => setEditBot(bot)}
                          className="rounded-lg p-1.5 text-ink-muted hover:bg-surface-muted hover:text-ink"
                        >
                          <Pencil className="h-3.5 w-3.5" />
                        </button>
                        <button
                          type="button"
                          title="Delete bot"
                          onClick={() => {
                            void (async () => {
                              try {
                                if (bot.source === "hub") {
                                  const token = accessToken?.trim();
                                  if (!token) {
                                    throw new Error("Sign in required to delete Hub bots");
                                  }
                                  await deleteCloudAgent(deviceId, token, bot.id);
                                  // Best-effort: drop matching daemon cache by name.
                                  if (isTauriRuntime()) {
                                    const match = daemonProfiles.find(
                                      (p) =>
                                        p.name.trim().toLowerCase() ===
                                        bot.name.trim().toLowerCase(),
                                    );
                                    if (match) {
                                      try {
                                        await daemonApi.deleteAgentProfile(match.id);
                                      } catch {
                                        /* cache cleanup optional */
                                      }
                                    }
                                  }
                                } else {
                                  await daemonApi.deleteAgentProfile(bot.id);
                                }
                                await loadBots();
                              } catch (e) {
                                setError(
                                  e instanceof Error ? e.message : String(e),
                                );
                              }
                            })();
                          }}
                          className="rounded-lg p-1.5 text-ink-muted hover:bg-status-failed/10 hover:text-status-failed"
                        >
                          <Trash2 className="h-3.5 w-3.5" />
                        </button>
                      </div>
                    </div>
                    {bot.description ? (
                      <p
                        className="mt-2 line-clamp-2 text-xs text-ink-muted"
                        title="Role brief shown to teammate agents"
                      >
                        {bot.description}
                      </p>
                    ) : (
                      <p className="mt-2 text-xs text-amber-700/80">
                        No role brief — teammates won&apos;t know this bot&apos;s
                        boundaries until you add one.
                      </p>
                    )}
                    <dl className="mt-3 space-y-1 text-xs">
                      {bot.reasoningEffort ? (
                        <div className="flex justify-between gap-2">
                          <dt className="text-ink-muted">Effort</dt>
                          <dd className="font-medium capitalize text-ink">
                            {bot.reasoningEffort}
                          </dd>
                        </div>
                      ) : null}
                      {bot.systemPrompt.trim() ? (
                        <div className="flex justify-between gap-2">
                          <dt className="text-ink-muted">Instructions</dt>
                          <dd className="font-medium text-ink">Custom</dd>
                        </div>
                      ) : null}
                      {bot.status && bot.status !== "active" ? (
                        <div className="flex justify-between gap-2">
                          <dt className="text-ink-muted">Status</dt>
                          <dd className="font-medium capitalize text-ink">
                            {bot.status}
                          </dd>
                        </div>
                      ) : null}
                    </dl>
                  </div>
                );
              })}
            </div>
          )}
        </section>
      </div>

      {createOpen ? (
        <CreateAgentDialog
          clis={clis}
          cloudOnline={cloudOnline}
          deviceId={deviceId}
          accessToken={accessToken}
          onClose={() => setCreateOpen(false)}
          onCreated={async () => {
            setCreateOpen(false);
            await loadBots();
          }}
        />
      ) : null}

      {editBot ? (
        <EditAgentDialog
          bot={editBot}
          cloudOnline={cloudOnline && editBot.source === "hub"}
          deviceId={deviceId}
          accessToken={accessToken}
          onClose={() => setEditBot(null)}
          onSaved={async () => {
            setEditBot(null);
            await loadBots();
          }}
        />
      ) : null}
    </div>
  );
}

function CreateAgentDialog({
  clis,
  cloudOnline,
  deviceId,
  accessToken,
  onClose,
  onCreated,
}: {
  clis: RuntimeCliDescriptor[];
  cloudOnline: boolean;
  deviceId: string;
  accessToken: string | undefined;
  onClose: () => void;
  onCreated: () => void | Promise<void>;
}) {
  const runtimeOptions = useMemo(
    () => runtimeOptionsFromClis(clis),
    [clis],
  );
  const defaultRuntime =
    defaultRuntimeId(runtimeOptions) ?? runtimeOptions[0]?.id ?? "codex";

  const [name, setName] = useState("");
  const [description, setDescription] = useState("");
  const [instructions, setInstructions] = useState("");
  const [runtime, setRuntime] = useState(defaultRuntime);
  const [model, setModel] = useState("");
  const [customModel, setCustomModel] = useState("");
  const [effort, setEffort] = useState("");
  const [saving, setSaving] = useState(false);
  const [err, setErr] = useState<string | null>(null);

  const queryClient = useQueryClient();
  const modelsQuery = useModelsQuery(runtime);
  const models = (modelsQuery.data?.models ?? []) as ModelCatalogEntry[];
  const modelsSource = modelsQuery.data?.source ?? "";
  const loadingModels = modelsQuery.isLoading || modelsQuery.isFetching;
  const refreshModels = useCallback(() => {
    void queryClient.invalidateQueries({
      queryKey: queryKeys.models(runtime),
    });
  }, [queryClient, runtime]);

  // Reset selection when runtime catalog changes.
  useEffect(() => {
    setCustomModel("");
    if (modelsQuery.error) {
      setModel("");
      setEffort("");
      setErr(
        modelsQuery.error instanceof Error
          ? modelsQuery.error.message
          : String(modelsQuery.error),
      );
      return;
    }
    if (!modelsQuery.isSuccess) return;
    const list = models;
    const def = list.find((m) => m.is_default) ?? list[0] ?? null;
    setModel(def?.id ?? "");
    setEffort(defaultEffortForModel(def));
    setErr(null);
    // Only re-seed defaults when the catalog identity changes.
    // eslint-disable-next-line react-hooks/exhaustive-deps -- models list from query
  }, [runtime, modelsQuery.isSuccess, modelsQuery.dataUpdatedAt, modelsQuery.error]);

  const selectedModelMeta = models.find((m) => m.id === model);
  const effortOptions = effortOptionsForModel(selectedModelMeta);
  const showEffort = shouldShowEffortPicker(selectedModelMeta);

  const resolvedModel = (customModel.trim() || model).trim();

  const modelSelectOptions = useMemo(
    () =>
      models.map((m) => ({
        value: m.id,
        label: m.is_default ? `${m.display_name} · default` : m.display_name,
      })),
    [models],
  );

  const onSubmit = async () => {
    const nameErr = validateProfileName(name);
    if (nameErr) {
      setErr(nameErr);
      return;
    }
    if (!resolvedModel) {
      setErr("Model is required");
      return;
    }
    setSaving(true);
    try {
      const trimmedName = name.trim();
      const trimmedDesc = description.trim();
      const trimmedInstr = instructions.trim();
      const effortVal = showEffort ? effort.trim() : "";

      if (cloudOnline) {
        const token = accessToken?.trim();
        if (!token) {
          throw new Error("Sign in required to create Hub bots");
        }
        // Hub is bot identity SSOT.
        await createCloudAgent(deviceId, token, {
          name: trimmedName,
          displayName: trimmedName,
          description: trimmedDesc,
          runtimeAgent: runtime,
          model: resolvedModel,
          defaultReasoningEffort: effortVal,
          systemPrompt: trimmedInstr,
        });
        // Optional Host cache mirror for offline / session launch. Daemon mint
        // its own profile id — name-matched cache, not dual identity SSOT.
        if (isTauriRuntime()) {
          try {
            await daemonApi.createAgentProfile({
              name: trimmedName,
              description: trimmedDesc,
              runtimeAgent: runtime,
              model: resolvedModel,
              reasoningEffort: effortVal,
              instructions: trimmedInstr,
            });
          } catch {
            /* cache mirror best-effort */
          }
        }
        await queryClient.invalidateQueries({ queryKey: queryKeys.cloudAgents });
        await queryClient.invalidateQueries({
          queryKey: queryKeys.agentProfiles,
        });
      } else {
        // Offline: local cache only (not multi-device identity).
        await daemonApi.createAgentProfile({
          name: trimmedName,
          description: trimmedDesc,
          runtimeAgent: runtime,
          model: resolvedModel,
          reasoningEffort: effortVal,
          instructions: trimmedInstr,
        });
        await queryClient.invalidateQueries({
          queryKey: queryKeys.agentProfiles,
        });
      }
      await onCreated();
    } catch (e) {
      setErr(e instanceof Error ? e.message : String(e));
    } finally {
      setSaving(false);
    }
  };

  return (
    <div
      className={cn(
        "fixed inset-0 z-50 flex items-center justify-center p-4",
        MODAL_BACKDROP_CLASS,
      )}
      role="dialog"
      aria-modal="true"
      aria-label="Create agent"
      onPointerDown={(e) => {
        if (e.target === e.currentTarget) onClose();
      }}
    >
      <div className="flex max-h-[min(92vh,760px)] w-full max-w-[440px] flex-col overflow-hidden rounded-2xl border border-ink/8 bg-surface shadow-2xl">
        <header className="flex shrink-0 items-center justify-between px-5 pb-3 pt-5">
          <div>
            <h2 className="text-base font-semibold tracking-tight text-ink">
              Create agent
            </h2>
            <p className="mt-0.5 text-xs text-ink-muted">
              {cloudOnline
                ? "Creates a global Hub bot (identity SSOT). Digital body is shared across conversations."
                : "Offline: Host profile cache only. Sign in to create a Hub bot."}
            </p>
          </div>
          <button
            type="button"
            onClick={onClose}
            className="rounded-xl p-2 text-ink-muted hover:bg-surface-raised/80 hover:text-ink"
          >
            <X className="h-4 w-4" />
          </button>
        </header>

        <div className="scrollbar-thin min-h-0 flex-1 space-y-5 overflow-y-auto px-5 pb-2">
          <Field label="Name" required>
            <input
              value={name}
              onChange={(e) => setName(e.target.value)}
              placeholder="e.g. Research Grok"
              className={fieldClass}
              autoFocus
            />
          </Field>

          <Field
            label="Role brief for teammates"
            hint="peer-facing · ≤500 chars · recommended"
          >
            <textarea
              value={description}
              onChange={(e) => setDescription(e.target.value)}
              rows={2}
              maxLength={500}
              placeholder="e.g. implements features in the worktree; prefers small PRs"
              className={cn(fieldClass, "resize-none")}
            />
            <p className="mt-1 text-2xs leading-snug text-ink-muted">
              Other agents see this via conversation roster and session
              briefing. Conversation create can override it per chat.
            </p>
          </Field>

          <Field label="Runtime">
            <div className="grid grid-cols-2 gap-2 sm:grid-cols-3">
              {runtimeOptions.map((opt) => {
                const meta = agentMeta[opt.id as AgentRuntime] ?? {
                  label: opt.displayName,
                  color: "bg-ink/10 text-ink-secondary",
                };
                const isOn = opt.installed;
                const selected = runtime === opt.id;
                return (
                  <button
                    key={opt.id}
                    type="button"
                    disabled={!isOn}
                    onClick={() => setRuntime(opt.id)}
                    className={cn(
                      "rounded-xl border px-2.5 py-2.5 text-left transition",
                      selected
                        ? "border-ink/30 bg-surface-raised shadow-sm ring-2 ring-ink/10"
                        : "border-ink/8 bg-surface-raised/60 hover:border-ink/15 hover:bg-surface-raised",
                      !isOn && "cursor-not-allowed opacity-40",
                    )}
                  >
                    <div
                      className={cn(
                        "inline-flex rounded-md px-1.5 py-0.5 text-2xs font-semibold",
                        meta.color,
                      )}
                    >
                      {opt.displayName || meta.label}
                    </div>
                    <div className="mt-1 font-mono text-3xs text-ink-muted">
                      @{opt.id}
                      {!isOn ? " · missing" : ""}
                    </div>
                  </button>
                );
              })}
            </div>
          </Field>

          <Field
            label="Model"
            trailing={
              <button
                type="button"
                title="Refresh models for this runtime"
                onClick={() => refreshModels()}
                className="inline-flex items-center gap-1 rounded-lg px-1.5 py-0.5 text-2xs text-ink-muted hover:bg-surface-raised hover:text-ink"
              >
                <RefreshCw
                  className={cn("h-3 w-3", loadingModels && "animate-spin")}
                />
                Refresh
              </button>
            }
          >
            <select
              value={model}
              disabled={loadingModels || modelSelectOptions.length === 0}
              onChange={(e) => {
                const id = e.target.value;
                setModel(id);
                setCustomModel("");
                const m = models.find((x) => x.id === id);
                setEffort(defaultEffortForModel(m));
              }}
              className={cn(fieldClass, "appearance-none pr-9")}
              style={{
                backgroundImage: `url("data:image/svg+xml,${encodeURIComponent(
                  `<svg xmlns='http://www.w3.org/2000/svg' width='16' height='16' viewBox='0 0 24 24' fill='none' stroke='%2378716c' stroke-width='2' stroke-linecap='round' stroke-linejoin='round'><polyline points='6 9 12 15 18 9'/></svg>`,
                )}")`,
                backgroundRepeat: "no-repeat",
                backgroundPosition: "right 0.75rem center",
                backgroundSize: "16px 16px",
              }}
            >
              <option value="" disabled>
                {loadingModels
                  ? "Loading models…"
                  : modelSelectOptions.length === 0
                    ? "No models — type a custom id below"
                    : "Select a model"}
              </option>
              {modelSelectOptions.map((o) => (
                <option key={o.value} value={o.value}>
                  {o.label}
                </option>
              ))}
            </select>
            <input
              value={customModel}
              onChange={(e) => setCustomModel(e.target.value)}
              placeholder="Or type a custom model id…"
              className={cn(fieldClass, "mt-2")}
            />
            <p className="mt-1.5 text-2xs text-ink-muted">
              {loadingModels
                ? `Discovering models for ${
                    runtimeOptions.find((o) => o.id === runtime)?.displayName ??
                    agentMeta[runtime as AgentRuntime]?.label ??
                    runtime
                  }…`
                : modelsSource
                  ? `Loaded ${models.length} model${models.length === 1 ? "" : "s"} · ${modelsSource}`
                  : "No models discovered yet"}
            </p>
          </Field>

          {showEffort ? (
            <Field label="Reasoning effort">
              <div className="flex flex-wrap gap-1.5">
                <EffortChip
                  active={!effort}
                  label="Default"
                  onClick={() => setEffort("")}
                />
                {effortOptions.map((e) => (
                  <EffortChip
                    key={e}
                    active={effort === e}
                    label={e}
                    onClick={() => setEffort(e)}
                  />
                ))}
              </div>
            </Field>
          ) : null}

          <Field label="Instructions" hint="system prompt / digital body">
            <textarea
              value={instructions}
              onChange={(e) => setInstructions(e.target.value)}
              rows={4}
              maxLength={12000}
              placeholder="Optional system prompt for this bot (role, constraints, style)…"
              className={cn(fieldClass, "resize-y min-h-[96px]")}
            />
            <p className="mt-1.5 text-2xs leading-snug text-ink-muted">
              Stored on the Hub bot identity (system_prompt). Session start may
              still append Minos teamwork guidance.
            </p>
          </Field>

          {err ? <p className="text-xs text-rose-600">{err}</p> : null}
        </div>

        <footer className="flex shrink-0 justify-end gap-2 border-t border-ink/5 bg-surface-muted/60 px-5 py-3.5">
          <button
            type="button"
            onClick={onClose}
            className="rounded-xl border border-ink/10 bg-surface-raised px-3.5 py-2 text-xs font-medium text-ink-muted hover:bg-surface"
          >
            Cancel
          </button>
          <button
            type="button"
            disabled={saving || loadingModels}
            onClick={() => void onSubmit()}
            className="rounded-xl bg-ink px-4 py-2 text-xs font-semibold text-surface shadow-sm hover:bg-ink/90 disabled:opacity-50"
          >
            {saving ? "Creating…" : "Create agent"}
          </button>
        </footer>
      </div>
    </div>
  );
}

/**
 * Edit digital body: name, description, instructions/system_prompt, effort (Hub).
 * Runtime/model stay fixed after create on Host cache; Hub update may rewrite them.
 */
function EditAgentDialog({
  bot,
  cloudOnline,
  deviceId,
  accessToken,
  onClose,
  onSaved,
}: {
  bot: BotRow;
  cloudOnline: boolean;
  deviceId: string;
  accessToken: string | undefined;
  onClose: () => void;
  onSaved: () => void | Promise<void>;
}) {
  const [name, setName] = useState(bot.displayName || bot.name);
  const [description, setDescription] = useState(bot.description);
  const [instructions, setInstructions] = useState(bot.systemPrompt);
  const [effort, setEffort] = useState(bot.reasoningEffort);
  const [saving, setSaving] = useState(false);
  const [err, setErr] = useState<string | null>(null);

  const queryClient = useQueryClient();
  const modelsQuery = useModelsQuery(bot.runtimeAgent);
  const models = (modelsQuery.data?.models ?? []) as ModelCatalogEntry[];
  const selectedModelMeta =
    models.find((m) => m.id === bot.model) ?? models[0] ?? null;
  const effortOptions = effortOptionsForModel(selectedModelMeta);
  const showEffort = shouldShowEffortPicker(selectedModelMeta) || effortOptions.length > 0;

  const onSubmit = async () => {
    const nameErr = validateProfileName(name);
    if (nameErr) {
      setErr(nameErr);
      return;
    }
    setSaving(true);
    try {
      const trimmedName = name.trim();
      const trimmedDesc = description.trim();
      const trimmedInstr = instructions.trim();
      const effortVal = showEffort ? effort.trim() : bot.reasoningEffort;

      if (cloudOnline && bot.source === "hub") {
        const token = accessToken?.trim();
        if (!token) {
          throw new Error("Sign in required to update Hub bots");
        }
        await updateCloudAgent(deviceId, token, bot.id, {
          name: trimmedName,
          displayName: trimmedName,
          description: trimmedDesc,
          avatarUrl: bot.avatarUrl ?? undefined,
          runtimeAgent: bot.runtimeAgent,
          model: bot.model,
          defaultReasoningEffort: effortVal,
          systemPrompt: trimmedInstr,
          status: bot.status || "active",
        });
        // Best-effort Host cache: update name-matched profile if present.
        if (isTauriRuntime()) {
          try {
            const { profiles } = await daemonApi.listAgentProfiles();
            const match = (profiles ?? []).find(
              (p) =>
                p.name.trim().toLowerCase() === bot.name.trim().toLowerCase() ||
                p.name.trim().toLowerCase() === trimmedName.toLowerCase(),
            );
            if (match) {
              await daemonApi.updateAgentProfile({
                id: match.id,
                name: trimmedName,
                description: trimmedDesc,
                instructions: trimmedInstr,
              });
            }
          } catch {
            /* cache optional */
          }
        }
        await queryClient.invalidateQueries({ queryKey: queryKeys.cloudAgents });
        await queryClient.invalidateQueries({
          queryKey: queryKeys.agentProfiles,
        });
      } else {
        await daemonApi.updateAgentProfile({
          id: bot.id,
          name: trimmedName,
          description: trimmedDesc,
          instructions: trimmedInstr,
        });
        await queryClient.invalidateQueries({
          queryKey: queryKeys.agentProfiles,
        });
      }
      await onSaved();
    } catch (e) {
      setErr(e instanceof Error ? e.message : String(e));
    } finally {
      setSaving(false);
    }
  };

  return (
    <div
      className={cn(
        "fixed inset-0 z-50 flex items-center justify-center p-4",
        MODAL_BACKDROP_CLASS,
      )}
      role="dialog"
      aria-modal="true"
      aria-label="Edit agent"
      onPointerDown={(e) => {
        if (e.target === e.currentTarget) onClose();
      }}
    >
      <div className="flex max-h-[min(92vh,640px)] w-full max-w-[440px] flex-col overflow-hidden rounded-2xl border border-ink/8 bg-surface shadow-2xl">
        <header className="flex shrink-0 items-center justify-between px-5 pb-3 pt-5">
          <div>
            <h2 className="text-base font-semibold tracking-tight text-ink">
              Edit digital body
            </h2>
            <p className="mt-0.5 text-xs text-ink-muted">
              {bot.source === "hub"
                ? "Updates Hub bot identity (system_prompt / default_reasoning_effort)."
                : "Updates local Host cache only."}
            </p>
          </div>
          <button
            type="button"
            onClick={onClose}
            className="rounded-xl p-2 text-ink-muted hover:bg-surface-raised/80 hover:text-ink"
          >
            <X className="h-4 w-4" />
          </button>
        </header>

        <div className="scrollbar-thin min-h-0 flex-1 space-y-5 overflow-y-auto px-5 pb-2">
          <Field label="Name" required>
            <input
              value={name}
              onChange={(e) => setName(e.target.value)}
              className={fieldClass}
              autoFocus
            />
          </Field>

          <Field label="Role brief" hint="peer-facing · ≤500 chars">
            <textarea
              value={description}
              onChange={(e) => setDescription(e.target.value)}
              rows={2}
              maxLength={500}
              className={cn(fieldClass, "resize-none")}
            />
          </Field>

          <div className="rounded-xl border border-ink/8 bg-surface-muted/50 px-3 py-2 text-xs text-ink-muted">
            Runtime <span className="font-mono text-ink">@{bot.runtimeAgent}</span>
            {" · "}
            Model <span className="font-mono text-ink">{bot.model || "—"}</span>
          </div>

          {showEffort && bot.source === "hub" ? (
            <Field label="Reasoning effort">
              <div className="flex flex-wrap gap-1.5">
                <EffortChip
                  active={!effort}
                  label="Default"
                  onClick={() => setEffort("")}
                />
                {effortOptions.map((e) => (
                  <EffortChip
                    key={e}
                    active={effort === e}
                    label={e}
                    onClick={() => setEffort(e)}
                  />
                ))}
              </div>
            </Field>
          ) : null}

          <Field label="Instructions" hint="system prompt">
            <textarea
              value={instructions}
              onChange={(e) => setInstructions(e.target.value)}
              rows={5}
              maxLength={12000}
              className={cn(fieldClass, "resize-y min-h-[120px]")}
            />
          </Field>

          {err ? <p className="text-xs text-rose-600">{err}</p> : null}
        </div>

        <footer className="flex shrink-0 justify-end gap-2 border-t border-ink/5 bg-surface-muted/60 px-5 py-3.5">
          <button
            type="button"
            onClick={onClose}
            className="rounded-xl border border-ink/10 bg-surface-raised px-3.5 py-2 text-xs font-medium text-ink-muted hover:bg-surface"
          >
            Cancel
          </button>
          <button
            type="button"
            disabled={saving}
            onClick={() => void onSubmit()}
            className="rounded-xl bg-ink px-4 py-2 text-xs font-semibold text-surface shadow-sm hover:bg-ink/90 disabled:opacity-50"
          >
            {saving ? "Saving…" : "Save"}
          </button>
        </footer>
      </div>
    </div>
  );
}

function EffortChip({
  active,
  label,
  onClick,
}: {
  active: boolean;
  label: string;
  onClick: () => void;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      className={cn(
        "rounded-full px-3 py-1.5 text-xs font-medium capitalize transition",
        active
          ? "bg-ink text-surface shadow-sm"
          : "bg-surface-raised text-ink-secondary ring-1 ring-ink/10 hover:ring-ink/20",
      )}
    >
      {label}
    </button>
  );
}

function Field({
  label,
  children,
  trailing,
  required,
  hint,
}: {
  label: string;
  children: ReactNode;
  trailing?: ReactNode;
  required?: boolean;
  hint?: string;
}) {
  return (
    <div className="block">
      <div className="mb-1.5 flex items-center justify-between gap-2">
        <div className="flex items-baseline gap-1.5">
          <span className="text-xs font-semibold text-ink">{label}</span>
          {required ? (
            <span className="text-2xs text-rose-500">*</span>
          ) : null}
          {hint ? (
            <span className="text-2xs text-ink-muted">{hint}</span>
          ) : null}
        </div>
        {trailing}
      </div>
      {children}
    </div>
  );
}
