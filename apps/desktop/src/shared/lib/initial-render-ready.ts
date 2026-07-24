import { emit } from "@tauri-apps/api/event";
import { isTauriRuntime } from "@/shared/lib/daemon";

/** Rust `window_reveal` listens for this before first show(). */
export const INITIAL_RENDER_READY_EVENT = "initial-render-ready";

let emitted = false;

/**
 * Signal the Tauri host that React has committed a first surface (or a
 * recoverable error UI). Idempotent — safe to call from App layout and from
 * ErrorBoundary after a render crash so the window still reveals.
 * No-op in plain Vite (browser).
 */
export function emitInitialRenderReady(): void {
  if (!isTauriRuntime() || emitted) return;
  emitted = true;
  void emit(INITIAL_RENDER_READY_EVENT).catch((err: unknown) => {
    // Allow a later path (ErrorBoundary) to retry if the first emit failed.
    emitted = false;
    console.warn("[initial-render-ready] emit failed", err);
  });
}
