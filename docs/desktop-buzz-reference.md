# Buzz Desktop 借鉴清单（Minos 对照）

> 源码对照：`block/buzz` 的 `desktop/` ↔ Minos `apps/desktop/`  
> 目的：记录可复刻的 UI / 架构 / 组件 / 逻辑 / 代码设计，指导 Minos desktop 演进。  
> 相关文档：[architecture-desktop.md](./architecture-desktop.md)

---

## 1. 总体架构（强烈建议对齐）

```
desktop/src/
  main.tsx              # Provider 树 + bootstrap
  app/                  # Shell、路由、全局快捷键、顶栏 chrome
  features/<domain>/    # 业务：ui / lib / hooks
  shared/
    api/                # Tauri invoke + query client + 连接层
    ui/                 # 设计系统 primitive
    layout/             # chrome / 侧栏 / 辅面板几何
    theme/              # 主题 token + accent
    hooks/ lib/         # 跨域纯能力
```

### 可复刻原则

| 原则 | Buzz 做法 | 为什么值得学 |
|------|-----------|--------------|
| **Feature 切片** | `channels` / `messages` / `sidebar` 各自 `ui`+`lib`+`hooks` | 避免 AppShell 变成上帝组件 |
| **共享只放 primitive** | `shared/ui` 是 button/dialog/sidebar，不含业务 | feature 互不 import（Buzz 也有违规，但方向对） |
| **Shell 只编排** | `AppShell` 拼 sidebar + outlet + overlays；业务在 feature | Minos 的 `AppShell` 还很薄，扩展时应继续保持薄 |
| **路由驱动视图** | TanStack Router + hash history | Minos 现在是 `primaryNav` store 切换；规模上来后 router 更稳 |
| **边界 remount** | `<AppReady key={communityKey} />` + `resetCommunityState()` | 多 workspace/host 切换时，模块级单例必须显式 reset |
| **文件硬顶 1000 行** | `check-file-sizes.mjs` | Minos 已有；继续严格执行，别开 override 逃 |

**对 Minos 的建议**：继续 `features/work|chat|agents|host`，不要把 transcript / approval / session 全塞进 `store/`。Zustand 适合 UI 状态；领域投影逻辑放到 `features/*/lib`，和 Buzz 的 `messages/lib`、`channels/readState` 一样。

---

## 2. UI 体系（可直接抄模式，别整文件硬搬）

### 2.1 设计系统层

Buzz 是 **shadcn/new-york + Radix + CVA + Tailwind CSS variables**：

- `components.json` 指向 `@/shared/ui`
- Token 在 `shared/styles/globals/theme.css`
- 语法高亮主题驱动 UI 主题：`shared/theme/ThemeProvider.tsx`（Catppuccin / Houston + accent）
- **rem 文本缩放**：`useWebviewZoomShortcuts` + `check:px-text`（可读文字禁止 `text-[13px]`）

Minos 已有 ThemeProvider / button / dialog / tooltip。建议补齐：

1. **统一 motion token**（几乎零成本、体感立刻一致）
   - Buzz：`modalMotion.ts` / `popoverSurface.ts` / `deferredModalOpen.ts`
   - 所有 Dialog/Popover/Dropdown 共用 enter/exit，避免每个弹层一套动画
2. **Chrome 几何常量**
   - Buzz：`chromeLayout.ts`：顶栏高度、content top padding 用 CSS 变量
   - 原生 traffic lights 用 **固定 px**，正文用 **rem**——这条规则很值钱
3. **Top chrome 模式**
   - Buzz：`AppTopChrome`：sidebar toggle + back/forward + drag region
   - macOS 全屏/非全屏对 traffic light 的 padding 分支
4. **辅面板 shell**
   - Buzz：`AuxiliaryPanelShell` + resizable width + overlay/split 双布局
   - Minos 的 `SessionInspector` / Approval / Transcript 侧栏非常适合这套

### 2.2 值得抄的具体组件（按 ROI）

| 组件 | Buzz 路径 | 用途 | Minos 适用场景 |
|------|-----------|------|----------------|
| `VirtualizedList` | `shared/ui/VirtualizedList.tsx` | 通用虚拟列表 + sticky header portal | 会话列表、文件变更列表 |
| `PageHeader` | `shared/ui/PageHeader.tsx` | 统一页头 | Work/Agents/Host 标题区 |
| `ViewLoadingFallback` | `shared/ui/ViewLoadingFallback.tsx` | lazy route fallback | 路由拆分后 |
| `UnreadPill` / `AnimatedCount` | `shared/ui/*` | 未读/计数动效 | Attention badge |
| `sidebar-action-card` | `shared/ui/sidebar-action-card.tsx` | 侧栏状态卡 | 连接失败 / host offline |
| `OverlayPanelBackdrop` | `shared/ui/OverlayPanelBackdrop.tsx` | 右侧浮层面板 | Inspector overlay |
| `UserAvatar` + framing CSS | `shared/ui/UserAvatar.tsx` | 头像裁切一致性 | Agent/user avatar |
| Markdown 管线 | `shared/ui/markdown*` | GFM + code + mention | Agent transcript（按需裁剪，别整份 2000 行搬） |
| Composer 族 | `features/messages/ui/MessageComposer*` | TipTap + draft + attach + emoji | 聊天输入（Minos Composer 可对照深化） |
| Create channel dialog | `features/sidebar/ui/CreateChannelDialog*` + form hook | 标题/描述/权限配置后再创建 | **已借鉴**：`features/work/ui/CreateConversationDialog`（title / priority / **agent roster**；roster 约束后续 @mention） |
| Message row 拆分 | `MessageRow` / `MessageHeader` / `MessageActionBar` / `MessageReactions` | 一行消息拆成稳定子树 | 已有雏形，可继续对齐 |
| Timeline 虚拟化 | `TimelineMessageList` + `useAnchoredScroll` + `useBufferedTimelineMessages` | stick-to-bottom / 加载更早 / 锚定滚动 | **聊天核心，最高价值** |

### 2.3 视觉/动效细节（产品感来源）

- **Boot splash hold**：冷启动最少显示 ~1.2s，再 fade；E2E 可关掉（`App.tsx`）
- **Community switch gate**：快切不闪 splash，>300ms 才出 spinner
- **EmojiBurst / PoofBurst**：反馈微交互（可后做）
- **Grainient / theme surfaces**：背景氛围层，和内容层分离
- **`motion-reduce:animate-none`**：无障碍默认就做对

---

## 3. 逻辑与状态设计（比 UI 更该学）

### 3.1 数据分层

Buzz 的实际分层：

```
Tauri/Rust commands  →  shared/api/*  →  React Query hooks  →  feature UI
Realtime WS/events   →  feature stores / useSyncExternalStore  →  UI
```

关键模式：

1. **React Query 管「拉取 + 缓存 + mutation」**  
   `createBuzzQueryClient()`：`refetchOnWindowFocus: false`、`networkMode: "always"`  
   桌面本地/daemon 场景同样适用。

2. **模块级 store + 显式 reset**  
   drafts、observer、reaction hydration、media cache……  
   切换 community/workspace 时 `resetX()` 集中在一处（`useCommunityInit`）。  
   **Minos 多 host / 多 workspace 时必抄这个合同**，否则旧会话会串台。

3. **跨 feature 解耦用「request event」而不是互相 import UI**
   ```ts
   // features/agents/openCreateAgentEvent.ts
   requestOpenCreateAgent(...)
   subscribeOpenCreateAgent(...)
   ```
   Channel 欢迎页可以请求「打开创建 Agent」，不必 import Agent dialog。

4. **Read-state 独立子系统**  
   `features/channels/readState/*`：format / storage / manager / hook 分离。  
   Minos 的 attention / unread 应同等对待，不要散落在组件里。

5. **`useStableMap` / `useStableArrayShallow` / `useStableSet`**  
   派生 `Map`/`[]` 时保 reference，让 `React.memo` 真的生效。  
   Buzz AGENTS.md 专门写了这个坑——Minos timeline 已经在吃性能，这个值得直接搬。

6. **纯函数 + 同文件测试**  
   滚动锚定、unread、mention ranking、timeline projection 全是可单测的纯逻辑。  
   Minos 已有这个习惯（`stick-to-bottom`、`virtual-timeline-items`），继续加强即可。

### 3.2 连接与韧性（Buzz 很强，按需裁剪）

Buzz `shared/api/relay*` 一整套：

- reconnect controller / policy / replay  
- stall watchdog  
- rate limit gate  
- connection state emitter  
- closed recovery  

Minos 对应的是 daemon 连接。不必抄 Nostr 细节，但应抄 **状态机拆分**：

- `connection-state`（connected/reconnecting/closed）
- `reconnect-policy`（backoff）
- `stall-watchdog`（假活检测）
- UI 只订阅投影（Buzz 的 `SidebarRelayConnectionCard` / `RelayConnectionOverlay`）

### 3.3 导航与深链

- `useAppNavigation` 集中 goHome/goChannel/goSettings  
- `communityNavigationStorage` 记住每个 community 上次停留页  
- `deep-link.ts` + `useMessageDeepLinks`  
- Hash router：Tauri 里比 browser history 省心  

Minos 若要做 `minos://session?...` 深链，直接对标这套。

---

## 4. App 组装顺序（可复刻的「启动合同」）

Buzz `main.tsx` / `App.tsx` 的门控链：

```
bootstrap
  → migrate storage
  → install E2E mocks (dev only)
  → Providers (Communities → Onboarding → Theme → Tooltip → …)
  → App gates:
       keyring locked?
       onboarding?
       community setup?
       community apply error?
       loading / switch gate?
  → AppReady (key=communityKey) → Router → AppShell
```

Minos 已有 BootScreen + `initial-render-ready`（很好）。建议补：

- Provider 分层更细（Tooltip/Toaster 不要只塞在 Shell）
- Boot / error / empty workspace 用 **判别联合状态**，而不是一堆 boolean
- 窗口 reveal 与数据 bootstrap 解耦（Buzz/Minos 都已做，保持）

---

## 5. 聊天时间线（如果只抄一块，抄这块）

Buzz 消息路径是 desktop 里最深的模块，Minos 已有简化版（`MessageList` + virtua）。对照升级路径：

| 能力 | Buzz | 建议 |
|------|------|------|
| 虚拟窗口 | `TimelineMessageList` | 保持 virtua，抽 `buildVirtualItems` |
| stick-to-bottom | `useAnchoredScroll` / settle gates | 对齐 Buzz 的 programmatic pin / split panel settle |
| 新消息缓冲 | `useBufferedTimelineMessages` | 用户上翻时缓冲，回底再 flush |
| 行组件 memo | `MessageRow` 拆 Header/Body/Actions | 每 prop 稳定；mutation 对象勿整传 |
| 日期分隔 / unread 分隔 | `DayDivider` `UnreadDivider` | 已有 grouping，补 unread 线 |
| 保留策略 | `timelineRetention` | 长会话限窗，防内存涨 |
| Composer | TipTap + draft persist + reply banner | 按需上 TipTap；draft 按 conversationId 持久化 |

**不要整文件复制** `MessageTimeline.tsx`（体量很大且绑 Nostr）。学的是 **状态机 + 纯函数边界 + memo 合同**。

---

## 6. 代码设计习惯（日常写码就该对齐）

1. **Co-locate test**：`foo.ts` + `foo.test.ts`（Buzz 多用 `*.test.mjs`）
2. **Helpers 抽离再测**：`AppShell.helpers.ts`、`ChannelPane.helpers.ts`
3. **Types 文件旁置**：`ChannelScreen.types.ts`，避免 UI 文件膨胀
4. **Lazy feature screens**：`React.lazy(() => import(SettingsScreen))`
5. **Escape / scroll lock hooks**：`useEscapeKey`、`useWebviewScrollBoundaryLock`（桌面 WebView 必要）
6. **快捷键表数据化**：`keyboard-shortcuts.ts` 一份 registry，设置页自动渲染
7. **平台分支集中**：`platform.ts` 的 `isMacPlatform` / `hasPrimaryShortcutModifier`
8. **E2E mock bridge**：`testing/e2eBridge` + URL `?e2e=mock`（Buzz 截图/CI 靠这个）
9. **CSS 按域拆分**：`globals/{composer,markdown,scrollbars,motion}.css`，不要一个 5k 行 index.css
10. **注释写「为什么」**：traffic light 用 px、boot splash hold、stable empty array for zustand 都有解释

---

## 7. 针对 Minos 的优先级路线图

Minos 现状：壳子在、chat 基础在、zustand 偏重、UI primitive 偏少、无 router、辅面板/chrome 体系未成型。

### P0 — 立刻可做，收益大

1. **Motion / surface token**（modal/popover 统一） — **done**（`modalMotion` / `popoverSurface` / `deferredModalOpen` / `modalBackdrop` blur+black tint）
2. **`useStable*` + memo 合同**（timeline/session list） — **done**（Wave B）
3. **Workspace/host 切换 reset 注册表** — **done**（`resetWorkspaceModuleState`；project 切换不触发）
4. **Chrome layout CSS 变量** + macOS top inset — **partial done**（`chromeLayout` vars；titlebar 仍系统装饰，traffic-light inset 待 hidden titlebar）
5. **连接状态 → 侧栏卡片 / toast 策略** — **done**（`SidebarConnectionCard` + `connection-card-policy`，与 toast 同 2s 防抖）

整合分支：`refactor/desktop-engineering-alignment`（门禁 + helpers/stable + shell + workspace reset；少碎分支）。  
覆盖：SessionsView/helpers 拆分、rem+zoom、invokeDaemon、useStable*、motion/chrome、连接卡、AuxiliaryPanel、`resetWorkspaceModuleState`。

### P1 — 产品形态对齐

6. **AuxiliaryPanel** 承载 SessionInspector / Approval / Diff
7. **PageHeader + 统一空态/加载态**
8. **Composer 深化**：draft、reply banner、快捷键发送、附件
9. **Attention/read-state 子系统**（独立 storage + hook）
10. **Command palette 数据化**（注册表驱动，而不是写死）

### P2 — 规模化时再上

11. TanStack Router（深链、back/forward、settings 子页）
12. Feature-level event bus（`requestOpen*`）
13. Markdown/shiki 管线增强（agent diff/code）
14. Playwright + mock bridge
15. 主题 accent + follow system（Buzz ThemeProvider 可裁剪移植）

### 不建议直接搬

- Nostr/relay 协议栈、NIP-29 频道模型
- Huddle 音视频、Blossom media
- 整份 `markdown.tsx` / `VideoPlayer.tsx` / `AppShell.tsx`（过大且域绑定）
- Community rail 多社区 UI（除非 Minos 真做多 backend 账户切换）
- TipTap 全套 mention/emoji 扩展（先 plain/textarea 或轻 editor，不够再加）

---

## 8. 概念映射表

| Buzz 概念 | Minos 对应 | 动作 |
|-----------|------------|------|
| Community | Workspace / Host | remount key + reset registry |
| Channel + Timeline | Conversation + MessageList | 升锚定滚动与缓冲 |
| Thread auxiliary panel | Session inspector | 用 AuxiliaryPanelShell |
| AppSidebar + CommunityRail | Sidebar | 拆 Section / DnD / status card |
| Agents feature | Agents + runtime | request-event 解耦创建流 |
| relay connection card | daemon connection | 状态机 + 侧栏卡 |
| Settings screens | Host/settings | lazy + section nav |
| Onboarding gates | Boot/pairing | 判别联合状态机 |
| `shared/api/tauri*` | `shared/api/invoke` | 按域拆文件，别单文件涨 |

---

## 9. Buzz 关键路径速查

便于在 buzz 仓库内跳转（路径相对 `desktop/src/`）：

| 主题 | 路径 |
|------|------|
| 入口 / Provider | `main.tsx`, `app/App.tsx` |
| Shell | `app/AppShell.tsx`, `app/AppTopChrome.tsx` |
| 路由 | `app/router.tsx`, `app/routes.ts`, `app/navigation/` |
| Community reset | `features/communities/useCommunityInit.ts` |
| 时间线 | `features/messages/ui/MessageTimeline.tsx`, `TimelineMessageList.tsx`, `useAnchoredScroll.ts` |
| Composer | `features/messages/ui/MessageComposer*.tsx` |
| 辅面板 | `shared/layout/AuxiliaryPanelShell.tsx`, `auxiliaryPanelLayout.ts` |
| Chrome 几何 | `shared/layout/chromeLayout.ts` |
| Motion token | `shared/ui/modalMotion.ts`, `popoverSurface.ts`, `deferredModalOpen.ts` |
| Stable refs | `shared/hooks/useStableReference.ts` |
| 主题 | `shared/theme/ThemeProvider.tsx`, `shared/styles/globals/` |
| 连接韧性 | `shared/api/relayReconnect*.ts`, `relayStallWatchdog.ts` |
| 质量门 | `desktop/scripts/check-file-sizes.mjs`, `check-px-text.mjs` |
| 贡献约定 | 仓库根 `AGENTS.md`（Community Switching、rem 文本、React.memo 坑） |

---

## 10. 总结

Buzz desktop 最值得复刻的不是某个页面像素，而是这套合同：

> **Feature 切片 + Shell 编排 + 设计系统 token + 纯逻辑可测 + 模块单例可 reset + 列表虚拟化与 memo 纪律 + 桌面 chrome（traffic light / zoom / scroll lock）细节。**

Minos 已经走在同一条路上；下一步最有杠杆的是：

1. 辅面板 / chrome 几何  
2. timeline 滚动状态机  
3. workspace reset  
4. 跨 feature request-event  
5. UI motion token  

---

## 修订记录

| 日期 | 说明 |
|------|------|
| 2026-07-24 | 初版：对照 buzz `desktop/` 与 minos `apps/desktop/` 整理 |
| 2026-07-24 | Wave B：helpers 拆分 + useStable* + 列表 memo 合同落地 |
| 2026-07-24 | Wave C：shell 手感 — motion / chrome / connection card / AuxiliaryPanel |
| 2026-07-24 | P0 收尾：`resetWorkspaceModuleState`；分支收敛为 `refactor/desktop-engineering-alignment` |
