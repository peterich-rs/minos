/**
 * Short, human path labels for desktop UI (transcript tools, session summary).
 *
 * Goals:
 * - Collapse home directories to `~` (`/Users/you/...` → `~/...`)
 * - Prefer the **tail** of long paths (workspace prefixes are usually identical)
 * - Keep `title` tooltips on full original paths at the call site
 */

export type FormatDisplayPathOptions = {
  /** Max path segments after optional `~/` or leading `/`. Default 6. */
  maxSegments?: number;
  /** Soft char budget; if still over after segment trim, force fewer segments. */
  maxChars?: number;
};

/** Detect Unix/Windows absolute or home-relative path-ish strings. */
export function looksLikeFilePath(s: string): boolean {
  const t = s.trim();
  if (!t || t.includes("\n") || t.length > 1024) return false;
  if (t === "~" || t.startsWith("~/") || t.startsWith("./") || t.startsWith("../")) {
    return true;
  }
  if (t.startsWith("/")) return true;
  // Windows: C:\… or C:/…
  if (/^[A-Za-z]:[\\/]/.test(t)) return true;
  // Relative with separators + likely filename
  if (
    (t.includes("/") || t.includes("\\")) &&
    (/\.[a-zA-Z0-9]{1,16}$/.test(t) || t.split(/[/\\]/).length >= 2)
  ) {
    return true;
  }
  return false;
}

/**
 * `/Users/name/...` → `~/...`, `/home/name/...` → `~/...`,
 * `C:/Users/name/...` → `~/...`. Already-`~/` paths pass through.
 */
export function collapseHomePrefix(path: string): string {
  const normalized = path.replace(/\\/g, "/").trim();
  if (!normalized) return path;
  if (normalized === "~" || normalized.startsWith("~/")) return normalized;

  // /Users/<user> or /home/<user>
  let m = normalized.match(/^\/(?:Users|home)\/[^/]+/);
  if (m) {
    const rest = normalized.slice(m[0].length);
    return rest ? `~${rest}` : "~";
  }

  // Windows C:/Users/<user>
  m = normalized.match(/^[A-Za-z]:\/Users\/[^/]+/i);
  if (m) {
    const rest = normalized.slice(m[0].length);
    return rest ? `~${rest}` : "~";
  }

  return normalized;
}

/**
 * Prefer trailing segments when a path is deep.
 * Examples:
 * - `/Users/me/code/repo/src/a.ts` → `~/code/repo/src/a.ts`
 * - very deep → `~/…/apps/desktop/src/a.ts`
 */
export function formatDisplayPath(path: string, opts: FormatDisplayPathOptions = {}): string {
  const maxSegments = opts.maxSegments ?? 6;
  const maxChars = opts.maxChars ?? 72;
  const raw = path.trim();
  if (!raw) return path;

  const p = collapseHomePrefix(raw.replace(/\\/g, "/"));

  let prefix = "";
  let body = p;
  if (p === "~") return "~";
  if (p.startsWith("~/")) {
    prefix = "~/";
    body = p.slice(2);
  } else if (p.startsWith("/")) {
    prefix = "/";
    body = p.slice(1);
  }

  const parts = body.split("/").filter(Boolean);
  if (parts.length === 0) return prefix || p;

  let keep = Math.max(1, maxSegments);
  let tail = parts.slice(-keep);
  let out = parts.length <= keep ? `${prefix}${parts.join("/")}` : `${prefix}…/${tail.join("/")}`;

  // Tighten further if still too long for a single-line tool header.
  while (out.length > maxChars && keep > 2) {
    keep -= 1;
    tail = parts.slice(-keep);
    out = parts.length <= keep ? `${prefix}${parts.join("/")}` : `${prefix}…/${tail.join("/")}`;
  }

  if (out.length > maxChars && tail.length > 0) {
    // Last resort: ellipsize the final segment's middle is harsh; prefer head of tail.
    const last = tail[tail.length - 1]!;
    const budget = Math.max(12, maxChars - (out.length - last.length));
    if (last.length > budget) {
      const head = Math.ceil(budget / 2) - 1;
      const end = Math.floor(budget / 2) - 1;
      const shortLast = `${last.slice(0, head)}…${last.slice(-end)}`;
      tail = [...tail.slice(0, -1), shortLast];
      out = parts.length <= keep ? `${prefix}${tail.join("/")}` : `${prefix}…/${tail.join("/")}`;
    }
  }

  return out;
}

/**
 * Format a tool header target: path-like → short path; otherwise collapse any
 * embedded home prefixes (e.g. commands that include absolute paths).
 */
export function formatToolTarget(target: string, opts?: FormatDisplayPathOptions): string {
  const t = target.trim();
  if (!t) return target;
  if (looksLikeFilePath(t) || t.startsWith("~/") || t.startsWith("/")) {
    return formatDisplayPath(t, opts);
  }
  // Collapse home prefixes inside free-form targets (e.g. long shell args).
  return t
    .replace(/\\/g, "/")
    .replace(/\/(?:Users|home)\/[^/\s"']+/g, "~")
    .replace(/[A-Za-z]:\/Users\/[^/\s"']+/gi, "~");
}
