import assert from "node:assert/strict";
import { describe, it } from "node:test";
import {
  defaultEffortForModel,
  defaultRuntimeId,
  effortOptionsForModel,
  runtimeOptionsFromClis,
  shouldShowEffortPicker,
  type ModelCatalogEntry,
  type RuntimeCliDescriptor,
} from "./agentConfigProjection.ts";

const sampleClis: RuntimeCliDescriptor[] = [
  {
    agent: "codex",
    displayName: "Codex",
    installed: true,
    status: "ok",
    supportsModelSelection: true,
    supportsReasoningEffort: true,
  },
  {
    agent: "claude",
    displayName: "Claude",
    installed: false,
    status: "missing",
    supportsModelSelection: true,
    supportsReasoningEffort: false,
  },
  {
    agent: "grok",
    displayName: "Grok",
    installed: true,
    status: "ok",
    supportsModelSelection: true,
    supportsReasoningEffort: true,
  },
];

describe("runtimeOptionsFromClis", () => {
  it("projects daemon rows without inventing runtimes", () => {
    const opts = runtimeOptionsFromClis(sampleClis);
    assert.equal(opts.length, 3);
    assert.deepEqual(
      opts.map((o) => o.id),
      ["codex", "claude", "grok"],
    );
    assert.equal(opts[0]!.supportsReasoningEffort, true);
    assert.equal(opts[1]!.supportsReasoningEffort, false);
    assert.equal(opts[1]!.installed, false);
  });

  it("uses empty displayName fallback only when displayName is blank", () => {
    const opts = runtimeOptionsFromClis([
      {
        agent: "opencode",
        displayName: "",
        installed: true,
        status: "ok",
        supportsModelSelection: true,
        supportsReasoningEffort: false,
      },
    ]);
    assert.equal(opts[0]!.displayName, "OpenCode");
    assert.equal(opts[0]!.supportsReasoningEffort, false);
  });
});

describe("defaultRuntimeId", () => {
  it("prefers first installed", () => {
    assert.equal(defaultRuntimeId(runtimeOptionsFromClis(sampleClis)), "codex");
  });

  it("returns null for empty", () => {
    assert.equal(defaultRuntimeId([]), null);
  });
});

describe("effortOptionsForModel", () => {
  it("returns empty when model has no efforts (no invented ladder)", () => {
    const model: ModelCatalogEntry = {
      id: "sonnet",
      display_name: "Sonnet",
      is_default: true,
      supported_reasoning_efforts: [],
    };
    assert.deepEqual(effortOptionsForModel(model), []);
    assert.equal(shouldShowEffortPicker(model), false);
    assert.equal(defaultEffortForModel(model), "");
  });

  it("returns empty for null/undefined model", () => {
    assert.deepEqual(effortOptionsForModel(null), []);
    assert.deepEqual(effortOptionsForModel(undefined), []);
    assert.equal(shouldShowEffortPicker(undefined), false);
  });

  it("returns honest model efforts only", () => {
    const model: ModelCatalogEntry = {
      id: "gpt-5.4",
      display_name: "GPT-5.4",
      is_default: true,
      supported_reasoning_efforts: ["low", "medium", "high", "xhigh"],
      default_reasoning_effort: "medium",
    };
    assert.deepEqual(effortOptionsForModel(model), [
      "low",
      "medium",
      "high",
      "xhigh",
    ]);
    assert.equal(shouldShowEffortPicker(model), true);
    assert.equal(defaultEffortForModel(model), "medium");
  });

  it("uses first effort when default_reasoning_effort missing", () => {
    const model: ModelCatalogEntry = {
      id: "m",
      display_name: "M",
      is_default: false,
      supported_reasoning_efforts: ["low", "high"],
    };
    assert.equal(defaultEffortForModel(model), "low");
  });
});
