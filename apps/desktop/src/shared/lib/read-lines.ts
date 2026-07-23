/**
 * Grok read_file / write tool body format:
 *   LINE_NUMBER→LINE_CONTENT
 *
 * Read numbers only the first visible line and every 10th line; other lines
 * are bare content. Write/create numbers every line. The arrow is a model
 * orientation aid — UI should parse it into a real gutter, not show `880→`.
 */

export type NumberedLine = {
  no: number;
  text: string;
};

const ARROW_LINE = /^(\d+)→(.*)$/;

function splitArrowLine(line: string): { no: number; text: string } | null {
  const m = ARROW_LINE.exec(line);
  if (!m) return null;
  const no = Number(m[1]);
  if (!Number.isFinite(no) || no < 1) return null;
  return { no, text: m[2] ?? "" };
}

/**
 * True when `text` looks like Grok-style arrow-numbered file content
 * (first line is `N→…`, optionally sparse decade markers).
 */
export function isGrokArrowNumbered(text: string): boolean {
  const lines = text.replace(/\r\n/g, "\n").replace(/\r/g, "\n").split("\n");
  // Drop trailing empty from final newline.
  if (lines.length > 0 && lines[lines.length - 1] === "") lines.pop();
  if (lines.length === 0) return false;
  if (!splitArrowLine(lines[0]!)) return false;
  // At least one numbered line (first). Sparse or dense both OK.
  return true;
}

/**
 * Parse Grok arrow-numbered body into consecutive (file line no, text) rows.
 * Returns null when the text is not in that format.
 */
export function parseGrokArrowNumberedLines(text: string): NumberedLine[] | null {
  if (!isGrokArrowNumbered(text)) return null;

  const raw = text.replace(/\r\n/g, "\n").replace(/\r/g, "\n").split("\n");
  if (raw.length > 0 && raw[raw.length - 1] === "") raw.pop();

  const out: NumberedLine[] = [];
  let next: number | null = null;

  for (const line of raw) {
    const numbered = splitArrowLine(line);
    if (numbered) {
      out.push({ no: numbered.no, text: numbered.text });
      next = numbered.no + 1;
      continue;
    }
    if (next == null) return null;
    out.push({ no: next, text: line });
    next += 1;
  }

  return out.length > 0 ? out : null;
}

/**
 * Plain source text with arrow prefixes removed (for copy / highlight).
 * If not arrow-numbered, returns the original string.
 */
export function stripGrokArrowNumbers(text: string): string {
  const parsed = parseGrokArrowNumberedLines(text);
  if (!parsed) return text;
  return parsed.map((l) => l.text).join("\n");
}
