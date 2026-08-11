#!/usr/bin/env node
/**
 * Soft file-size gate for apps/desktop/src (Buzz-inspired discipline).
 *
 * - WARN when a .ts/.tsx file exceeds WARN_LINES
 * - FAIL when a .ts/.tsx file exceeds HARD_LINES (unless allowlisted)
 *
 * Known oversized modules are listed in ALLOWLIST with a temporary higher cap
 * so the gate stays green while splits land in later waves. Do not grow the
 * allowlist without a plan to shrink the file.
 */

import { readdirSync, readFileSync, statSync } from "node:fs";
import { join, relative } from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = join(fileURLToPath(new URL("..", import.meta.url)), "src");
const WARN_LINES = 400;
const HARD_LINES = 800;

/** @type {Record<string, number>} path relative to src/ → temporary hard cap */
const ALLOWLIST = {
  // Agents page create/list/edit + cloud registry; split to AgentsView sections next wave.
  "features/agents/AgentsView.tsx": 1250,
  // WS connect / inbox digest / subscription fanout; split cloud-realtime lanes next wave.
  // Cap raised after Hub→Cloud identifier rename (hub-realtime @982→cloud-realtime ~1017).
  "shared/lib/cloud-realtime.ts": 1050,
  // Cloud REST helpers (auth + conversations + agents). sendConversationMessage removed;
  // further split to minos-cloud helpers planned (freeze above current LOC).
  "shared/lib/minos-cloud.ts": 1100,
  // Mock fixtures + Conversation types; extract mock-conversations.ts planned.
  "shared/lib/mock-data.ts": 850,
  // Outbox posts + per-lane worker share one module; split to im-outbox-worker.ts next.
  "shared/lib/im-cloud-sync.ts": 950,
  // Cloud IM bridge (lifecycle + mark-read + timeline merge). Split mark-read /
  // lifecycle helpers next; cap raised after Hub→Cloud rename (im-hub-bridge @784→~826).
  "shared/lib/im-cloud-bridge.ts": 850,
};

/**
 * @param {string} dir
 * @param {string[]} acc
 */
function walk(dir, acc = []) {
  for (const name of readdirSync(dir)) {
    const full = join(dir, name);
    const st = statSync(full);
    if (st.isDirectory()) {
      walk(full, acc);
      continue;
    }
    if (name.endsWith(".ts") || name.endsWith(".tsx")) {
      acc.push(full);
    }
  }
  return acc;
}

function countLines(path) {
  const text = readFileSync(path, "utf8");
  if (text.length === 0) return 0;
  // Count newlines; trailing content without final newline still counts as a line.
  let n = 0;
  for (let i = 0; i < text.length; i++) {
    if (text.charCodeAt(i) === 10) n++;
  }
  if (!text.endsWith("\n")) n++;
  return n;
}

const files = walk(ROOT);
const warnings = [];
const failures = [];

for (const full of files.sort()) {
  const rel = relative(ROOT, full).split("\\").join("/");
  const lines = countLines(full);
  const hard = ALLOWLIST[rel] ?? HARD_LINES;

  if (lines > hard) {
    failures.push({ rel, lines, hard, allowlisted: rel in ALLOWLIST });
  } else if (lines > WARN_LINES) {
    warnings.push({
      rel,
      lines,
      hard,
      allowlisted: rel in ALLOWLIST,
    });
  }
}

console.log(
  `check-file-sizes: scanned ${files.length} files under src/ (warn>${WARN_LINES}, hard>${HARD_LINES})`,
);

if (warnings.length > 0) {
  console.log("\nWarnings (over soft limit):");
  for (const w of warnings) {
    const tag = w.allowlisted ? " [allowlisted]" : "";
    console.log(`  WARN  ${w.rel}: ${w.lines} lines (hard ${w.hard})${tag}`);
  }
}

if (failures.length > 0) {
  console.log("\nFailures (over hard limit):");
  for (const f of failures) {
    const tag = f.allowlisted ? " [allowlisted]" : "";
    console.log(`  FAIL  ${f.rel}: ${f.lines} lines (hard ${f.hard})${tag}`);
  }
  console.error(
    `\ncheck-file-sizes: ${failures.length} file(s) exceed hard limit. Split or raise allowlist with a plan.`,
  );
  process.exit(1);
}

if (Object.keys(ALLOWLIST).length > 0) {
  console.log("\nAllowlisted oversized files (temporary caps):");
  for (const [rel, cap] of Object.entries(ALLOWLIST)) {
    const full = join(ROOT, rel);
    let lines = "?";
    try {
      lines = String(countLines(full));
    } catch {
      lines = "missing";
    }
    console.log(`  ${rel}: ${lines} / cap ${cap}`);
  }
}

console.log(
  `\ncheck-file-sizes: OK (${warnings.length} warning(s), ${failures.length} failure(s))`,
);
