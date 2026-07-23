/**
 * Unified / apply_patch line classifier for session transcript expand.
 * Agent-agnostic: Codex apply_patch, Grok search_replace, etc.
 */

export type DiffLineKind =
  | "add"
  | "del"
  | "hunk"
  | "meta"
  | "file"
  | "context"
  | "ellipsis";

export type DiffLine = {
  kind: DiffLineKind;
  text: string;
  no: number;
};

/**
 * Detect real patches only — must NOT match tool args JSON or markdown.
 * False positives used to render huge "diff" tables and break transcript scroll.
 */
export function isDiffLike(text: string): boolean {
  const t = text.trim();
  if (!t) return false;
  // Tool args / structured payloads are never patches.
  if (t.startsWith("{") || t.startsWith("[")) return false;

  if (t.includes("diff --git")) return true;
  if (t.includes("*** Begin Patch") || t.includes("*** Update File:")) return true;
  if (t.includes("*** Add File:") || t.includes("*** Delete File:")) return true;
  // Unified hunk header. Accept both `@@ … @@` and a bare `@@` token so the
  // classifier matches the TUI/Tauri Rust `is_diff_like` (Codex apply_patch
  // sometimes emits a lone `@@` without a closing marker). JSON tool args are
  // already rejected above, so this is safe from false positives.
  if (/(^|\n)@@/.test(t)) return true;
  // File headers from unified diff.
  if (/(^|\n)--- [^\n]+\n\+\+\+ /.test(t)) return true;
  return false;
}

export function parseDiffLines(
  raw: string,
  opts?: { head?: number; tail?: number },
): DiffLine[] {
  const head = opts?.head ?? 80;
  const tail = opts?.tail ?? 40;
  const src = raw.replace(/\r\n/g, "\n").replace(/\r/g, "\n");
  const lines = src.split("\n");
  if (lines.length > 0 && lines[lines.length - 1] === "") {
    lines.pop();
  }

  const classified: DiffLine[] = lines.map((text, i) => ({
    kind: classifyDiffLine(text),
    text,
    no: i + 1,
  }));

  if (classified.length <= head + tail) {
    return classified;
  }

  const out: DiffLine[] = [];
  out.push(...classified.slice(0, head));
  out.push({
    kind: "ellipsis",
    text: `… ${classified.length - head - tail} lines omitted …`,
    no: 0,
  });
  out.push(...classified.slice(classified.length - tail));
  return out.map((line, i) => ({ ...line, no: i + 1 }));
}

function classifyDiffLine(line: string): DiffLineKind {
  if (
    line.startsWith("diff --git") ||
    line.startsWith("index ") ||
    line.startsWith("similarity index") ||
    line.startsWith("rename from") ||
    line.startsWith("rename to") ||
    line.startsWith("new file mode") ||
    line.startsWith("deleted file mode")
  ) {
    return "meta";
  }
  if (
    line.startsWith("--- ") ||
    line.startsWith("+++ ") ||
    line.startsWith("*** Begin Patch") ||
    line.startsWith("*** End Patch") ||
    line.startsWith("*** Update File:") ||
    line.startsWith("*** Add File:") ||
    line.startsWith("*** Delete File:") ||
    line.startsWith("*** Move to:") ||
    line.startsWith("*** End of File")
  ) {
    return "file";
  }
  if (line.startsWith("@@")) {
    return "hunk";
  }
  if (line.startsWith("+") && !line.startsWith("+++")) {
    return "add";
  }
  if (line.startsWith("-") && !line.startsWith("---")) {
    return "del";
  }
  return "context";
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
    if (
      add !== undefined &&
      del !== undefined &&
      Number.isFinite(add) &&
      Number.isFinite(del)
    ) {
      return { add, del };
    }
  }
  return null;
}
