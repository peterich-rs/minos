import {
  useCallback,
  useEffect,
  useMemo,
  useState,
  type ReactNode,
} from "react";
import { useQueryClient } from "@tanstack/react-query";
import {
  CheckCircle2,
  CircleDashed,
  Loader2,
  Plus,
  RefreshCw,
  Trash2,
  X,
} from "lucide-react";
import { agentMeta, type AgentRuntime } from "@/shared/lib/mock-data";
import { useWorkspaceStore } from "@/store/workspace-store";
import { daemonApi } from "@/shared/lib/daemon";
import { validateProfileName } from "@/shared/lib/agent-route";
import { cn } from "@/shared/lib/utils";
import {
  useAgentProfilesQuery,
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

type AgentProfile = {
  id: string;
  name: string;
  description: string;
  runtime_agent: string;
  model: string;
  reasoning_effort: string;
  instructions?: string;
};

const fieldClass =
  "w-full rounded-xl border border-ink/10 bg-white px-3.5 py-2.5 text-sm text-ink shadow-sm outline-none transition placeholder:text-ink-muted/70 focus:border-ink/25 focus:ring-2 focus:ring-ink/10";

export function AgentsView() {
  const clis = useWorkspaceStore((s) => s.clis);
  const clisStatus = useWorkspaceStore((s) => s.clisStatus);
  const loadClis = useWorkspaceStore((s) => s.loadClis);
  const source = useWorkspaceStore((s) => s.source);
  const queryClient = useQueryClient();

  const profilesQuery = useAgentProfilesQuery();
  const profiles = (profilesQuery.data ?? []) as AgentProfile[];
  const profilesLoading = profilesQuery.isLoading || profilesQuery.isFetching;

  const [createOpen, setCreateOpen] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const loadProfiles = useCallback(async () => {
    await queryClient.invalidateQueries({ queryKey: queryKeys.agentProfiles });
  }, [queryClient]);

  useEffect(() => {
    if (source !== "daemon") return;
    void loadClis();
  }, [source, loadClis]);

  useEffect(() => {
    if (profilesQuery.error) {
      setError(
        profilesQuery.error instanceof Error
          ? profilesQuery.error.message
          : String(profilesQuery.error),
      );
    } else if (profilesQuery.isSuccess) {
      setError(null);
    }
  }, [profilesQuery.error, profilesQuery.isSuccess]);

  const phase = clisStatus.phase;

  return (
    <div className="flex min-h-0 flex-1 flex-col bg-surface">
      <header className="flex shrink-0 items-start justify-between gap-3 border-b border-ink/5 px-6 py-5">
        <div>
          <h1 className="text-xl font-semibold tracking-tight text-ink">
            Agents
          </h1>
          <p className="mt-1 max-w-xl text-sm text-ink-muted">
            Local CLI runtimes on this Host, plus personalized agents with a
            fixed model and optional instructions. Chat always happens inside a
            Project conversation.
          </p>
        </div>
        <button
          type="button"
          onClick={() => setCreateOpen(true)}
          className="inline-flex shrink-0 items-center gap-1.5 rounded-xl bg-ink px-3.5 py-2 text-xs font-semibold text-white shadow-sm hover:bg-ink/90"
        >
          <Plus className="h-3.5 w-3.5" />
          Create agent
        </button>
      </header>

      {error ? (
        <p className="px-6 pt-3 text-xs text-rose-600">{error}</p>
      ) : null}

      <div className="scrollbar-thin min-h-0 flex-1 space-y-8 overflow-y-auto p-5">
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
                className="rounded-xl bg-ink px-3 py-1.5 text-xs font-semibold text-white"
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
                  color: "bg-stone-100 text-stone-700",
                };
                return (
                  <div
                    key={rt.agent}
                    className="rounded-2xl border border-ink/5 bg-white p-4 shadow-sm"
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
                        Install the CLI and re-detect to use @{rt.agent} in
                        conversations.
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
              Personalized agents
            </h2>
            {profilesLoading ? (
              <Loader2 className="h-3.5 w-3.5 animate-spin text-ink-muted" />
            ) : null}
          </div>
          {profiles.length === 0 ? (
            <p className="rounded-2xl border border-dashed border-ink/10 bg-white/70 px-4 py-10 text-center text-sm leading-relaxed text-ink-muted">
              No personalized agents yet.
              <br />
              Create one to pin runtime, model, effort, and system instructions.
            </p>
          ) : (
            <div className="grid content-start gap-3 sm:grid-cols-2 lg:grid-cols-3">
              {profiles.map((p) => {
                const runtime = p.runtime_agent as AgentRuntime;
                const meta = agentMeta[runtime] ?? {
                  label: p.runtime_agent,
                  color: "bg-stone-100 text-stone-700",
                };
                return (
                  <div
                    key={p.id}
                    className="rounded-2xl border border-ink/5 bg-white p-4 shadow-sm"
                  >
                    <div className="flex items-start justify-between gap-2">
                      <div className="min-w-0">
                        <div className="truncate text-sm font-semibold text-ink">
                          {p.name}
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
                            {p.model}
                          </span>
                        </div>
                      </div>
                      <button
                        type="button"
                        title="Delete profile"
                        onClick={() => {
                          void (async () => {
                            try {
                              await daemonApi.deleteAgentProfile(p.id);
                              await loadProfiles();
                            } catch (e) {
                              setError(
                                e instanceof Error ? e.message : String(e),
                              );
                            }
                          })();
                        }}
                        className="rounded-lg p-1.5 text-ink-muted hover:bg-rose-50 hover:text-rose-700"
                      >
                        <Trash2 className="h-3.5 w-3.5" />
                      </button>
                    </div>
                    {p.description ? (
                      <p className="mt-2 line-clamp-2 text-xs text-ink-muted">
                        {p.description}
                      </p>
                    ) : null}
                    <dl className="mt-3 space-y-1 text-xs">
                      {p.reasoning_effort ? (
                        <div className="flex justify-between gap-2">
                          <dt className="text-ink-muted">Effort</dt>
                          <dd className="font-medium capitalize text-ink">
                            {p.reasoning_effort}
                          </dd>
                        </div>
                      ) : null}
                      {p.instructions?.trim() ? (
                        <div className="flex justify-between gap-2">
                          <dt className="text-ink-muted">Instructions</dt>
                          <dd className="font-medium text-ink">Custom</dd>
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
          onClose={() => setCreateOpen(false)}
          onCreated={async () => {
            setCreateOpen(false);
            await loadProfiles();
          }}
        />
      ) : null}
    </div>
  );
}

function CreateAgentDialog({
  clis,
  onClose,
  onCreated,
}: {
  clis: RuntimeCliDescriptor[];
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
      await daemonApi.createAgentProfile({
        name: name.trim(),
        description: description.trim(),
        runtimeAgent: runtime,
        model: resolvedModel,
        reasoningEffort: showEffort ? effort.trim() : "",
        instructions: instructions.trim(),
      });
      await onCreated();
    } catch (e) {
      setErr(e instanceof Error ? e.message : String(e));
    } finally {
      setSaving(false);
    }
  };

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-ink/40 p-4"
      role="dialog"
      aria-modal="true"
      aria-label="Create agent"
      // Close only when the scrim itself is the event target (not children).
      // Do not use document-level listeners or stopPropagation on content —
      // those patterns swallow the subsequent `click` in WKWebView (feels like
      // every control needs a double-click).
      onPointerDown={(e) => {
        if (e.target === e.currentTarget) onClose();
      }}
    >
      <div className="flex max-h-[min(92vh,760px)] w-full max-w-[440px] flex-col overflow-hidden rounded-2xl border border-ink/8 bg-[#f7f3ec] shadow-2xl">
        <header className="flex shrink-0 items-center justify-between px-5 pb-3 pt-5">
          <div>
            <h2 className="text-base font-semibold tracking-tight text-ink">
              Create agent
            </h2>
            <p className="mt-0.5 text-xs text-ink-muted">
              Model and effort stay fixed after create.
            </p>
          </div>
          <button
            type="button"
            onClick={onClose}
            className="rounded-xl p-2 text-ink-muted hover:bg-white/80 hover:text-ink"
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

          <Field label="Description" hint="optional">
            <textarea
              value={description}
              onChange={(e) => setDescription(e.target.value)}
              rows={2}
              maxLength={3000}
              placeholder="Short label for this agent…"
              className={cn(fieldClass, "resize-none")}
            />
          </Field>

          <Field label="Runtime">
            <div className="grid grid-cols-2 gap-2 sm:grid-cols-3">
              {runtimeOptions.map((opt) => {
                const meta = agentMeta[opt.id as AgentRuntime] ?? {
                  label: opt.displayName,
                  color: "bg-stone-100 text-stone-700",
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
                        ? "border-ink/30 bg-white shadow-sm ring-2 ring-ink/10"
                        : "border-ink/8 bg-white/60 hover:border-ink/15 hover:bg-white",
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
                className="inline-flex items-center gap-1 rounded-lg px-1.5 py-0.5 text-2xs text-ink-muted hover:bg-white hover:text-ink"
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

          <Field
            label="Instructions"
            hint="system prompt / developer notes"
          >
            <textarea
              value={instructions}
              onChange={(e) => setInstructions(e.target.value)}
              rows={4}
              maxLength={12000}
              placeholder="Optional extra system prompt appended when this agent starts (role, constraints, style)…"
              className={cn(fieldClass, "resize-y min-h-[96px]")}
            />
            <p className="mt-1.5 text-2xs leading-snug text-ink-muted">
              Combined with Minos teamwork guidance at session start. Fixed for
              the life of this profile.
            </p>
          </Field>

          {err ? <p className="text-xs text-rose-600">{err}</p> : null}
        </div>

        <footer className="flex shrink-0 justify-end gap-2 border-t border-ink/5 bg-[#f0ebe3]/60 px-5 py-3.5">
          <button
            type="button"
            onClick={onClose}
            className="rounded-xl border border-ink/10 bg-white px-3.5 py-2 text-xs font-medium text-ink-muted hover:bg-surface"
          >
            Cancel
          </button>
          <button
            type="button"
            disabled={saving || loadingModels}
            onClick={() => void onSubmit()}
            className="rounded-xl bg-ink px-4 py-2 text-xs font-semibold text-white shadow-sm hover:bg-ink/90 disabled:opacity-50"
          >
            {saving ? "Creating…" : "Create agent"}
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
          ? "bg-ink text-white shadow-sm"
          : "bg-white text-ink-secondary ring-1 ring-ink/10 hover:ring-ink/20",
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
