import { QueryClient } from "@tanstack/react-query";

/**
 * Desktop server-state cache (Buzz-style TanStack Query).
 * Catalog / index lists live here; streaming windows stay in Zustand.
 */
export function createMinosQueryClient() {
  return new QueryClient({
    defaultOptions: {
      queries: {
        retry: 1,
        refetchOnWindowFocus: false,
        networkMode: "always",
        gcTime: 5 * 60 * 1_000,
        staleTime: 30_000,
      },
      mutations: {
        networkMode: "always",
      },
    },
  });
}

/** Singleton for store / live-ingress invalidation outside React. */
export const minosQueryClient = createMinosQueryClient();
