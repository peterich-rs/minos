# Minos Desktop

Host-side desktop shell that will replace the TUI as the primary local control surface.

**Stack:** Tauri 2 · React 19 · TypeScript · Vite · Tailwind · Zustand

**Status:** UI + daemon local-RPC bridge (same discovery as TUI). Falls back to mock when daemon is offline or when running plain Vite in a browser.

## Layout

```
┌ Sidebar          │ Work: Project header (Conversations | Board)     │
│ Work/Attention/  │ Conversations list │ Timeline + @input │ Sessions │
│ Agents/Host      │            or Board kanban of conversations      │
│ + Projects       │                                                  │
```

## Develop

```bash
# Tauri shell: connects to existing local-rpc daemon if present, otherwise
# starts a managed in-process daemon (same strategy as minos-tui).
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

## Architecture notes

- Frontend lives in `src/` and is intentionally **not** shared with `apps/web` yet — visual baseline is the desktop mockup, not the current web admin demo.
- Rust host process lives in `src-tauri/` as a **standalone Cargo package** (not a workspace member of the root `crates/*` workspace).
- Bridge to `minos-daemon` will reuse the same local JSON-RPC surface the TUI uses (`minos_local_*`).

See [docs/architecture-desktop.md](../../docs/architecture-desktop.md).
