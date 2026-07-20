/**
 * Strip terminal ANSI escape sequences from tool / command output.
 * Grok bash ACP content is raw process bytes with SGR colors; when rendered
 * outside a terminal the ESC byte is invisible and users see `[31m` / `[90m`.
 */
export function stripAnsiEscapes(input: string): string {
  if (!input) return input;
  // CSI: ESC[ … final; OSC: ESC] … BEL or ST; other 2-byte ESC sequences.
  // Also handle 8-bit CSI (0x9B).
  return input
    .replace(/\u001b\[[\x30-\x3f]*[\x20-\x2f]*[\x40-\x7e]/g, "")
    .replace(/\u001b\][^\u0007\u001b]*(?:\u0007|\u001b\\)/g, "")
    .replace(/\u001b./g, "")
    .replace(/\u009b[\x30-\x3f]*[\x20-\x2f]*[\x40-\x7e]/g, "");
}
