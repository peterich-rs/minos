/**
 * Sole account-boundary teardown for Desktop.
 *
 * Logout / account switch / auth-invalid must call this before another account
 * may observe product state. Workspace bootstrap reset remains daemon-scoped;
 * this owner tears down account-scoped caches and invalidates in-flight work.
 *
 * Note: im-cloud-* imports form a mild cycle with account-store through this
 * module. ESM evaluates fine as long as we only call stop helpers after init.
 */

import { minosQueryClient } from "@/shared/api/queryClient";
import { cloudDigestCache } from "@/shared/lib/cloud-digest-cache";
import { clearAllTopicCursors } from "@/shared/lib/cloud-cursors";
import { cancelCloudDigestHydrate } from "@/store/im/cloud-digest-ensure";
import { daemonApi, isTauriRuntime } from "@/shared/lib/daemon";
import { stopImCloudBridge } from "@/store/im/im-cloud-bridge";
import {
  resetImCloudSyncState,
  stopImOutboxWorker,
} from "@/store/im/im-cloud-sync";
import { useUiStore } from "@/store/ui-store";
import { emptyWorkspace } from "@/store/workspace/empty-workspace";
import { resetWorkspaceModuleState } from "@/store/workspace/reset-workspace-state";
import { useWorkspaceStore } from "@/store/workspace-store";

export type LeaveAccountScopeReason =
  | "sign-out"
  | "account-switch"
  | "auth-invalid";

/** Bumped on every leave so async writers can detect staleness. */
let accountScopeGeneration = 0;

export function getAccountScopeGeneration(): number {
  return accountScopeGeneration;
}

export function bumpAccountScopeGeneration(): number {
  accountScopeGeneration += 1;
  return accountScopeGeneration;
}

/**
 * Tear down every account-scoped process surface.
 * Safe to call multiple times; idempotent for workers/caches.
 */
export function leaveAccountScope(
  _reason: LeaveAccountScopeReason = "sign-out",
): void {
  bumpAccountScopeGeneration();

  // 1) Stop realtime + outbox before wiping caches (prevents late writes).
  stopImOutboxWorker();
  stopImCloudBridge();
  resetImCloudSyncState();
  // stopImCloudBridge already clears unreadCountedMessageIds.

  // 2) Hub list / resume state must not carry across accounts.
  cancelCloudDigestHydrate();
  cloudDigestCache.invalidate();
  clearAllTopicCursors();
  minosQueryClient.clear();

  // 3) Workspace module singletons + entity plane.
  resetWorkspaceModuleState({
    stopEventBridge: true,
    reactions: "durable-empty",
    clearUiEphemeral: true,
  });
  useWorkspaceStore.setState({
    ...emptyWorkspace,
    booting: false,
    bootPhase: "Signed out",
    bootProgress: 100,
    bootEpoch: 0,
    workspaceAccountId: null,
    livePush: false,
    focusedConversationId: null,
    readMessageCountById: {},
    loading: false,
    error: null,
    actionError: null,
  });

  // 4) Navigation selection is account-scoped (not reconnect-scoped).
  useUiStore.getState().clearAccountScopedUi();

  // 5) Best-effort drop host hit_ so next account cannot inherit /ws/host.
  if (isTauriRuntime()) {
    void daemonApi.hostClearCredential().catch(() => {
      /* daemon may be offline; ensure path force-registers on next login */
    });
  }
}
