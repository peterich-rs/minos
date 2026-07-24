/**
 * Pure policy for daemon connection toasts.
 *
 * Transient disconnects are debounced so brief flaps (daemon restart, pump
 * rearm) do not flash error → success toasts. Terminal reconnect after a
 * debounced disconnect toasts success immediately.
 */

export const DISCONNECT_TOAST_DEBOUNCE_MS = 2_000;

export type ConnectionToastAction =
  | { type: "none" }
  | { type: "schedule_disconnect"; message: string }
  | { type: "cancel_pending" }
  | { type: "toast_connected"; detail: string }
  | { type: "commit_disconnected" };

/**
 * Decide toast side-effects for a connected-flag transition.
 *
 * @param prev - last **committed** toast state (`null` = first observation)
 * @param connected - current `connection.connected`
 * @param pendingDisconnect - a disconnect toast is already scheduled
 * @param disconnectMessage - error detail if disconnecting
 * @param connectedDetail - success detail if reconnecting
 */
export function planConnectionToast(input: {
  prev: boolean | null;
  connected: boolean;
  pendingDisconnect: boolean;
  disconnectMessage: string;
  connectedDetail: string;
}): ConnectionToastAction {
  const { prev, connected, pendingDisconnect, disconnectMessage, connectedDetail } =
    input;

  if (prev === null) {
    return { type: "none" };
  }

  if (prev && !connected) {
    if (pendingDisconnect) return { type: "none" };
    return { type: "schedule_disconnect", message: disconnectMessage };
  }

  if (prev && connected) {
    // Flap during debounce: stay connected without toasting.
    if (pendingDisconnect) return { type: "cancel_pending" };
    return { type: "none" };
  }

  if (!prev && connected) {
    return { type: "toast_connected", detail: connectedDetail };
  }

  // !prev && !connected
  return { type: "none" };
}
