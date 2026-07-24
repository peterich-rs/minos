import { emit } from "@tauri-apps/api/event";
import { isTauriRuntime } from "@/shared/lib/daemon";

/** Rust `window_reveal` listens for this before first show(). */
export const INITIAL_RENDER_READY_EVENT = "initial-render-ready";

/**
 * Signal the Tauri host that React has committed its first layout.
 * No-op in plain Vite (browser) so browser dev is unaffected.
 */
export function emitInitialRenderReady(): void {
  if (!isTauriRuntime()) return;
  void emit(INITIAL_RENDER_READY_EVENT);
}
