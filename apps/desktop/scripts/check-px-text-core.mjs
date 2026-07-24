import { promises as fs } from "node:fs";
import path from "node:path";

/**
 * Shared "no hardcoded px text size" guard (Buzz-inspired).
 *
 * Zoom (Cmd +/-) scales the root <html> font-size, so only rem-based text
 * scales. Hardcoded `text-[15px]` / `font-size: 15px` freeze against zoom.
 *
 * Flags:
 *   - Tailwind arbitrary text-size utilities: `text-[NNpx|rem|em]`
 *   - CSS `font-size: NNpx`
 *
 * Existing debt is frozen in an allowlist (`relativePath:matchedLiteral`).
 * New hits fail the gate; remove allowlist rows when converting to tokens.
 */

const TEXT_ARBITRARY_RE = /\btext-\[\d+(?:\.\d+)?(?:px|rem|em)\]/g;
const FONT_SIZE_PX_RE = /(?<!-)\bfont-size:\s*\d+(?:\.\d+)?px/g;

async function walkFiles(directory) {
  const entries = await fs.readdir(directory, { withFileTypes: true });
  const files = await Promise.all(
    entries.map(async (entry) => {
      const fullPath = path.join(directory, entry.name);
      if (entry.isDirectory()) {
        return walkFiles(fullPath);
      }
      return [fullPath];
    }),
  );
  return files.flat();
}

/**
 * @param {object} options
 * @param {string} options.projectRoot
 * @param {Array<{root: string, extensions: Set<string>}>} options.rules
 * @param {string} options.label
 * @param {Set<string>} [options.overrides]
 * @param {string} options.scriptPath
 */
export async function runPxTextCheck({
  projectRoot,
  rules,
  label,
  overrides = new Set(),
  scriptPath,
}) {
  const candidateFiles = (
    await Promise.all(
      rules.map((rule) => {
        const dir = path.join(projectRoot, rule.root);
        return fs
          .access(dir)
          .then(() => walkFiles(dir))
          .catch(() => []);
      }),
    )
  ).flat();

  const violations = [];
  const seenAllowlisted = new Set();

  for (const filePath of candidateFiles) {
    const relativePath = path.relative(projectRoot, filePath).split(path.sep).join("/");
    const rule = rules.find((r) =>
      relativePath === r.root || relativePath.startsWith(`${r.root}/`),
    );
    if (!rule) continue;
    if (!rule.extensions.has(path.extname(relativePath))) continue;

    const content = await fs.readFile(filePath, "utf8");
    const lines = content.split(/\r?\n/);
    lines.forEach((line, index) => {
      const lineNumber = index + 1;
      const matches = [
        ...(line.match(TEXT_ARBITRARY_RE) ?? []),
        ...(line.match(FONT_SIZE_PX_RE) ?? []),
      ];
      for (const match of matches) {
        const key = `${relativePath}:${match}`;
        if (overrides.has(key)) {
          seenAllowlisted.add(key);
          continue;
        }
        violations.push({ relativePath, lineNumber, match });
      }
    });
  }

  const unused = [...overrides].filter((k) => !seenAllowlisted.has(k)).sort();

  if (violations.length > 0) {
    console.error(`${label} px-text check failed (${violations.length} new):`);
    for (const v of violations) {
      console.error(`- ${v.relativePath}:${v.lineNumber}: ${v.match}`);
    }
    console.error(
      "Use a rem-based Tailwind token (text-base / text-sm / text-xs / text-2xs). " +
        "If genuinely decorative, add `relativePath:matchedLiteral` to the allowlist in " +
        `\`${scriptPath}\` (prefer shrinking the allowlist, never grow without reason).`,
    );
    process.exit(1);
  }

  if (unused.length > 0) {
    console.warn(
      `${label} px-text: ${unused.length} allowlist entr(y/ies) no longer match — remove them:`,
    );
    for (const k of unused) {
      console.warn(`  stale  ${k}`);
    }
    // Soft: warn only so fixing tokens is encouraged without blocking.
  }

  console.log(
    `${label} px-text: OK (0 new; ${seenAllowlisted.size} allowlisted; ${unused.length} stale)`,
  );
}
