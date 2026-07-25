/**
 * Platform / modifier helpers for desktop shortcuts.
 * Keep Cmd/Ctrl branching here so AppShell, Composer, zoom, etc. stay consistent.
 */

/** True when the primary shortcut modifier is held (⌘ on macOS, Ctrl elsewhere). */
export function hasPrimaryShortcutModifier(
  event: Pick<KeyboardEvent, "metaKey" | "ctrlKey">,
): boolean {
  return event.metaKey || event.ctrlKey;
}
