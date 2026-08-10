# Minos Desktop

Host-side desktop shell that will replace the TUI as the primary local control surface.

**Stack:** Tauri 2 · React 19 · TypeScript · Vite · Tailwind · Zustand

**Status:** UI + daemon local-RPC bridge (same discovery as TUI). Falls back to mock when daemon is offline or when running plain Vite in a browser.

**Host plugins:** single-instance (focus existing window), window-state (geometry; VISIBLE excluded), initial-window-reveal (`visible: false` until React first layout emits `initial-render-ready`).

## Layout

```
┌ Sidebar          │ Work: Project header (Conversations | Board)     │
│ Work/Attention/  │ Conversations list │ Timeline + @input │ Sessions │
│ Agents/Host      │            or Board kanban of conversations      │
│ + Projects       │                                                  │
```

## Cloud account + Host Link

Optional remote collaboration (phone / web). Local coding works without any of this.

```bash
# apps/desktop/.env.local (see .env.example)
VITE_MINOS_BACKEND_URL=http://127.0.0.1:8787
# Optional Supabase IdP:
# VITE_SUPABASE_URL=https://<project>.supabase.co
# VITE_SUPABASE_ANON_KEY=<anon-key>
```

On **Host → Account & remote**:

1. Sign in (Supabase email/password → Minos exchange, or Minos password if Supabase unset)
2. With daemon online: **Link this Mac** (daemon proof → `POST /v1/hosts/link` → apply `hit_*`)
3. Sidebar shows `Ready · Linked` when `account-store.hostLink.linked`

## Develop

```bash
# Tauri shell: connects to existing local-rpc daemon if present, otherwise
# starts a managed in-process daemon.
just dev-desktop
```

Frontend-only (browser, **always mock** — no daemon bridge):

```bash
just dev-desktop-ui
# → http://localhost:1420
```

### Port 1420 (one Vite, many clients)

| 误解 | 实际 |
|------|------|
| 浏览器和 Tauri 各起一个 Vite 共用端口 | **不行** — 一个端口只能有一个监听进程 |
| 共用端口 = 一份 Vite，浏览器 + WebView 都连它 | **可以** — 热更新两边一起生效 |

`just dev-desktop` 的 `beforeDevCommand` 会：

1. 若 **1420 已有服务**（例如你已跑 `dev-desktop-ui`）→ **复用**，不再起第二个 Vite  
2. 若 1420 空闲 → 自己启动 Vite  

推荐日常：

- 只开桌面：`just dev-desktop`  
- 先浏览器后加窗口：终端 A `just dev-desktop-ui`，终端 B `just dev-desktop`（会 reuse）  
- 若 1420 被无关进程占用：关掉该进程，或 `lsof -i :1420`

## Build

```bash
just build-desktop
# or: cd apps/desktop && pnpm tauri:build
```

## Quality gates

Lightweight gates (Buzz-inspired) so the React tree does not rot as it grows.

```bash
just check-desktop          # full gates (= pnpm check:all)
# or from apps/desktop:
pnpm check:all
```

| Script | What |
|--------|------|
| `pnpm check` | TypeScript (`tsc --noEmit`) |
| `pnpm test` | Unit tests (`src/shared/lib/*.test.ts`, `src/features/chat/lib/*.test.ts`) |
| `pnpm check:biome` | Biome **lint errors only** (format opt-in; warnings may remain) |
| `pnpm check:file-sizes` | Soft file-size gate on `src/**/*.{ts,tsx}` |
| `pnpm check:px-text` | No new `text-[Npx]` / `font-size: Npx` (existing debt frozen in allowlist) |
| `pnpm check:all` | All of the above in order |

**Biome** (`biome.json`): double quotes + semicolons to match existing style. The gate fails only on lint **errors** — format is **not** required on every PR (`pnpm format` when you want Biome’s layout); remaining **warnings** are non-blocking. Excludes `dist/`, `src-tauri/`, `node_modules/`.

**File sizes** (`scripts/check-file-sizes.mjs`): warn `>400` lines, hard fail `>800`. `SessionsView.tsx` is temporarily allowlisted with a freeze cap (~1850; do not raise without a split plan). Prefer splitting before raising caps.

**Lockfile:** use **pnpm** only (`pnpm-lock.yaml`). Do not reintroduce `package-lock.json`.

## Architecture notes

- Frontend lives in `src/` and is intentionally **not** shared with `apps/web` yet — visual baseline is the desktop mockup, not the current web admin demo.
- Rust host process lives in `src-tauri/` as a **standalone Cargo package** (not a workspace member of the root `crates/*` workspace).
- Bridge to `minos-daemon` will reuse the same local JSON-RPC surface the TUI uses (`minos_local_*`).
- **Design tokens** live in `src/index.css` (`:root` CSS vars) and map through `tailwind.config.js`. Markdown/code polish is in `shared/ui/MarkdownText.tsx` + tone CSS vars (streaming still plain text). See [docs/architecture-desktop.md](../../docs/architecture-desktop.md) § Design tokens + markdown.
- **Chat reactions:** MessageRow action bar + emoji-mart picker; durable via local daemon (`toggle_conversation_message_reaction` / `chat_message_reactions`); `reaction-store` optimistic toggle (generation-gated) + hydrate from `list_conversation_messages`; mock seed only in browser Vite. See [docs/architecture-desktop.md](../../docs/architecture-desktop.md).

See [docs/architecture-desktop.md](../../docs/architecture-desktop.md).
