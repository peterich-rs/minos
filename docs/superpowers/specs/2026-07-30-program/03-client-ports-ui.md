# D03 · Client Ports & UI SSOT (Desktop + Web)

| Field | Value |
|-------|--------|
| Domain ID | D03 |
| Status | Implementation-ready (refined 2026-07-31) |
| L0 | [long-term spec §4.5](../2026-07-30-cloud-identity-clients-long-term.md) |
| Tasks | `T-ui-*` in [tasks/TASKS.md](tasks/TASKS.md) |
| Depends on | P0 recommended; **D01** for OIDC login UI |
| Blocks | Web remote UX quality; shared presenters for cloud timeline |

---

## 1. Goal

- **一个 React 设计系统**（Desktop tokens + `shared/ui` + shells）。
- **两个数据端口**：
  - `HostPort` → daemon（Desktop / Tauri）
  - `CloudPort` → HTTPS `/v1/*` + `/ws/client`（Web；Desktop cloud panels 如需）
- Web **停止**投资独立的 shadcn demo 应用。
- **包装策略已锁定**：workspace alias（Option B）。

Mobile 不共享 React chrome（见 D04）。

---

## 2. Decisions (locked)

| # | Decision |
|---|----------|
| 1 | Desktop 视觉 + 交互模式是 React SSOT |
| 2 | Web 不嵌入 Tauri/daemon；CloudPort only |
| 3 | **workspace alias**（Option B）：Web 通过 `tsconfig paths` + vite alias import `apps/desktop/src/shared/*` |
| 4 | Web P0 product：Auth → Hosts → 一个 conversation/session timeline → Settings |
| 5 | Pure presenters（markdown、tool cards、status chips）**不能** import `@tauri-apps/*` |
| 6 | 第二个消费者稳定后再抽取 `packages/ui`（不是 Phase 2 的工作） |

---

## 3. Workspace alias 配置

### 3.1 目录结构

```text
apps/desktop/src/shared/
  ui/           # Button, Input, Dialog, Badge, Card 等纯展示组件
  theme/        # tokens (colors, spacing, typography), theme provider
  lib/          # utils (cn, formatters, hooks)
  types/        # 共享 TypeScript 类型（HostSummary, SessionEvent, ...）
  presenters/   # TranscriptPresenter, ToolCard, MessageRow, StatusChip
```

**约束**：`shared/` 下的所有文件**不能** import `@tauri-apps/*`、不能调用 daemon `invoke`。这些是 pure presenters。

### 3.2 Web tsconfig + vite alias

**`apps/web/tsconfig.json`**：

```json
{
  "compilerOptions": {
    "paths": {
      "@shared/ui/*": ["../desktop/src/shared/ui/*"],
      "@shared/theme/*": ["../desktop/src/shared/theme/*"],
      "@shared/lib/*": ["../desktop/src/shared/lib/*"],
      "@shared/types/*": ["../desktop/src/shared/types/*"],
      "@shared/presenters/*": ["../desktop/src/shared/presenters/*"]
    }
  }
}
```

**`apps/web/vite.config.ts`**：

```ts
import { resolve } from 'path';

export default defineConfig({
  resolve: {
    alias: {
      '@shared/ui': resolve(__dirname, '../desktop/src/shared/ui'),
      '@shared/theme': resolve(__dirname, '../desktop/src/shared/theme'),
      '@shared/lib': resolve(__dirname, '../desktop/src/shared/lib'),
      '@shared/types': resolve(__dirname, '../desktop/src/shared/types'),
      '@shared/presenters': resolve(__dirname, '../desktop/src/shared/presenters'),
    },
  },
});
```

### 3.3 Presenter purity gate（`T-ui-08`）

添加 lint 规则（ESLint `no-restricted-imports`）：

```js
// apps/desktop/eslint.config.js (shared/ 子目录 override)
{
  files: ['src/shared/**/*'],
  rules: {
    'no-restricted-imports': ['error', {
      patterns: [
        { group: ['@tauri-apps/*'], message: 'shared/ must not depend on Tauri' }
      ]
    }]
  }
}
```

---

## 4. CloudPort interface

### 4.1 TypeScript 接口（实现级别）

```ts
// apps/web/src/ports/cloud-port.ts

export interface MinosSession {
  account_id: string;
  email: string;
  access_token: string;
  refresh_token: string;
  expires_in: number;
}

export interface HostSummary {
  host_installation_id: string;
  host_display_name: string | null;
  linked_at_ms: number;
  online: boolean;
}

export interface AgentSessionSummary {
  session_id: string;
  host_installation_id: string;
  agent_type: string;
  status: string;
  // ... 与 backend OpenAPI 对齐
}

export interface CloudPort {
  // --- Auth (D01) ---
  exchangeSupabase(supabaseAccessToken: string): Promise<MinosSession>;
  loginPassword(email: string, password: string): Promise<MinosSession>; // transitional
  refresh(): Promise<MinosSession>;
  logout(): Promise<void>;

  // --- Hosts (D02) ---
  listHosts(): Promise<HostSummary[]>;

  // --- Sessions/Projection (D05) ---
  listAgentSessions(hostId: string): Promise<AgentSessionSummary[]>;
  readTurns(sessionId: string, opts?: { afterSeq?: number }): Promise<Turn[]>;

  // --- Realtime ---
  openClientSocket(): ClientRealtime; // ws-ticket + /ws/client

  // --- Host commands (golden path) ---
  sendHostCommand(hostId: string, command: HostCommand): Promise<void>;
}

export interface ClientRealtime {
  subscribe(sessionId: string, handler: (event: UiEvent) => void): Unsubscribe;
  close(): void;
}
```

### 4.2 CloudPort 实现骨架

```ts
// apps/web/src/ports/cloud-port-http.ts

export class CloudPortHttp implements CloudPort {
  constructor(
    private baseUrl: string,      // https://minos.ainexc.com
    private getSession: () => MinosSession | null,
    private onSessionRefreshed: (s: MinosSession) => void,
  ) {}

  async exchangeSupabase(supabaseAccessToken: string): Promise<MinosSession> {
    const res = await fetch(`${this.baseUrl}/v1/auth/supabase`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json', ...deviceHeaders() },
      body: JSON.stringify({ access_token: supabaseAccessToken }),
    });
    if (!res.ok) throw new CloudPortError(await res.json());
    const session = await res.json();
    return session;
  }

  // ... 其他方法类似，Bearer token 从 getSession() 获取
  // refresh() 用 runWithSessionRefresh wrapper（已有）
}

export const cloudPort = new CloudPortHttp(
  import.meta.env.VITE_MINOS_BACKEND_URL,
  () => useAuthStore.getState().session,
  (s) => useAuthStore.getState().setSession(s),
);
```

### 4.3 HostPort（Desktop 已有的 daemonApi）

HostPort 不需要重新设计——Desktop 的 `invokeDaemon(command, args)` 已经是成熟的 HostPort 实现。只需要把类型提取到 `shared/types/` 供 Web 在类型层面复用（虽然 Web 不调用 daemon）。

---

## 5. Web P0 IA

```text
AuthScreen
  ├── Supabase OAuth（Google 按钮 / email magic link）
  └── [transitional] Password login form

AppShell（Desktop 风格 sidebar density）
  ├── Hosts（GET /v1/hosts → host cards with online/offline status）
  ├── Work
  │   └── Session timeline（选中一个 host → sessions list → 一个 timeline）
  │       ├── TranscriptPresenter（from @shared/presenters）
  │       ├── MessageRow（from @shared/presenters）
  │       └── Composer（send via CloudPort.sendHostCommand）
  └── Settings
      ├── Backend origin（显示 VITE_MINOS_BACKEND_URL）
      ├── Account（email, logout）
      └── Theme（from @shared/theme）
```

**P0 不做**：full Board、full Agents CRUD、social friends、multi-pane resizable（这些是 Desktop 深度功能）。

---

## 6. Desktop P0 additions

- **Account session entry**（登录/登出）—— wired to D01
  - 系统浏览器 OAuth + 深链接回调
  - 登录状态显示在 sidebar 或 menu bar
- **Connection card**：Local only / Linked（状态来自 D02）
  - "Link this Mac" 按钮 → 调 daemon link RPC
  - Linked 后显示 account email + host name
- **不需要重建 Work multi-pane**（已有的功能保持）

---

## 7. Supabase JS client 集成（Web）

```ts
// apps/web/src/lib/supabase.ts
import { createClient } from '@supabase/supabase-js';

export const supabase = createClient(
  import.meta.env.VITE_SUPABASE_URL,
  import.meta.env.VITE_SUPABASE_ANON_KEY,
);

// Auth flow:
// 1. const { data } = await supabase.auth.signInWithOAuth({ provider: 'google' });
//    → 系统浏览器/同页 OAuth → 回调带 access_token
// 2. const session = supabase.auth.getSession();
// 3. const minosSession = await cloudPort.exchangeSupabase(session.access_token);
```

---

## 8. Exit criteria

- [ ] Web 使用 Desktop tokens/components（通过 workspace alias）
- [ ] CloudPort 拥有 Web 所有 golden path 网络 I/O
- [ ] ESLint 规则阻止 `shared/` import `@tauri-apps/*`
- [ ] Web Auth → Hosts → 一个 timeline 端到端可用
- [ ] Desktop account session + connection card 可用
- [ ] `pnpm check` Web + Desktop 均绿
- [ ] 旧 demo components 移除

---

## 9. Task slice

`T-ui-01` … `T-ui-10` in [tasks/TASKS.md](tasks/TASKS.md).
