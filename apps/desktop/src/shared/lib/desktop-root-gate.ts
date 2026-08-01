/**
 * Pure root surface decision for Desktop account-first boot.
 *
 * Aligns with Mobile `decideRootRoute`: no valid Minos session → full-screen
 * login; valid session → app (daemon bootstrap may still show BootScreen).
 */

export type DesktopAuthPhase =
  | "booting"
  | "unauthenticated"
  | "authenticated";

export type DesktopRootSurface = "boot" | "login" | "app";

export function decideDesktopRoot(input: {
  authPhase: DesktopAuthPhase;
  /** Daemon/workspace bootstrap in flight after auth. */
  workspaceBooting: boolean;
}): DesktopRootSurface {
  if (input.authPhase === "booting") return "boot";
  if (input.authPhase === "unauthenticated") return "login";
  if (input.workspaceBooting) return "boot";
  return "app";
}
