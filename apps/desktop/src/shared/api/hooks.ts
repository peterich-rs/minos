import { useQuery, useQueryClient } from "@tanstack/react-query";
import { daemonApi } from "@/shared/lib/daemon";
import { listCloudAgents } from "@/shared/lib/minos-cloud";
import { useAccountStore } from "@/store/account-store";
import { useWorkspaceStore } from "@/store/workspace-store";
import { minosQueryClient } from "./queryClient";
import { queryKeys } from "./queryKeys";

/** True when hooks may hit the daemon (Tauri-connected path). */
function useDaemonEnabled() {
  const source = useWorkspaceStore((s) => s.source);
  return source === "daemon";
}

/**
 * Host daemon agent profile cache.
 * Offline buffer / session launch helper — **not** bot identity SSOT.
 * Prefer `useCloudAgentsQuery` when account is online.
 */
export function useAgentProfilesQuery() {
  const enabled = useDaemonEnabled();
  const bootEpoch = useWorkspaceStore((s) => s.bootEpoch);
  return useQuery({
    queryKey: [...queryKeys.agentProfiles, bootEpoch],
    queryFn: async () => {
      const res = await daemonApi.listAgentProfiles();
      return res.profiles;
    },
    enabled,
  });
}

/**
 * Hub bot directory (global bot identity SSOT).
 * Enabled when the account has an access token.
 */
export function useCloudAgentsQuery() {
  const deviceId = useAccountStore((s) => s.deviceId);
  const accessToken = useAccountStore((s) => s.session?.accessToken);
  const enabled = Boolean(accessToken?.trim());
  return useQuery({
    queryKey: [...queryKeys.cloudAgents, deviceId, accessToken ?? ""],
    queryFn: async () => {
      const token = accessToken?.trim();
      if (!token) return [];
      return listCloudAgents(deviceId, token);
    },
    enabled,
    staleTime: 30_000,
  });
}

export function useModelsQuery(runtime: string | null | undefined) {
  const enabled = useDaemonEnabled() && Boolean(runtime);
  return useQuery({
    queryKey: queryKeys.models(runtime ?? ""),
    queryFn: () => daemonApi.listModels(runtime!),
    enabled,
    staleTime: 5 * 60_000,
  });
}

/** Invalidate catalog caches after mutations or reconnect. */
export function invalidateCatalogQueries(opts?: {
  projectId?: string;
  conversationId?: string;
  all?: boolean;
}) {
  if (opts?.all) {
    void minosQueryClient.invalidateQueries();
    return;
  }
  void minosQueryClient.invalidateQueries({ queryKey: queryKeys.projects });
  void minosQueryClient.invalidateQueries({ queryKey: queryKeys.clis });
  void minosQueryClient.invalidateQueries({
    queryKey: queryKeys.agentProfiles,
  });
  void minosQueryClient.invalidateQueries({
    queryKey: queryKeys.cloudAgents,
  });
  if (opts?.projectId) {
    void minosQueryClient.invalidateQueries({
      queryKey: queryKeys.conversations(opts.projectId),
    });
    void minosQueryClient.invalidateQueries({
      queryKey: queryKeys.projectSessions(opts.projectId),
    });
  } else {
    void minosQueryClient.invalidateQueries({
      queryKey: ["projects"],
      predicate: (q) =>
        Array.isArray(q.queryKey) &&
        (q.queryKey[2] === "conversations" || q.queryKey[2] === "sessions"),
    });
  }
  if (opts?.conversationId) {
    void minosQueryClient.invalidateQueries({
      queryKey: queryKeys.inspectorSessions(opts.conversationId),
    });
  }
}

export function useInvalidateCatalog() {
  const qc = useQueryClient();
  return (opts?: {
    projectId?: string;
    conversationId?: string;
    all?: boolean;
  }) => {
    if (opts?.all) {
      void qc.invalidateQueries();
      return;
    }
    void qc.invalidateQueries({ queryKey: queryKeys.projects });
    if (opts?.projectId) {
      void qc.invalidateQueries({
        queryKey: queryKeys.conversations(opts.projectId),
      });
      void qc.invalidateQueries({
        queryKey: queryKeys.projectSessions(opts.projectId),
      });
    }
    if (opts?.conversationId) {
      void qc.invalidateQueries({
        queryKey: queryKeys.inspectorSessions(opts.conversationId),
      });
    }
  };
}
