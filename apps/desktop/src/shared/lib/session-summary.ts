/**
 * Derive a session summary (files touched + line stats) from transcript items.
 * v1: no token usage — that is not projected uniformly across CLIs yet.
 *
 * Sources (already in Desktop transcript DTOs):
 * - kind `tool` / `tool_result` / `tool_error`
 * - title = tool name, text = bare path/cmd, detail = args or output
 */

import type { TranscriptItem } from "./daemon.ts";
import {
  countDiffLines,
  isDiffLike,
  parseDiffstat,
  toolKindFromName,
} from "./tool-present.ts";

export type FileChangeEntry = {
  path: string;
  add: number;
  del: number;
  /** True when at least one edit tool completed without error for this path. */
  ok: boolean;
  /** True when any edit for this path failed. */
  failed: boolean;
};

export type SessionSummary = {
  files: FileChangeEntry[];
  totalAdd: number;
  totalDel: number;
  /** Edit tool calls still in flight (kind === tool). */
  pendingEdits: number;
  toolCallCount: number;
  editCallCount: number;
};

type Acc = {
  add: number;
  del: number;
  ok: boolean;
  failed: boolean;
};

function emptySummary(): SessionSummary {
  return {
    files: [],
    totalAdd: 0,
    totalDel: 0,
    pendingEdits: 0,
    toolCallCount: 0,
    editCallCount: 0,
  };
}

function lookLikePath(s: string): boolean {
  const t = s.trim();
  if (!t || t.length > 512) return false;
  if (t.includes("\n")) return false;
  // Reject obvious non-paths (commands, sentences).
  if (/\s{2,}/.test(t)) return false;
  if (t.startsWith("Running ") || t.startsWith("Done ") || t.startsWith("Failed ")) {
    return false;
  }
  // Path-ish: slash, backslash, or extension.
  return (
    t.includes("/") ||
    t.includes("\\") ||
    /\.[a-zA-Z0-9]{1,12}$/.test(t) ||
    t.startsWith("./") ||
    t.startsWith("../")
  );
}

/** Prefer display-friendly relative tail when path is absolute. */
export function displayPath(path: string, maxSegments = 4): string {
  const normalized = path.replace(/\\/g, "/").trim();
  if (!normalized) return path;
  const parts = normalized.split("/").filter(Boolean);
  if (parts.length <= maxSegments) return normalized.startsWith("/")
    ? `/${parts.join("/")}`
    : parts.join("/");
  return `…/${parts.slice(-maxSegments).join("/")}`;
}

/**
 * Parse multi-file patch bodies (apply_patch / unified diff) into per-path stats.
 */
export function fileStatsFromPatchBody(
  body: string,
): Array<{ path: string; add: number; del: number }> {
  const out: Array<{ path: string; add: number; del: number }> = [];
  if (!body.trim()) return out;

  // *** Update File: path / *** Add File: / *** Delete File:
  const applyRe =
    /\*\*\*\s+(Update|Add|Delete)\s+File:\s*(.+)$/gm;
  const applyHits: Array<{ path: string; index: number; kind: string }> = [];
  let m: RegExpExecArray | null;
  while ((m = applyRe.exec(body)) !== null) {
    applyHits.push({
      kind: m[1]!,
      path: m[2]!.trim(),
      index: m.index,
    });
  }
  if (applyHits.length > 0) {
    for (let i = 0; i < applyHits.length; i++) {
      const hit = applyHits[i]!;
      const end = applyHits[i + 1]?.index ?? body.length;
      const chunk = body.slice(hit.index, end);
      const counted = countDiffLines(chunk);
      // Delete with no hunks → at least mark as del-ish unknown: count 0/0 but keep path.
      out.push({ path: hit.path, add: counted.add, del: counted.del });
    }
    return out;
  }

  // diff --git a/path b/path
  const gitRe = /^diff --git a\/(.+?) b\/(.+)$/gm;
  const gitHits: Array<{ path: string; index: number }> = [];
  while ((m = gitRe.exec(body)) !== null) {
    gitHits.push({ path: m[2]!.trim(), index: m.index });
  }
  if (gitHits.length > 0) {
    for (let i = 0; i < gitHits.length; i++) {
      const hit = gitHits[i]!;
      const end = gitHits[i + 1]?.index ?? body.length;
      const chunk = body.slice(hit.index, end);
      const counted = countDiffLines(chunk);
      out.push({ path: hit.path, add: counted.add, del: counted.del });
    }
    return out;
  }

  // --- a/foo\n+++ b/foo  single-file unified
  const plusPlus = body.match(/^\+\+\+\s+(?:b\/)?(.+)$/m);
  if (plusPlus?.[1] && plusPlus[1] !== "/dev/null") {
    const counted = countDiffLines(body);
    if (counted.add > 0 || counted.del > 0) {
      out.push({ path: plusPlus[1].trim(), add: counted.add, del: counted.del });
    }
  }

  return out;
}

function mergeInto(
  map: Map<string, Acc>,
  path: string,
  add: number,
  del: number,
  flags: { ok?: boolean; failed?: boolean },
) {
  const key = path.trim();
  if (!key) return;
  const prev = map.get(key) ?? { add: 0, del: 0, ok: false, failed: false };
  prev.add += add;
  prev.del += del;
  if (flags.ok) prev.ok = true;
  if (flags.failed) prev.failed = true;
  map.set(key, prev);
}

/**
 * Aggregate file modifications from a session transcript.
 * Multiple edits to the same path sum +/− counts.
 */
export function summarizeSessionFromTranscript(
  items: readonly TranscriptItem[],
): SessionSummary {
  if (items.length === 0) return emptySummary();

  const byPath = new Map<string, Acc>();
  let pendingEdits = 0;
  let toolCallCount = 0;
  let editCallCount = 0;

  for (const item of items) {
    if (
      item.kind !== "tool" &&
      item.kind !== "tool_result" &&
      item.kind !== "tool_error"
    ) {
      continue;
    }
    toolCallCount += 1;
    const toolName = item.title ?? "";
    const kind = toolKindFromName(toolName);
    // Codex thread items often use name "fileChange"; treat as edit.
    if (kind !== "edit" && !/file\s*change/i.test(toolName)) continue;
    editCallCount += 1;

    if (item.kind === "tool") {
      pendingEdits += 1;
      const path = (item.text ?? "").trim();
      if (lookLikePath(path)) {
        mergeInto(byPath, path, 0, 0, {});
      }
      continue;
    }

    const failed = item.kind === "tool_error";
    const detail = item.detail ?? "";
    const target = (item.text ?? "").trim();

    // Prefer multi-file patch parse from detail.
    const fromPatch = fileStatsFromPatchBody(detail);
    if (fromPatch.length > 0) {
      for (const f of fromPatch) {
        mergeInto(byPath, f.path, f.add, f.del, {
          ok: !failed,
          failed,
        });
      }
      continue;
    }

    let add = 0;
    let del = 0;
    const parsed = parseDiffstat(detail);
    if (parsed) {
      add = parsed.add;
      del = parsed.del;
    } else if (isDiffLike(detail)) {
      const counted = countDiffLines(detail);
      add = counted.add;
      del = counted.del;
    }

    if (lookLikePath(target)) {
      mergeInto(byPath, target, add, del, { ok: !failed, failed });
    } else if (add > 0 || del > 0) {
      // Stats without a clear path — keep under a synthetic bucket only if useful.
      mergeInto(byPath, toolName || "edited file", add, del, {
        ok: !failed,
        failed,
      });
    }
  }

  const files: FileChangeEntry[] = [...byPath.entries()]
    .map(([path, acc]) => ({
      path,
      add: acc.add,
      del: acc.del,
      ok: acc.ok,
      failed: acc.failed,
    }))
    .sort((a, b) => {
      // Most churn first, then path.
      const score = b.add + b.del - (a.add + a.del);
      if (score !== 0) return score;
      return a.path.localeCompare(b.path);
    });

  const totalAdd = files.reduce((n, f) => n + f.add, 0);
  const totalDel = files.reduce((n, f) => n + f.del, 0);

  return {
    files,
    totalAdd,
    totalDel,
    pendingEdits,
    toolCallCount,
    editCallCount,
  };
}

/** Format like `path -23 +46`. */
export function formatFileChangeLine(entry: FileChangeEntry): string {
  const path = displayPath(entry.path);
  const parts = [path];
  if (entry.del > 0) parts.push(`-${entry.del}`);
  if (entry.add > 0) parts.push(`+${entry.add}`);
  if (entry.del === 0 && entry.add === 0) {
    parts.push(entry.failed ? "failed" : entry.ok ? "touched" : "…");
  }
  return parts.join(" ");
}
