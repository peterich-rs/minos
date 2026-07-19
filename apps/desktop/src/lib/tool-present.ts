/**
 * Grok-style tool presentation helpers (mirrors TUI tool_kind / tool_summary).
 * Used by desktop transcript UI; Rust bridge emits bare targets + tool name.
 */

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

export function toolKindFromName(name: string): ToolKind {
  const n = name.toLowerCase();
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
  const s = summary.trim();
  if (s.startsWith("+")) {
    const rest = s.slice(1);
    const idx = rest.indexOf("/-");
    if (idx >= 0) {
      const add = Number(rest.slice(0, idx).trim());
      const del = Number(rest.slice(idx + 2).trim());
      if (Number.isFinite(add) && Number.isFinite(del)) {
        return { add, del };
      }
    }
  }
  if (s.startsWith("diff ")) {
    let add: number | undefined;
    let del: number | undefined;
    for (const part of s.slice(5).split(/\s+/)) {
      if (part.startsWith("+")) add = Number(part.slice(1));
      else if (part.startsWith("-")) del = Number(part.slice(1));
    }
    if (add !== undefined && del !== undefined && Number.isFinite(add) && Number.isFinite(del)) {
      return { add, del };
    }
  }
  return null;
}

export function isDiffLike(text: string): boolean {
  return (
    text.includes("diff --git") ||
    text.includes("\n@@") ||
    text.startsWith("@@") ||
    text.includes("*** Begin Patch") ||
    text.includes("*** Update File:") ||
    text.includes("*** Add File:") ||
    text.includes("*** Delete File:") ||
    text.includes("*** End Patch") ||
    text.split("\n").some((line) => line.startsWith("+++ ") || line.startsWith("--- "))
  );
}

export function countDiffLines(text: string): { add: number; del: number } {
  let add = 0;
  let del = 0;
  for (const line of text.split("\n")) {
    if (line.startsWith("+") && !line.startsWith("+++")) add += 1;
    else if (line.startsWith("-") && !line.startsWith("---")) del += 1;
  }
  return { add, del };
}

export type ToolHeaderModel = {
  verb: string;
  target: string;
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
export function buildToolHeader(opts: {
  toolName: string;
  target: string;
  kind: "tool" | "tool_result" | "tool_error";
  detail?: string | null;
}): ToolHeaderModel {
  const running = opts.kind === "tool";
  const failed = opts.kind === "tool_error";
  const kind = toolKindFromName(opts.toolName || "tool");
  const verb = toolHeaderVerb(kind, running);
  let target = (opts.target || "").trim();
  // Legacy bridge text like "Done read_file · …" — fall back to tool name.
  if (!target || /^(Running|Done|Failed)\s/i.test(target)) {
    target = opts.toolName || "…";
  }
  if (!target) target = "…";

  let diffstat: { add: number; del: number } | null = null;
  if (!running && !failed && opts.detail) {
    const parsed = parseDiffstat(opts.detail);
    if (parsed) {
      diffstat = parsed;
    } else if (isDiffLike(opts.detail)) {
      const counted = countDiffLines(opts.detail);
      if (counted.add > 0 || counted.del > 0) {
        diffstat = counted;
      }
    }
  }

  return { verb, target, running, failed, diffstat };
}

export function collapsedThinkingSummary(text: string, max = 100): string {
  const one = text.replace(/\s+/g, " ").trim();
  if (one.length <= max) return one;
  return `${one.slice(0, max)}…`;
}
