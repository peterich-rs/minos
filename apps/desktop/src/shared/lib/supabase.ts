/**
 * Supabase Auth client for Desktop IdP (D01).
 *
 * When `VITE_SUPABASE_URL` + `VITE_SUPABASE_ANON_KEY` are set, email/password
 * goes through Supabase → exchange. OAuth/deep-link is deferred until
 * `tauri-plugin-deep-link` is wired (scope: password minimum).
 */

import { createClient, type SupabaseClient } from "@supabase/supabase-js";

const url = import.meta.env.VITE_SUPABASE_URL as string | undefined;
const anonKey = import.meta.env.VITE_SUPABASE_ANON_KEY as string | undefined;

/** True when both Supabase env vars are set (IdP path enabled). */
export function isSupabaseConfigured(): boolean {
  return Boolean(url?.trim() && anonKey?.trim());
}

let client: SupabaseClient | null = null;

export function getSupabase(): SupabaseClient {
  if (!isSupabaseConfigured()) {
    throw new Error(
      "Supabase is not configured. Set VITE_SUPABASE_URL and VITE_SUPABASE_ANON_KEY.",
    );
  }
  if (!client) {
    client = createClient(url!.trim(), anonKey!.trim(), {
      auth: {
        persistSession: true,
        autoRefreshToken: true,
        // Desktop password flow does not use URL hash tokens.
        detectSessionInUrl: false,
        storageKey: "minos.desktop.supabase",
      },
    });
  }
  return client;
}

/** Email + password via Supabase Auth (not Minos password). */
export async function signInWithSupabasePassword(
  email: string,
  password: string,
): Promise<string> {
  const supabase = getSupabase();
  const { data, error } = await supabase.auth.signInWithPassword({
    email,
    password,
  });
  if (error) throw error;
  const token = data.session?.access_token;
  if (!token) {
    throw new Error("Supabase sign-in returned no access_token");
  }
  return token;
}

export async function signUpWithSupabasePassword(
  email: string,
  password: string,
): Promise<string> {
  const supabase = getSupabase();
  const { data, error } = await supabase.auth.signUp({ email, password });
  if (error) throw error;
  const token = data.session?.access_token;
  if (!token) {
    throw new Error(
      "Supabase sign-up succeeded but no session yet (email confirmation may be required)",
    );
  }
  return token;
}

export async function getSupabaseAccessToken(): Promise<string | null> {
  if (!isSupabaseConfigured()) return null;
  const { data } = await getSupabase().auth.getSession();
  return data.session?.access_token ?? null;
}

/** Best-effort Supabase signOut; network failure must not block logout. */
export async function signOutSupabase(): Promise<void> {
  if (!isSupabaseConfigured()) return;
  try {
    await getSupabase().auth.signOut();
  } catch {
    // best-effort
  }
}
