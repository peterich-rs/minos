/**
 * When to show the persistent sidebar connection card.
 * Complements toast policy: toasts are ephemeral; the card stays until
 * reconnect or explicit dismiss (for the current disconnect episode).
 */

export type ConnectionCardVisibilityInput = {
  /** App still on BootScreen. */
  booting: boolean;
  /** Workspace data source. */
  source: "mock" | "daemon";
  /** Daemon bridge connected. */
  connected: boolean;
  /** User dismissed the card for this disconnect episode. */
  dismissed: boolean;
};

export type ConnectionCardVisibility = "hidden" | "show";

/**
 * Pure visibility for the sidebar daemon connection card.
 * - Mock / boot → hidden
 * - Connected → hidden (and dismiss state should be cleared by the host)
 * - Disconnected + not dismissed → show
 */
export function planConnectionCardVisibility(
  input: ConnectionCardVisibilityInput,
): ConnectionCardVisibility {
  if (input.booting) return "hidden";
  if (input.source !== "daemon") return "hidden";
  if (input.connected) return "hidden";
  if (input.dismissed) return "hidden";
  return "show";
}
