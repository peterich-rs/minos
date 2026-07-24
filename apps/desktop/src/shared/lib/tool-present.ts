/**
 * Grok-style tool presentation helpers (mirrors TUI tool_kind / tool_summary).
 * Used by desktop transcript UI; Rust bridge emits bare targets + tool name.
 */

import { stripAnsiEscapes } from "./ansi.ts";
import {
  countDiffLines,
  isDiffLike,
  parseDiffstat as parseDiffstatFromLine,
} from "./diff-view.ts";
import { formatToolTarget } from "./display-path.ts";

export { isDiffLike, countDiffLines } from "./diff-view.ts";
export { stripAnsiEscapes } from "./ansi.ts";
export {
  formatDisplayPath,
  formatToolTarget,
  looksLikeFilePath,
} from "./display-path.ts";

export type ToolKind =
  | "read"
  | "edit"
  | "execute"
  | "search"
  | "list"
  | "web_fetch"
  | "web_search"
  | "skill"
  | "other";

const KIND_TOKENS: Record<string, ToolKind> = {
  read: "read",
  read_file: "read",
  readfile: "read",
  cat: "read",
  edit: "edit",
  write: "edit",
  diff: "edit",
  search_replace: "edit",
  apply_patch: "edit",
  str_replace: "edit",
  execute: "execute",
  terminal: "execute",
  bash: "execute",
  shell: "execute",
  run: "execute",
  command: "execute",
  search: "search",
  grep: "search",
  glob: "search",
  find: "search",
  rg: "search",
  list: "list",
  list_dir: "list",
  listdir: "list",
  list_directory: "list",
  ls: "list",
  web_fetch: "web_fetch",
  webfetch: "web_fetch",
  fetch: "web_fetch",
  web_search: "web_search",
  websearch: "web_search",
  skill: "skill",
  other: "other",
};

/** Subject after unified translator `"kind: subject"` name, else full name. */
export function toolSubjectFromName(name: string): string {
  const idx = name.indexOf(":");
  if (idx <= 0) return name.trim();
  const token = name.slice(0, idx).trim().toLowerCase();
  if (KIND_TOKENS[token]) {
    const subject = name.slice(idx + 1).trim();
    if (subject) return subject;
  }
  return name.trim();
}

/**
 * Classify tool from unified `ToolCallPlaced.name` (any agent after translator).
 * Prefer leading kind token (`read: path`) so subject text cannot mis-route.
 */
export function toolKindFromName(name: string): ToolKind {
  const n = name.toLowerCase();
  const colon = n.indexOf(":");
  if (colon > 0) {
    const token = n.slice(0, colon).trim();
    if (KIND_TOKENS[token]) return KIND_TOKENS[token];
  }
  if (KIND_TOKENS[n.trim()]) return KIND_TOKENS[n.trim()];

  if (n.includes("skill")) return "skill";
  if (n.includes("web_search") || n === "websearch") return "web_search";
  if (n.includes("web_fetch") || n.includes("webfetch") || n === "fetch") {
    return "web_fetch";
  }
  if (
    n.includes("list_dir") ||
    n.includes("listdir") ||
    n.includes("list_directory") ||
    n === "ls" ||
    n.includes("glob_file")
  ) {
    return "list";
  }
  // Edit before search: names like `search_replace` contain "search".
  if (
    n.includes("write") ||
    n.includes("edit") ||
    n.includes("apply_patch") ||
    n.includes("str_replace") ||
    n.includes("search_replace") ||
    n.includes("create_file") ||
    n.includes("delete_file") ||
    n.includes("filechange") ||
    n.includes("file_change")
  ) {
    return "edit";
  }
  if (
    n.includes("grep") ||
    n.includes("search") ||
    n.includes("glob") ||
    n.includes("find") ||
    n.includes("rg")
  ) {
    return "search";
  }
  if (n.includes("read") || n === "cat" || n.endsWith("_read")) return "read";
  if (
    n.includes("bash") ||
    n.includes("shell") ||
    n.includes("exec") ||
    n.includes("command") ||
    n === "run_terminal_command" ||
    n === "run"
  ) {
    return "execute";
  }
  return "other";
}

export function toolHeaderVerb(kind: ToolKind, running: boolean): string {
  if (kind === "skill") return "Skill";
  const pairs: Record<ToolKind, [string, string]> = {
    read: ["Read", "Reading"],
    skill: ["Skill", "Skill"],
    search: ["Searched", "Searching"],
    web_search: ["Searched", "Searching"],
    list: ["Listed", "Listing"],
    web_fetch: ["Fetched", "Fetching"],
    edit: ["Edited", "Editing"],
    execute: ["Ran", "Running"],
    other: ["Ran", "Running"],
  };
  const [past, present] = pairs[kind];
  return running ? present : past;
}

export function parseDiffstat(
  summary: string,
): { add: number; del: number } | null {
  return parseDiffstatFromLine(summary);
}

export type ToolHeaderModel = {
  verb: string;
  /** Short label for the row (home → `~`, tail-preferred). */
  target: string;
  /** Original target for `title` tooltip. */
  targetFull: string;
  toolKind: ToolKind;
  running: boolean;
  failed: boolean;
  diffstat: { add: number; del: number } | null;
};

/**
 * Build a Grok-style tool header from transcript fields.
 * - title: tool name
 * - text: bare target (path/cmd) from the bridge
 * - detail: args (running) or output (done)
 */
/** Markup / XML first lines must never become the tool header target. */
export function isMarkupishToolLine(line: string): boolean {
  const t = line.trimStart();
  return (
    t.startsWith("<") &&
    (t.startsWith("<task") ||
      t.startsWith("<path") ||
      t.startsWith("<type") ||
      t.startsWith("<content") ||
      t.startsWith("<tool") ||
      t.startsWith("<?xml") ||
      t.includes("<task id="))
  );
}

export function buildToolHeader(opts: {
  toolName: string;
  target: string;
  kind: "tool" | "tool_result" | "tool_error";
  detail?: string | null;
}): ToolHeaderModel {
  const running = opts.kind === "tool";
  const failed = opts.kind === "tool_error";
  const toolKind = toolKindFromName(opts.toolName || "tool");
  const verb = toolHeaderVerb(toolKind, running);
  const toolName = (opts.toolName || "tool").trim();
  let target = (opts.target || "").trim();
  // Legacy bridge text like "Done read_file · …" — fall back to subject.
  if (!target || /^(Running|Done|Failed)\s/i.test(target)) {
    target = toolSubjectFromName(toolName) || "";
  }
  // Avoid "Read read: path" when target still carries a kind prefix.
  target = toolSubjectFromName(target) || target;
  // Ban "Reading read" and raw XML titles (`<path>…`, `<task id=…>`).
  if (
    !target ||
    isMarkupishToolLine(target) ||
    target.toLowerCase() === toolName.toLowerCase() ||
    target.toLowerCase() === toolSubjectFromName(toolName).toLowerCase()
  ) {
    target = "…";
  }

  const targetFull = target;
  // Paths: `~/…` + prefer trailing segments so rows stay scannable.
  if (target !== "…") {
    target = formatToolTarget(target);
  }

  let diffstat: { add: number; del: number } | null = null;
  if (!running && !failed && opts.detail) {
    const detail = stripAnsiEscapes(opts.detail);
    const parsed = parseDiffstat(detail);
    if (parsed) {
      diffstat = parsed;
    } else if (isDiffLike(detail)) {
      const counted = countDiffLines(detail);
      if (counted.add > 0 || counted.del > 0) {
        diffstat = counted;
      }
    }
  }

  return { verb, target, targetFull, toolKind, running, failed, diffstat };
}

/** Clean tool body text for expanded transcript rows (strip SGR colors). */
export function displayToolDetail(detail: string | null | undefined): string {
  if (!detail) return "";
  return stripAnsiEscapes(detail);
}

export function collapsedThinkingSummary(text: string, max = 100): string {
  const one = text.replace(/\s+/g, " ").trim();
  if (one.length <= max) return one;
  return `${one.slice(0, max)}…`;
}
