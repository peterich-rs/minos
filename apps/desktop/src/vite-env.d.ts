/// <reference types="vite/client" />

interface ImportMetaEnv {
  readonly VITE_MINOS_BACKEND_URL?: string;
  readonly VITE_SUPABASE_URL?: string;
  readonly VITE_SUPABASE_ANON_KEY?: string;
  readonly VITE_MINOS_RELEASES_URL?: string;
}

interface ImportMeta {
  readonly env: ImportMetaEnv;
}
