/**
 * Desktop live event bridge — mirrors TUI subscribe pumps.
 * Listens to Tauri-emitted daemon://* events and applies them via callbacks.
 */
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import {
  DAEMON_EVENT,
  isTauriRuntime,
  type DaemonConversationEvent,
  type DaemonIngestEvent,
  type DaemonManagerEvent,
  type DaemonPushStatusEvent,
} from "@/shared/lib/daemon";

export type DaemonEventHandlers = {
  onIngest: (ev: DaemonIngestEvent) => void;
  onManager: (ev: DaemonManagerEvent) => void;
  onConversation: (ev: DaemonConversationEvent) => void;
  /** Pump arm/death — drives workspace `livePush`. */
  onPushStatus?: (ev: DaemonPushStatusEvent) => void;
};

let unsubs: UnlistenFn[] = [];
let started = false;

/** Start global daemon push listeners (idempotent). */
export async function startDaemonEventBridge(
  handlers: DaemonEventHandlers,
): Promise<void> {
  if (!isTauriRuntime()) return;
  if (started) return;
  started = true;

  const u1 = await listen<DaemonIngestEvent>(DAEMON_EVENT.ingest, (e) => {
    handlers.onIngest(e.payload);
  });
  const u2 = await listen<DaemonManagerEvent>(DAEMON_EVENT.manager, (e) => {
    handlers.onManager(e.payload);
  });
  const u3 = await listen<DaemonConversationEvent>(
    DAEMON_EVENT.conversation,
    (e) => {
      handlers.onConversation(e.payload);
    },
  );
  const u4 = await listen<DaemonPushStatusEvent>(
    DAEMON_EVENT.pushStatus,
    (e) => {
      handlers.onPushStatus?.(e.payload);
    },
  );
  unsubs = [u1, u2, u3, u4];
}

export function stopDaemonEventBridge(): void {
  for (const u of unsubs) {
    try {
      u();
    } catch {
      /* ignore */
    }
  }
  unsubs = [];
  started = false;
}
