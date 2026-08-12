/**
 * Map outbox settlement → UI delivery for user bubbles.
 *
 * Only `acked` may become `sent`. Timeout and terminal failure both surface as
 * `failed` so the user sees a retry affordance (WeChat-style red `!`), even when
 * the durable outbox may still retry in the background.
 */
export function deliveryStatusAfterUserSettlement(
  settlement: "acked" | "failed_terminal" | "timeout",
): "sent" | "failed" {
  if (settlement === "acked") return "sent";
  return "failed";
}
