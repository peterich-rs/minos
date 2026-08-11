/**
 * Map outbox settlement → UI delivery for user bubbles.
 * Only `acked` may become `sent`; timeout keeps `sending` (still pending).
 */
export function deliveryStatusAfterUserSettlement(
  settlement: "acked" | "failed_terminal" | "timeout",
): "sent" | "sending" | "failed" {
  if (settlement === "acked") return "sent";
  if (settlement === "failed_terminal") return "failed";
  return "sending";
}
