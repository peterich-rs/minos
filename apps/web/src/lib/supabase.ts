import { createClient, type SupabaseClient } from '@supabase/supabase-js'

const url = import.meta.env.VITE_SUPABASE_URL as string | undefined
const anonKey = import.meta.env.VITE_SUPABASE_ANON_KEY as string | undefined

/** True when both Supabase env vars are set (OAuth / email IdP path enabled). */
export function isSupabaseConfigured(): boolean {
  return Boolean(url?.trim() && anonKey?.trim())
}

let client: SupabaseClient | null = null

export function getSupabase(): SupabaseClient {
  if (!isSupabaseConfigured()) {
    throw new Error(
      'Supabase is not configured. Set VITE_SUPABASE_URL and VITE_SUPABASE_ANON_KEY in .env.local',
    )
  }
  if (!client) {
    client = createClient(url!.trim(), anonKey!.trim(), {
      auth: {
        persistSession: true,
        autoRefreshToken: true,
        detectSessionInUrl: true,
      },
    })
  }
  return client
}

/**
 * Start Google OAuth. Redirects the browser to Google → Supabase → back to
 * the current origin (hash/query tokens are picked up by detectSessionInUrl).
 */
export async function signInWithGoogle(): Promise<void> {
  const supabase = getSupabase()
  const redirectTo = `${window.location.origin}/`
  const { error } = await supabase.auth.signInWithOAuth({
    provider: 'google',
    options: { redirectTo },
  })
  if (error) {
    throw error
  }
}

/** Email + password via Supabase Auth (not Minos password). */
export async function signInWithSupabasePassword(
  email: string,
  password: string,
): Promise<string> {
  const supabase = getSupabase()
  const { data, error } = await supabase.auth.signInWithPassword({ email, password })
  if (error) {
    throw error
  }
  const token = data.session?.access_token
  if (!token) {
    throw new Error('Supabase sign-in returned no access_token')
  }
  return token
}

export async function signUpWithSupabasePassword(
  email: string,
  password: string,
): Promise<string> {
  const supabase = getSupabase()
  const { data, error } = await supabase.auth.signUp({ email, password })
  if (error) {
    throw error
  }
  const token = data.session?.access_token
  if (!token) {
    throw new Error(
      'Supabase sign-up succeeded but no session yet (email confirmation may be required)',
    )
  }
  return token
}

export async function getSupabaseAccessToken(): Promise<string | null> {
  if (!isSupabaseConfigured()) {
    return null
  }
  const { data } = await getSupabase().auth.getSession()
  return data.session?.access_token ?? null
}

export async function signOutSupabase(): Promise<void> {
  if (!isSupabaseConfigured()) {
    return
  }
  await getSupabase().auth.signOut()
}
