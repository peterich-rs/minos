/**
 * Pure projection: daemon CLI / model catalog → Agents UI options.
 *
 * Capability facts must come from Rust (list_clis + list_models). This module
 * never invents runtimes or default effort ladders.
 */

/** Daemon / store CLI inventory row (capability fields from list_clis SSOT). */
export type RuntimeCliDescriptor = {
  agent: string;
  displayName: string;
  installed: boolean;
  status: string;
  path?: string | null;
  version?: string | null;
  supportsModelSelection: boolean;
  supportsReasoningEffort: boolean;
};

/** One model entry from `list_models` (honest efforts array). */
export type ModelCatalogEntry = {
  id: string;
  display_name: string;
  is_default: boolean;
  supported_reasoning_efforts: string[];
  default_reasoning_effort?: string | null;
};

/** Runtime option for pickers (derived from CLI inventory). */
export type RuntimeOption = {
  id: string;
  displayName: string;
  installed: boolean;
  status: string;
  supportsModelSelection: boolean;
  supportsReasoningEffort: boolean;
};

/**
 * Map daemon CLI descriptors → ordered runtime options for UI pickers.
 * Order follows daemon list_clis (domain AgentName::all()).
 */
export function runtimeOptionsFromClis(
  clis: readonly RuntimeCliDescriptor[],
): RuntimeOption[] {
  return clis.map((c) => ({
    id: c.agent,
    displayName: c.displayName.trim() || titleCaseAgent(c.agent),
    installed: c.installed,
    status: c.status,
    supportsModelSelection: c.supportsModelSelection,
    supportsReasoningEffort: c.supportsReasoningEffort,
  }));
}

/** Prefer first installed runtime; else first entry; else null. */
export function defaultRuntimeId(
  options: readonly RuntimeOption[],
): string | null {
  const installed = options.find((o) => o.installed);
  return installed?.id ?? options[0]?.id ?? null;
}

/**
 * Effort chips for the selected model.
 * Empty ⇒ UI must hide effort controls (no invented ladder).
 */
export function effortOptionsForModel(
  model: ModelCatalogEntry | null | undefined,
): string[] {
  if (!model) return [];
  const efforts = model.supported_reasoning_efforts;
  if (!Array.isArray(efforts) || efforts.length === 0) return [];
  return efforts.filter((e) => typeof e === "string" && e.length > 0);
}

/**
 * Whether the create form should show reasoning-effort UI.
 * Requires a non-empty model effort list (model is the honest SSOT for options).
 */
export function shouldShowEffortPicker(
  model: ModelCatalogEntry | null | undefined,
): boolean {
  return effortOptionsForModel(model).length > 0;
}

/** Initial effort value after model selection (empty when unsupported). */
export function defaultEffortForModel(
  model: ModelCatalogEntry | null | undefined,
): string {
  if (!model) return "";
  if (model.default_reasoning_effort?.trim()) {
    return model.default_reasoning_effort.trim();
  }
  const options = effortOptionsForModel(model);
  return options[0] ?? "";
}

function titleCaseAgent(agent: string): string {
  if (!agent) return agent;
  if (agent === "opencode") return "OpenCode";
  return agent.charAt(0).toUpperCase() + agent.slice(1);
}
