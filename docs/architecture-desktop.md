# Desktop 应用 (apps/desktop) 架构文档

> Host 端桌面控制台：Tauri + React，短期目标是 **TUI 能力的可视化**（Project → Conversation → Agent session），可选 Project Board 俯视图。

## 概述

| 项 | 值 |
|----|-----|
| 源码路径 | `apps/desktop/` |
| 产品定位 | **Conversation 协作工作台**（Project → Timeline 为主舞台；Session/Approval 为对话内能力与 Attention）。本机对标 TUI 能力；**account-first**：在本机登录 Desktop 即拥有 host 控制权；手机/Web 远程依赖 cloud **Online**（live `/ws/host`） |
| 当前阶段 | **Daemon-backed**：Tauri 宿主嵌 daemon；bootstrap 经 `daemonApi` 拉 projects / CLIs / live push。浏览器 `vite` 直开时 fallback mock 数据。**根门禁 account-first**：冷启动 `hydrateAuth` → 无 session 则 `LoginPage`；有 session 进 `AppShell`。**登录后自动** `ensureCloudConnection`（内部 prepare/sign/`POST /v1/hosts/link`/apply + 等待 hub online）。用户只见 **Online / Connecting / Offline**（顶栏 banner + 品牌副标题），无 Link/Unlink 主路径。**Hub conversation_id** 经 `agent_session.start` 透传到 Host 落库（禁止再造 Direct agent sessions 假会话） |
| 视觉 | 暖色多栏（参考 `res/desktop.jpeg` 气质，非客服 Inbox 语义） |
| 产品 spec | [2026-07-18-desktop-product-experience.md](superpowers/specs/2026-07-18-desktop-product-experience.md) |
| 状态拆分 spec | [2026-07-21-desktop-state-by-consumption.md](superpowers/specs/2026-07-21-desktop-state-by-consumption.md)（**P0–P4 done**；P5 cleanup reviewed；编码入口 §18） |
| 状态 review | [2026-07-22-desktop-state-p0-p4-review.md](superpowers/reviews/2026-07-22-desktop-state-p0-p4-review.md) |
| Buzz 借鉴清单 | [desktop-buzz-reference.md](./desktop-buzz-reference.md)（UI / 架构 / 组件 / 逻辑可复刻项与 P0–P2 路线图） |

## 标识命名（全栈约定）

| 名 | 含义 |
|----|------|
| `conversationId` | 协作对话 |
| `sessionId` | agent session（Minos 主键；DB / RPC / ingest / UI 统一） |
| `selectedSessionId` | Navigation 焦点指针（`ui-store`） |

历史上的 `threadId` / `thread_id` / 表 `threads` 已从 Minos 自有层移除。上游 Codex app-server 仍使用 `thread/start`、`threadId`——仅在 `minos-codex-protocol` 与 agent-runtime 的 Codex adapter 边界出现，并映射为 Minos `session_id`。`provider_session_id` 是 CLI 侧会话 id，与 Minos `session_id` 不同。

本地 daemon SQLite 为破坏性 schema 变更：清库后从 0 重建即可。

## 技术栈

| 层 | 技术 | 用途 |
|----|------|------|
| 桌面壳 | Tauri 2 | WebView 窗口 + Rust 宿主（single-instance、window-state、initial-window-reveal） |
| UI | React 19 + TypeScript | 多栏产品界面 |
| 构建 | Vite 7 | dev/build |
| 样式 | Tailwind CSS 3.4 + `tailwindcss-animate` | 设计 token（`index.css` CSS vars → Tailwind theme）+ enter/exit utilities |
| 交互原语 | Radix (Dialog / Dropdown / Tooltip / Popover / Slot) + CVA | shadcn 模式 headless 组件（`shared/ui/*`） |
| Emoji | `@emoji-mart/react` + `@emoji-mart/data` | 消息 reaction picker + Composer 插入；reactions 持久在 local daemon |
| 分栏 | `react-resizable-panels` v4 (`Group`/`Panel`/`Separator`) | Work：list \| timeline \| inspector |
| 命令面板 | `cmdk` + Radix Dialog | 全局 ⌘K 跳转 project / conversation / session / nav |
| Toast | `sonner` | 发送失败、daemon 断连/恢复、审批结果 |
| 动效 | `motion`（layout 导航指示）+ CSS `duration-150/200`（`--duration-fast/normal`） | 克制动效；全局尊重 `prefers-reduced-motion`（`animate-spin` 除外） |
| Markdown | `react-markdown` + `remark-gfm` + Shiki `CodeBlock` | 完成态 GFM；streaming 纯文本；fenced code 懒加载 Shiki 高亮 |
| 主题 | Shiki theme JSON → CSS vars（`ThemeProvider`） | Host → Appearance 选主题/强调色；FOUC 用 localStorage 缓存 vars；默认 `minos`（warm） |
| 长列表 | `@tanstack/react-virtual`（侧栏）+ `virtua`（conversation timeline） | ConversationList + **Sessions 左栏**（`SessionListPane` + flatten 树）用 `VirtualizedList`；主时间线 `VList` + stick-to-bottom/`shift` prepend；session **transcript** 仍分页 DOM（硬上限 2000），避免与审批/流式测高互殴 |
| 状态 | Zustand 5 + TanStack Query 5 | **混合**：RQ 可缓存 catalog 网络；**会 merge 进 SessionEntity 的 list**（inspector / project sessions）一律 `staleTime: 0`，禁止 30s 陈旧 lifecycle。Zustand 管 timeline/transcript/SessionEntity/乐观发送/UI 指针（L0–L6）。**SessionEntity 唯一写漏斗**：`mergeSessionEntity` / `patchSessionEntity` → `commitSessionEntity`（membership 投影 + `conversation.runningCount/approvalCount` 由 Entity Σ 重算）。list hydrate 是 sample（防 stale demote）；manager / resume / resolve 为 authoritative。Inspector 列表只读投影行，禁止组件层再 overlay 全表 Entity。 |)
| 图标 | Lucide React | 导航与工具栏 |
| 本机 API | `@tauri-apps/api` | `invoke` → Rust |
| 自动更新 | `tauri-plugin-updater` + `plugin-process` | 仅 release 构建启用；见 [desktop-auto-update.md](./desktop-auto-update.md) |

## 信息架构

```text
Sidebar
  Work | Attention | Agents | Host
  Projects list (select → Work)

Work → Project
  header: Conversations | Board
  Conversations view:
    list (progress filter: All | To do | In progress | Done; default All)
      | timeline + @input | session inspector
  Board view:
    backlog | running | needs_you | done  (cards = conversations)
```

| UI | mock | 后续接入 |
|----|------|----------|
| Projects | 3 fixtures | `list_projects` |
| Conversations | per-project | `list_conversations` |
| Timeline | messages + approval cards | conversation messages + ingest |
| Sessions 树 | 含 subagent | `list_conversation_agent_sessions` |
| Board | 四列派生状态 | 非独立任务系统 |
| Attention | needs_approval | approvals |
| Agents | CLI inventory + personalized profiles | `list_clis` (runtime set + capability flags from Rust SSOT), `list_models` (honest per-model efforts), agent profile CRUD; **profile `description` = peer-facing role brief** (≤500, seeds conversation roster when member brief empty); start session accepts optional `profile_id` (daemon resolves model/effort/instructions; explicit fields override) |
| Host | Local Ready + Server Online/Offline + 诊断；Account（身份 / Sign out）；Appearance / Updates | `deriveHostPresence`（cloud status）+ `account-store.ensureCloudConnection`；顶栏 `CloudConnectionBanner`。登录表单在根 `LoginPage` |

### Agents capability SSOT

Harness facts (which runtimes exist, model selection, reasoning-effort support, per-model effort ladders) live only in Rust:

- Domain: `AgentName` metadata + `AgentDescriptor` capability fields
- Daemon: `list_clis` (install + caps), `list_models` / `model_catalog` (models + efforts; empty efforts when unsupported)
- Desktop: projects via `daemon_list_clis` / `daemon_list_models` and `features/agents/lib/agentConfigProjection.ts`

UI must not hardcode rival runtime tables or invent effort chips when the catalog returns empty. Presentation maps (`agentMeta` colors) are allowed. See `apps/desktop/src/features/agents/AGENTS.md`.

## 目录结构

Feature-slice 布局（Wave 1 Phase 1–2）：按 **app 壳 / features / shared / store** 分层；`@/*` → `src/*`。

### Quality gates（Wave 1 Phase 3）

| 命令 | 作用 |
|------|------|
| `pnpm check` | `tsc --noEmit` |
| `pnpm test` | `src/shared/{lib,api,hooks,ui}/*.test.ts` + `src/store/workspace/*.test.ts` + `src/features/{chat,agents,work,host}/lib/*.test.ts` |
| `pnpm check:biome` | Biome **lint errors only**（format 不进 gate；warnings 可残留） |
| `pnpm check:file-sizes` | `src/**/*.{ts,tsx}` 行数：warn `>400` / hard `>800`（ALLOWLIST 当前为空） |
| `pnpm check:px-text` | 禁止 `text-[Npx]` / `font-size: Npx`；**allowlist 已清空**，新增即失败 |
| `pnpm check:all` | 以上串联（= `just check-desktop`） |

CI：`desktop` job（macos-15）跑 `pnpm check:all` + `cargo check -p minos-desktop`。Tauri Rust crate 不在 Linux workspace clippy/test 内（GUI 系统依赖）。门禁矩阵见 [`docs/ci-gates.md`](ci-gates.md)。

- Biome 作用域：`src/**`、`scripts/**`、根配置；排除 `dist/`、`src-tauri/`、`node_modules/`。
- Formatter **opt-in**：`pnpm format`；gate 只拦 lint **errors**，不强制全树 reformat，不因 warnings 失败。
- 文件体积：`SessionsView` 已拆为 thin shell（~265 行）+ `features/work/ui/*` / `lib/*`；file-size ALLOWLIST **无** SessionsView 条目。`helpers.ts` 已拆为 `dto-map` / `transcript-merge` / `empty-workspace` / `mock-bundle`（barrel 保留）。`TranscriptPane`（~561）仅 soft warn（`>400`），未抬 hard cap。
- 列表 memo 合同：派生 `Map`/`[]`/`Set` 经 `useStable*` 保 identity；行组件 `React.memo`；回调传稳定 `(id) => void`，禁止 per-row inline closure。
- 包管理：`pnpm` + `pnpm-lock.yaml` 为唯一 lockfile（勿再生成 `package-lock.json`）。

### Design tokens + markdown（Wave 1 Phase 4）

暖色多栏 palette 的**唯一真源**在 `apps/desktop/src/index.css` 的 `:root` CSS 变量；`tailwind.config.js` 用 `rgb(var(--color-*) / <alpha-value>)` 映射，保留 `bg-ink/5` 等 alpha 写法。

| Token 组 | CSS vars | Tailwind |
|----------|----------|----------|
| Surfaces | `--color-canvas*`、`--color-surface*` | `bg-canvas`、`bg-surface` / `muted` / `hover` / `raised` |
| Ink | `--color-ink*` | `text-ink` / `secondary` / `muted` / `faint` |
| Accent | `--color-accent*` | `bg-accent` / `strong` / `soft` |
| Bubbles（legacy tokens） | `--color-bubble-out/in` | Conversation 主时间线已改为 **Slack/Buzz 全宽行**（无左右气泡）；token 保留给其它 surface |
| Status | `--color-status-{idle,running,approval,suspended,failed,done}` | `bg-status-running` 等（semantic；既有 `statusMeta` 仍可用 stone/amber 类） |
| Radius / shadow | `--radius-shell/panel/code`、`--shadow-shell/panel` | `rounded-shell`、`shadow-panel`… |
| Motion | `--duration-fast` (150)、`--duration-normal` (200)、`--ease-out` | `duration-150` / `duration-200` / `ease-out` |

Markdown 呈现（`shared/ui/MarkdownText.tsx` + `index.css`）：
- 完成态：`react-markdown` + `remark-gfm`；tone 通过 `.markdown-tone-light|dark` 切 CSS vars（链接 / 行内 code / fenced pre / quote / table / list marker）。
- Streaming：仍为 plain pre-wrap + pulse cursor，避免逐 token 重建 MDAST。光标仅当 **timeline tail** 仍是 open 的 text/reasoning 气泡时显示（`transcript-streaming.ts`）；tool / subagent 插入后立即关掉，与 TUI `finish_open_content_streaming`、mobile `closeTextSegment` 一致——禁止回找「最后一条 streamable 文本」而在 session 仍 `running` 时留下 █。
- **时间线冻结**：`TextReplace` / live `mergeTranscriptItems` 不得改写 tool 上方的 assistant 行。同 body 的 OpenCode finished-part snapshot 丢弃；不同 body（新 part / tool 后叙述）在末尾 append。OpenCode text/reasoning 事件用 `message_id + U+001E + part_id` 绑定 part 段（`minos-ui-protocol` opencode translator）。
- **Subagent 单卡**：OpenCode `task` + `SubagentSpawned` / `SubagentStatusUpdated` 投影为 `kind: "subagent"` 一行（TUI `SubagentCall` 对齐）；禁止 `Running task …`、禁止 header 展示 `<task id=…>` XML、禁止整段 prompt 当 status。
- **Tool header**：优先 `state.title` / path / command；禁止 `Reading read`；禁止 XML 首行（`<path>` / `<task>`）当 target。
- Code block：背景、圆角、padding、横向滚动；行内 code 对比度略加强。

```
apps/desktop/
  scripts/
    check-file-sizes.mjs         # soft/hard line-count gate (empty ALLOWLIST)
    check-px-text.mjs            # rem zoom-safe text sizes
    check-px-text.allowlist.txt  # empty; any text-[Npx] fails
    dev-for-tauri.mjs
  biome.json                     # lint + optional format
  src/
    main.tsx · App.tsx · index.css · vite-env.d.ts
    app/                         # App shell composition
      AppShell.tsx · Sidebar.tsx · BootScreen.tsx
      CommandPalette.tsx · ConnectionToasts.tsx · SidebarConnectionCard.tsx
      useWebviewZoomShortcuts.ts # Cmd±/0 → root rem scale; webview zoom pinned to 1
    features/
      auth/                      # Full-screen LoginPage (root gate; Supabase → Minos)
      work/                      # Project → Conversations / Sessions / Board
        WorkView.tsx · ProjectHeader.tsx · ConversationList.tsx
        SessionInspector.tsx · SessionsView.tsx · SessionListPane.tsx
        ProjectBoard.tsx · CreateProjectEmpty.tsx
        # SessionsView is a thin shell (~265); transcript/modals live under ui/
        ui/
          TranscriptPane.tsx · TranscriptItemView.tsx · ApprovalModal.tsx
          SessionSummaryPanel.tsx · FileChangeRow.tsx
        lib/
          session-view-resolve.ts  # project-scoped deep-link session resolve
          user-action.ts           # approval/question decision routing
        # imports Timeline from features/chat (no local copy)
      attention/                 # AttentionView.tsx
      agents/                    # AgentsView.tsx · AGENTS.md (capability SSOT rule)
        lib/agentConfigProjection.ts  # pure map: list_clis/list_models → UI options
      host/                      # HostView.tsx · identity + cloud status (auto-connect; no link CTA)
        lib/host-link-flow.ts    # pure prepare→sign→cloud→apply orchestration
        lib/host-account-presenter.ts  # Local only / Linked / Error (+ defensive signed out)
      chat/                      # Conversation chat UI (Wave 1–2)
        Timeline.tsx             # thin container: load/poll + compose children
        TimelineEmpty.tsx        # no-conversation selected shell
        TimelineHeader.tsx       # title edit · branch/worktree · priority/progress
        MessageList.tsx          # stick-to-bottom · load-older · day dividers · rows
        MessageRow.tsx           # bubble · grouping · hover action bar · reactions
        MessageActionBar.tsx     # Reply + React (keyboard-focusable)
        MessageReactions.tsx     # reaction pills under bubble
        EmojiPicker.tsx          # quick strip + emoji-mart popover
        Composer.tsx             # draft · reply chip · @mention · emoji insert · send
        ReplyPreview.tsx         # reply-to chip on a bubble
        reaction-store.ts        # durable local reactions: hydrate + optimistic toggle
        lib/format.ts            # shortWorktree · replyPreviewBody · replyAuthorLabel
        lib/message-grouping.ts  # same-author continuity · day divider helpers
        lib/reactions.ts         # ReactionGroup types · pure toggle + daemon map/hydrate
        lib/reaction-seed.ts     # mock-only fixtures (browser Vite; gated when daemon)
    shared/
      api/
        invoke.ts                # invokeDaemon — single Tauri invoke entry + DaemonInvokeError
        daemon-invoke-error.ts · hooks.ts · queryClient.ts · queryKeys.ts
      ui/                        # Avatar · StatusPill · Tag · ErrorBoundary
                                 # MarkdownText · DiffView · ReadView · IncrementalText
                                 # button · dialog · dropdown-menu · popover · toaster · tooltip
                                 # modalMotion · popoverSurface · deferredModalOpen
                                 # sidebar-action-card (connection / nudge cards)
      hooks/
        useStableReference.ts    # useStableMap/Array/Set — keep derived identity for memo
        useMediaQuery.ts         # narrow viewport → inspector overlay
      layout/
        chromeLayout.ts          # sidebar / aux panel CSS vars + breakpoints
        AuxiliaryPanel.tsx       # split | rail | overlay right panel shell
      lib/
        desktop-root-gate.ts     # decideDesktopRoot: boot | login | app
        platform.ts              # hasPrimaryShortcutModifier (⌘/Ctrl)
        connection-card-policy.ts # when to show sidebar daemon offline card
        host-status.ts           # Ready · Local only / Linked / This Mac
        account-session.ts       # MinosSession + HostLinkState localStorage
        minos-cloud.ts           # /v1/auth/* + /v1/hosts/* + Hub IM HTTP
        im-cloud-sync.ts         # Hub shell upsert + user/agent_result Outbox (host_projection uplink)
        im-outbox.ts             # durable localStorage Outbox (user_message | agent_result | reaction_toggle | approval_resolve)
        im-cloud-inbound.ts      # Hub cold pull → TimelineMessage[] (no daemon append)
        hub-timeline.ts          # mergeHubAndLocalTimeline (Hub chat + local tool/git; same-id only)
        hub-cursors.ts           # per-topic topic_seq resume_after (localStorage)
        hub-realtime.ts          # Sync SM + account/conversation Subscribe + SnapshotRequired
        hub-digest-cache.ts      # Hub list digests: hydrate once, live patchOne
        conversation-list-merge.ts # daemon rows ∥ Hub digests (isHubImMode ≠ host-linked)
        im-hub-bridge.ts         # auth → realtime → timeline + rail digest patch
        # 协作气泡：docs/superpowers/specs/2026-08-02-hub-collaboration-message-ssot.md
        supabase.ts              # optional Supabase email/password IdP
        mock-data.ts
        toast.ts                 # sonner wrappers
        use-stick-to-bottom.ts
        scroll-restore.ts        # identity-based prepend restore
        enter-animation.ts       # gate list enter anim to new ids only
        list-identity.ts         # reuse row objects across quiet reloads
        transcript-history.ts    # transcript tail/older + hardMax trim
        message-history.ts       # conversation timeline tail/older + hardMax trim
        session-entity.ts        # L4 SessionEntity merge / status / attention filter
        session-list-projection.ts  # Entity → list membership projection (pure)
        desktop-inflight.ts      # single-flight loads + resume Sets (no window.__minos*)
        daemon-events.ts         # listen daemon://ingest|manager|conversation|push-status
        initial-render-ready.ts  # emit first-layout event so host can show window
        daemon.ts · agent-route.ts · …
    store/                       # L0–L6 工作区契约不变；chat reactions 不进 workspace
      ui-store.ts                # nav + drafts + replyTo + commandPaletteOpen
      account-store.ts           # Minos session + auto cloud ensure (sign-in / Online·Offline)
      workspace-store.ts         # thin create() + re-export useWorkspaceStore
      workspace/
        types.ts                 # WorkspaceState / ResourceFetchStatus / ProjectSession
        helpers.ts               # barrel re-export (stable import path)
        dto-map.ts               # Daemon DTO → UI + list patches
        transcript-merge.ts      # mergeTranscriptItems / tool lifecycle dedupe
        empty-workspace.ts       # empty caches, bootstrap flight, refresh timers
        reset-workspace-state.ts # module singleton teardown (Buzz resetCommunityState)
        mock-bundle.ts           # browser mock seed + KNOWN_AGENTS_FALLBACK
        projection.ts            # commitSessionEntity + hydrate sibling projection
        shared.ts                # quietRefresh / startNewAgentSession helpers
        create-actions.ts        # compose L1–L6 action factories
        connection.ts            # L1 bootstrap / livePush / refreshProjects
        conversation-list.ts     # L3a ConversationList
        timeline.ts              # L3a Timeline
        inspector.ts             # L3a Inspector
        session-list.ts          # L3b SessionList
        transcript.ts            # L3b Transcript
        attention.ts             # Attention queue (not badge)
        live-ingress.ts          # L5 applyIngest/Manager/Conversation
        agents-host.ts           # CLI inventory
        use-cases.ts             # L6 send / approvals / mutations
  src-tauri/                     # workspace 成员 minos-desktop
```

### 交互基础设施（桌面 UX 层）

| 能力 | 实现 |
|------|------|
| Approval / Question modal | Radix Dialog（Esc、focus trap、`aria-modal`）；决策路由 `features/work/lib/user-action.ts` |
| Work 三栏可拖拽 | `react-resizable-panels`；列表折叠时退回 rail + flex |
| 全局跳转 | ⌘/Ctrl+K → `CommandPalette` |
| 文本缩放 | `useWebviewZoomShortcuts` 挂在 **`App`**（boot + shell 始终生效）：⌘/Ctrl ±/0 调 `documentElement` rem（`minos:text-scale`）；Tauri webview zoom 固定为 1 |
| Daemon 连接反馈 | `ConnectionToasts` 监听 `connection.connected` 边沿；**disconnect 防抖 2s**（`connection-toast-policy`）。持久侧栏卡 `SidebarConnectionCard`（同防抖 + dismiss 至本 episode；Retry → bootstrap / Host） |
| Workspace 边界 reset | `resetWorkspaceModuleState`：bootstrap wipe / mock 路径统一清 timers、inflight、event bridge、reactions、composer ephemeral。**Project 切换不调用**。新模块单例必须登记于此 |
| Inspector 辅面板 | `AuxiliaryPanel`：`split`（resizable）/ `rail` / `overlay`（&lt;1100px 浮层+backdrop）；`SessionInspector` 统一壳 |
| Motion tokens | `modalMotion` / `popoverSurface` 统一 Dialog / Popover / Dropdown enter-exit |
| Modal backdrop | `modalBackdrop`：`backdrop-blur-[10px]` + `bg-black/[0.04] dark:bg-black/25`（偏透毛玻璃，能看清背景结构；禁止 `bg-ink/…` 当 scrim） |
| Transcript 滚动 | stick-to-bottom（rAF 合并 pin + wheel-up suppress re-follow/pin）+ tail/load-older；identity 锚点；top sentinel；`overscroll-y-none` |
| Timeline 分页 | 打开只拉 tail（`MESSAGE_PAGE_SIZE`）；`loadOlderMessages(beforeSeq)`；**hard + quiet** 均 `mergeMessagesQuietTail`（保留 older + 并发更新）；identity restore / following 不 restore |
| Transcript 打开 | 用 Entity/list `messageCount`（= last_seq）seek tail；ingest 抬升 `messageCount`；若 page `nextSeq` 仍指向更新事件则 catch-up 追到末尾，避免 stale last_seq 只看到中间窗 |
| 已处理 approval 不回潮 | 历史仍保留 `approval/request` 帧；`demoteResolvedApprovalItems` 在 merge/load 后若卡片后有 agent/user progress 则降为 `status` 并清 `requestId`。`resolve_approval` 写 durable `approval/resolved`；assembler 处理 resolved/timeout。Attention/`hasPendingApproval` 只信 working-set 窗口 |
| Timeline 渲染 | `features/chat`：窄 selector；`sortTimelineMessages`；`MarkdownText` memo + streaming plain；`MessageRow` memo；入场动画仅新增 id；`list-identity` 复用行对象 |
| MessageRow polish（Wave 2） | 同作者 ~10min 内隐藏 avatar/header；hover/focus-within action bar（Reply + React）；可选 day divider（`createdAtMs`）；保留 delivery/retry/session short-id/markdown/tool_summary |
| Reactions | **Hub IM 消息**：`POST …/reactions/toggle` + Durable `ConversationMessageReactionUpdated`（conversation topic；`reactions` 聚合 SSOT，`action` 仅动画）。Desktop `reaction-store` 在 authenticated + conversationId 时走 Hub。**Local workbench 消息**：daemon `LocalReaction*` / `chat_message_reactions` 路径。禁止 dual-write 同一气泡。 |
| Reply draft（Wave 2） | `ui-store.replyToMessageIdByConversation`；Composer 显示 reply chip；`sendMessage(..., { replyToMessageId })` 写乐观行；daemon append 尚未持久化 reply_to |
| Follow 迟滞 | unfollow 80px / re-follow 12px；wheel-up 后 ~320ms 禁止 re-follow 与 pin，减轻到底回弹再上滑的抢滚动 |
| Project tab 切换 | Conversations / Sessions / Board **keep-alive**（`hidden` + `inert`，不 unmount）；有缓存时 transcript **quiet append**；`useLayoutEffect` 首帧 pin 到底。**Sessions keep-alive 不得**在 Conversations 下 `selectSession(null)` / auto-select（否则抹掉 Inspector 点选 SessionDetail）。`loadTranscript` quiet peek **不 bump** generation，避免与 hard open 竞态丢弃整页；打开 session 必建 `transcriptsBySession` key 供 ingest 合并 |
| Session 状态 | **L4 `sessionsById`（SessionEntity）为 status / hasPendingApproval 唯一真相**；`sessionsByConversation` / `projectSessionsByProject` / Attention 经 `projectEntityIntoLists` 投影。hydrate：RQ 缓存 list 索引 → upsert Entity → `rowsFromEntities` 同步兄弟 list。无 `projectSessions` 全局镜像。SessionList **只** `listProjectSessions(projectId)`（`queryKeys.projectSessions`）；Inspector **只** `listSessions(conversationId)`（`queryKeys.inspectorSessions`）；Attention 打开再跨 project hydrate（**不**驱动侧栏 badge） |
| Live status | Manager / ingest 经 Entity 写入；`hasPendingApproval` 抬 `needs_approval`，manager 不得在 pending 时降级。Transcript 淘汰后审批 fallback 看 Entity。**`livePush===true` 时不 setInterval 盲刷**；pump 结束 emit `daemon://push-status` → `livePush=false` 恢复 Timeline/Sessions 降级 quiet poll |
| Attention badge | Σ `project.needsAttention`；bootstrap / refreshProjects 后 **quiet** `loadConversations` 全 known projects（有界并发），用 DTO `approvalCount`+unread 聚合；**不**常驻 Attention 队列 |
| ensureLoaded | per-key single-flight（`shared/lib/desktop-inflight.ts`）；Timeline hardMax 500 / Transcript hardMax 2000；resume 去重用模块 Set（禁止 `window.__minos*`） |
| Ghost Running 根因 | Daemon SQLite + `list_conversation_agent_sessions` 已是 `idle`，UI 缓存仍 `running`：漏推/ sticky elevation 后无对账。以 manager 事件 + Inspector 可见时的 `listSessions` 覆盖；禁止 live 下周期双 RPC |
| @agent / @profile 路由 | `@agent`：复用最近未关闭 session，新建时 convenience 绑定该 runtime **最新** host profile（`profile_id`）。`@agent#short`：续写。`@ProfileName` / `@p/<id>`：**始终新建**并传显式 `profile_id`。补全：runtime + profiles（hint: `profile · runtime`）+ continue sessions。解析：runtime 名优先；重名或非 clean token（含空白/`#`/`@`）profile 用 `@p/<id>`。Create form + daemon 拒绝非法 profile 名。Daemon `resolve_launch_options`：`agent` 必须匹配 profile runtime；explicit model/effort/instructions > profile > None。 |
| Timeline agent 身份 | Agent 气泡标题显示 `OpenCode #b15d06d4`（`sessionId` → short id，对齐 TUI `[OpenCode@short]`）；点击跳转 Sessions transcript |
| OpenCode 双气泡 | OpenCode 在同 `message_id` 上 `text_delta*` → `tool_call` → 再 `text_replace` 全量快照。Assembler 若只认 timeline **tail**，会在 tool 后再插一条相同正文。修复：`text_replace` 按 `message_id` 回写已有 assistant 气泡（Desktop `TranscriptAssembler` + TUI `ChatState`） |
| 重启后 Paused | Daemon 对 mid-turn 线程写 `suspended` + `needs_continue=1`。非 quiet **`loadInspector`** 会对最多一个 top-level `needsContinue` 调 `resume(autoContinue)`；打开 transcript 时 `resumeInterruptedSession` 同样路径 |
| OpenCode 僵尸 serve | **Bug**：`shutdown_instances` 原先只杀 Codex；OpenCode `serve`（`setpgid` 独立进程组）在 Desktop 退出时未 SIGTERM/SIGKILL，reparent 到 launchd（PPID=1），占满 `4096..=4106`。修复：shutdown 同时杀 opencode/gemini/grok 子进程；Desktop `RunEvent::ExitRequested` / `Exit` 幂等调用 `DaemonBridge::shutdown_managed` → `DaemonHandle::stop` |

## Rust 宿主 / Daemon 桥

`src-tauri` 是 root Cargo workspace 成员（`minos-desktop`），依赖 `minos-daemon`。

### 宿主插件与窗口生命周期

| 插件 / 模块 | 作用 |
|-------------|------|
| `tauri-plugin-single-instance` | 第二次启动 focus 已有 `main` 窗口（unminimize + show + focus），避免多进程各起一个 managed daemon |
| `tauri-plugin-window-state` | 持久化位置/尺寸/最大化；**排除 `VISIBLE` flag**，可见性由 reveal 插件控制 |
| `window_reveal`（inline plugin） | 窗口 `visible: false` 启动；geometry 连续 4 次一致 + 前端 `initial-render-ready`（或 5s 超时）后再 `show`/`set_focus`，消除 React 首帧前白闪 |
| `tracing` + `tracing-subscriber` | Host 入口 `init_tracing`（`RUST_LOG` / 默认 info filter）；reveal / single-instance / shutdown 关键路径打点 |
| `shutdown` + `ctrlc`（Unix） | `ExitRequested` / `Exit` / SIGINT·SIGTERM 幂等 `shutdown_managed`（**10s 超时**，避免 Cmd+Q 卡死）；信号在专用线程 `block_on` 后 `exit(130)`，不依赖 RunEvent |
| `commands/*` | 按 domain 拆分：`app` / `connection` / `projects` / `conversations` / `agents` / `sessions` / `approvals`；`lib.rs` 只负责 builder + lifecycle |

前端：`App` 的 `useLayoutEffect` + 根/`app` `ErrorBoundary.componentDidCatch` 幂等 `emitInitialRenderReady()`（`isTauriRuntime` 门控）。不与 bootstrap 完成绑定——BootScreen 或 crash UI 都可作为首帧表面。文本缩放 hook 同样挂在 `App`（见上表），避免 boot 阶段未挂载导致 `minos:text-scale` 晚应用。

Daemon RPC 前端入口：`shared/api/invoke.ts` 的 `invokeDaemon(command, args?)` 统一包一层 `DaemonInvokeError`（附 command 名）；非 Tauri runtime 直接抛错，由 store / RQ hooks 消化。

关闭：`AtomicBool` 幂等 `shutdown_managed`（10s `timeout`），覆盖 `ExitRequested`、`Exit`、Unix 信号。Host 进程内 tracing 以 desktop `init_tracing` / `RUST_LOG` 为 SSOT；托管 daemon **不**走 `minos_daemon::logging::init`（mars-xlog 仅独立 daemon 二进制）。

### 启动策略（对齐 TUI）

1. 读 `~/.minos/run/tui-daemon-rpc.json`（若存在）并 `minos_local_health`
2. 失败 / 无 discovery / stale port → **进程内托管** `DaemonHandle::start_with_local_rpc`（`127.0.0.1:0` + 写 discovery）
3. 连接使用 **binder 返回的 `local_rpc_url()`**（不依赖再读 discovery，避免竞态/陈旧端口）

### Teamwork MCP 注入（全 agent 共用）

会话协作依赖 `minos_teamwork` MCP：列消息、委派、等委托结果、回写 conversation 等。  
注入失败时 **Codex / Claude / Gemini / OpenCode / Grok 都无法做跨 agent 协作**，不是某一 CLI 的单独问题。

托管 daemon 启动时 `AgentGlue` → `enable_default_mcp()`：

1. 解析 MCP 入口（`MINOS_TEAMWORK_MCP_BIN` → 同目录 `minos-teamwork-mcp` → **当前 exe + hidden `__minos-teamwork-mcp`**）
2. 仅当 agent 绑定 **conversation_id**（`start_agent_in_conversation`）时，把 `minos_teamwork` 写入该 CLI 的 MCP 配置（各 agent 线格式不同，例如 OpenCode 为 `mcp.minos_teamwork.type=local`）
3. Desktop 进程名是 Tauri **`Minos`** / cargo **`minos-desktop`**：须识别为 sidecar host；`main.rs` 在 Tauri 启动前处理 `__minos-teamwork-mcp`（`minos_chat_store::mcp_server::serve_stdio`），以便 agent 子进程能 `spawn(current_exe, …)` 起 MCP
4. 若 locator 找不到可执行入口，或 host 未实现 sidecar 子命令，则 **静默跳过注入**（runtime warn）；agent 仍可单聊编码，但 **teamwork 工具集为空**

`DaemonHandle` 由 `DaemonBridge` 持有；`connect` 经全局锁串行，避免 StrictMode 双启动。

### 用户交互请求（approval / question）

Session transcript 组装（`TranscriptAssembler`）消费 daemon 投影后的 `UiEventMessage`：

| 事件 | 卡片 kind | 回复 RPC |
|------|-----------|----------|
| `approval/request`（Codex / ACP permission / Grok plan） | `approval` | `minos_local_approval_decision` |
| `approval/request` + `x.ai/ask_user_question` | `question` | 同上（`outcome` + `answers`） |
| `opencode/permission.updated` | `approval`（method `opencode/permission`） | `minos_local_respond_opencode_permission` |
| `opencode/question.asked` | `question`（method `opencode/question`） | `minos_local_respond_opencode_question` |

UI：`SessionsView` thin shell + `ui/ApprovalModal` / transcript chips；决策经 `lib/user-action.ts`。Claude 与 Codex/Gemini/Grok/OpenCode 共用同一 `approval/request` 路径（host 将 Claude control 权限 park 为 `PendingApprovalTarget::ClaudeControl`）。

| Command | 作用 |
|---------|------|
| `daemon_connect` | 发现 → 失败则 managed start → 连接 |
| `daemon_status` | `connected` / `managed` / `endpoint`（实现细节） |
| `daemon_list_*` / `append` / `create` | 同 TUI `minos_local_*` |
| `daemon_toggle_message_reaction` | `minos_local_toggle_conversation_message_reaction`（本机 durable） |
| `daemon_create_project` | `minos_local_create_project`（选文件夹后创建） |
| `daemon_resume_session` | `minos_local_resume_session`（reattach；可选 `autoContinue`） |
| `daemon_send_user_message` | 发消息前应先 resume(reattach-only) |

### Host 产品状态（UI，非 wire 协议）

三层状态不要混成 `Daemon · managed`：

| 层 | 含义 | UI 落点 | v1 行为 |
|----|------|---------|---------|
| **Runtime (A)** | 本机 daemon 是否可用 | 侧栏品牌区圆点 + Ready/Unavailable | 连上 daemon → Ready |
| **Link (B)** | backend/relay 协作链路 | 品牌区 `· Local only` / `· Linked`；Host 页 | 仅本地 → **Local only**（不是 Offline） |
| **Project locus (C)** | 项目挂在哪台 Host | Project header pill / 列表（远程时） | **This Mac** |

派生逻辑：`src/shared/lib/host-status.ts` → `deriveHostPresence` / `projectHostLabel`。

- 侧栏：`Ready · Local only`（绿）/ `Unavailable`（红）/ `Preview`（mock，琥珀）；点击进 Host
- Project 顶栏 pill：宿主标签 `This Mac`（替代原 `MANAGED`）；多设备后可显示设备名
- Host 页（高密度）：顶栏状态 chip + Reconnect；一块 **Runtime** 键值表（Machine / Status / Link / Process）；**Pairing** 单行占位；**Diagnostics** 默认折叠（endpoint / managed）
- 不重复 Summary 卡；不占大块空 QR；`managed` 仅诊断区

**Session 复用与 resume：**

- 无 `@agent#shortId` 且非 profile mention 时：同 conversation + 同 agent 复用最近非 Closed top-level session；否则 `start_agent_in_conversation`（bare `@agent` 新建时传最新 profile 的 `profile_id`）。
- `@ProfileName` / `@p/<id>`：不复用 session，始终 `start_agent_in_conversation` + 显式 `profile_id`。
- `sendMessage`：`resumeSession(id, false)` 再 `sendUserMessage`（用户文本优先于 CONTINUE）。
- 非 quiet `loadInspector`：对最多一个 `needsContinue` top-level session 调 `resumeSession(id, true)`。
- Session 状态 pill：`suspended` → “Paused”（不再误标为 needs_approval）。
- **Idle 重启不该变 Paused：** daemon 停机/脏恢复时，**仅 mid-flight** 线程落 `suspended`；原本 `idle` 保持 `idle`。对历史错误行：`Suspended{DaemonRestart}` + `needs_continue=false` 在 bridge 仍映射为 UI `idle`。

空项目态：主内容区为全幅 **Create project**（大 +），系统文件夹选择器 → create_project → 刷新列表并选中。

### Project views

`Conversations | Sessions | Board`

| View | 作用 |
|------|------|
| Conversations | 协作主时间线 + @agent |
| Sessions | Project 内 agent runs：**按 Conversation 折叠分组** + full transcript（`read_session_raw_history`） |
| Board | Conversation 俯视图（由 progress + session 运行态派生） |

**Sessions 左侧列表（Codex-style）：**

- 顶层 = **Conversation** 文件夹（可折叠）；组内 = 该对话下的 top-level agent sessions，subagent 缩进挂在 parent 下
- 组排序 = 组内最近 `lastTsMs` DESC；header 显示 live 数 / attention / session count
- 每个 session 显示状态 pill；`running` / `needs_approval` 用 **spinner** 表示执行中
- 选中 session 时自动展开其 Conversation；切换 project 重置折叠态
- 分组逻辑：`src/shared/lib/session-list-group.ts`

深链：Conversation inspector / 气泡上的 session → `openSessionTranscript` 切到 Sessions 并选中；Sessions 顶栏 **Back to conversation** 回 Conversations。

### Session transcript 消费（与 TUI 同契约）

Desktop **Sessions** 详情与 TUI AgentDetail 共用 daemon 投影，不解析 CLI 原生事件：

```text
minos_local_read_session_raw_history / subscribe_ingest
  → LocalIngestFrame { ui_events: Vec<UiEventMessage> }
  → TranscriptAssembler (src-tauri/daemon.rs)   // 对齐 ChatState 语义
  → TranscriptItemDto { kind, text, title, detail, … }
  → features/work/ui/TranscriptItemView (React; SessionsView shell)
```

| UiEventMessage | TranscriptItem.kind | UI |
|----------------|---------------------|-----|
| TextDelta (user/assistant) | `user` / `assistant` | `❯` 前缀 / Markdown |
| ReasoningDelta | `reasoning` | 可折叠 Thought |
| ToolCallPlaced → Completed | `tool` → `tool_result`/`tool_error` | 动词 + bare target；**Edit/patch 默认展开** `DiffView`（unified / apply_patch 着色，非整页编辑器） |
| Subagent* | `status` | 一行 subagent 状态 |
| Raw(approval/*) | `approval` | 审批卡 |
| Raw(其它) | 丢弃 | 不进 timeline |

**Conversation 主时间线**：Linked 会话以 **Hub 协作气泡** 为 SSOT；本地 daemon `chat_messages` 提供 tool/git 与未上行 agent-result 缺口填充（**同 id 相等**，禁止 body 软去重）。Desktop-native 回合：daemon 本地 `agent-result:…` + Outbox **`host_projection`** 上行（规范 id）；Mobile `client_live` 回合由 Hub projector 写气泡。见 [Hub 协作消息 SSOT](superpowers/specs/2026-08-02-hub-collaboration-message-ssot.md) 与 [IM Reliability](superpowers/specs/2026-08-03-im-reliability-program/README.md)。  
**Session transcript** 仍只认 Host（user / assistant / tool / reasoning），**不含**把全过程 tool 流水写进协作气泡——与 TUI 分层一致。

### Conversation timeline（messenger 气泡）

| 项 | 行为 |
|----|------|
| 排序键 | 服务端 `message_seq` ASC（bridge reverse + 前端 `sortTimelineMessages` 防御）；**不用** `createdAtMs` 排序 |
| 字段 | `messageSeq` / `messageId` / `replyToMessageId` / `mentions` / `delegationId` 经 Tauri DTO 贯通 |
| 时钟 | **epoch ms SSOT**（`createdAtMs` / 列表 `updatedAtMs`）。气泡用 `formatLocalClock`（本地 `HH:mm`）；列表/Board 用 `formatListActivityTime`（今天时钟 / Yesterday / 周几 / 日期）在 **render 时**格式化，**禁止**把相对时间串写进 store |
| 列表 last activity | Hub IM：`max(hub.lastMessageAtMs, daemon.updatedAtMs)`；preview 跟随较新一侧（防 host_projection 滞后钉死旧 digest）。本机发送乐观更新 rail。**Recall**：从已打开时间线剩余气泡重算 last activity，禁止用被撤回消息的 `createdAtMs` 覆盖（会倒退列表时钟）；无窗口时保留 prev digest。Account digest 缺 `at_ms` 时 **不** 用 `Date.now()` 伪造。Daemon 物理 `conversations.updated_at_ms` **仅** message upsert 写入；title/git/session count 不 bump；list SELECT 用 top-level `MAX(created_at_ms)` 作 last-activity SSOT |
| 正文 | user + agent 气泡用 `MarkdownText`：`react-markdown` + `remark-gfm`（标题/列表/表格/fence/粗斜体/链接；默认不渲染 raw HTML） |
| 引用 | 有 `replyToMessageId` 时显示短引用条（委托 result → request 等） |
| Optimistic | 本地 `sending` 行立刻用本地时钟；下一次 list / Hub merge 以 id 对齐服务端真相 |
| Live | `daemon://conversation` → debounce re-list；仍以 `message_seq` 序展示 |
| Subagent | 主时间线不展示 subagent session 消息（daemon list 过滤）；细节在 Sessions transcript |

### Agent transcript UX（对齐 TUI AgentDetail）

Sessions 主区是 **Grok-style 日志 transcript** + 右侧 **session summary**（类 OpenCode 右栏）：

| 项 | 行为 |
|----|------|
| User | `❯ ` 前缀 + 正文（无右侧气泡） |
| Assistant | 裸 markdown（无 avatar / role 标签） |
| Reasoning | `Thinking…` / `Thought`；展开后 `│` quote bar |
| Tool | `{Verb} {bare target}`（`Read path` / `Ran cmd`）；展开看 detail；错误后缀 `failed`；diff 显示 `+n/-m` |
| Bridge 字段 | `title` = tool name；`text` = bare target；`detail` = args→output |
| Approvals | 可操作卡片 + modal（Allow / Deny / plan 三态） |
| Summary 面板 | 从 transcript **派生**（`session-summary.ts`）：edit tool 路径 + 累计 `-N +M`；header 可折叠；**token 暂不展示**（各 CLI 格式不一，ui-protocol 无统一 usage 投影） |

**Stick-to-bottom（`useStickToBottom`，对齐 TUI `auto_scroll`）：**

- 默认 following：内容增长（含 in-place stream 与 ResizeObserver）时 **即时** `scrollTop = scrollHeight`（不用 smooth 排队）。
- 用户上滚离开底部阈值（~80px）→ unfollow；回到底部或点 **Jump to latest** → re-follow。
- 未 following 时 **禁止** 程序化滚到底（避免与用户读历史冲突）。
- 内容未溢出（`scrollHeight ≤ clientHeight`）时：wheel 手势 **不** unfollow，scroll 事件也保持 following，避免短列表误显 **Jump to latest**。
- Timeline 共用同一套 follow 语义；顶栏可显示 `[manual scroll]`。

前端 `workspace-store`：Tauri 下走 bridge；浏览器-only 仍 mock；托管/连接最终失败才 mock。

### 声明式数据加载（导航 vs 资源）

**原则：** 导航 store 只改 id；View 用 props/`key` 做 init load；渲染订 data + per-resource status。

| Surface | 导航 | View init | Status |
|---------|------|-----------|--------|
| App boot | — | connect + listProjects + listClis + **subscribe pumps**（single-flight） | `error`（仅连接）；`bootEpoch++`、`livePush=true` |
| Conversation list | `projectId` props/`key` | **`ConversationList`** → `loadConversations(projectId)`（依赖 `bootEpoch`） | `conversationsStatusByProject` |
| Timeline | `conversationId` props/`key` | `loadTimeline`（`listMessages` only；依赖 `bootEpoch`） | `timelineStatusByConversation` |
| Inspector | `conversationId` + `detailsOpen` | `loadInspector` → RQ `fetchQuery(inspectorSessions)` + Entity；关栏不拉 | `inspectorStatusByConversation` |
| Sessions list | `projectId` props/`key` | `loadProjectSessions` → RQ `fetchQuery(projectSessions)` + Entity upsert；UI=`SessionListPane`/`VirtualizedList` | `projectSessionsStatusByProject` |
| Transcript | `sessionId` props/`key` | `loadTranscript`（**不** RQ / **不** virtua） | `transcriptStatusBySession` |
| Attention | — | `loadAttentionSessions` | `attentionStatus` |
| Agents | CLI cards + Create agent dialog + host profiles | `loadClis` + profile/model RPCs | `clisStatus` |
| Board | `projectId` | 吃 conversation list 缓存 | progress 单一真相（无 local override） |

**启动顺序：** `bootstrap` → projects → `WorkView` 用 `resolvedProjectId`（`ui.projectId` 或 `projects[0]`）→ `ConversationList` init load → 列表 `ready` 后 auto-select conversation → `Timeline` `loadTimeline`；右栏打开时 `SessionInspector` `loadInspector`。

**计数一致性：** 顶栏 conversation 数在 list `ready` 后只信 store 列表长度；禁止用未加载时的 `project.conversationCount` 掩盖空列表。

### Live push（对齐 TUI）

与 TUI 相同的三条 daemon JSON-RPC subscription，经 Tauri `emit` 到 webview：

| Wire | 事件 | Store |
|------|------|-------|
| `minos_local_subscribe_ingest` | `daemon://ingest` | `applyIngestEvent` — **有** Transcript 工作集 key 才 merge items；无 key 只抬 `needs_approval`（不 `?? []` 建窗） |
| `minos_local_subscribe_manager_events` | `daemon://manager` | `applyManagerEvent` — session status（**不**用 running 覆盖 needs_approval） |
| `minos_local_subscribe_conversation_events` | `daemon://conversation` | `applyConversationEvent` — 防抖 ~200ms；有 Timeline 工作集时 quiet **`loadTimeline`（仅 listMessages）** + quiet `loadConversations`（`staleTime:0` 刷新 rail preview）；无 entry 只 mark `timelineDirty`、零 RPC。**`loadTimeline` 始终 `mergeMessagesQuietTail`**，避免 hard open 与 quiet 竞态把已到达的新消息冲掉 |
| pump arm / death | `daemon://push-status` `{ live }` | `livePush` 门闸；`live=false` 时 View 降级 poll 重新启用 |

- Hydrate 仍用 `list*` / `read_transcript`；**live 路径靠推送**。
- `livePush===true` 时 **不** setInterval 盲刷 Timeline / project sessions / transcript；仅 `livePush===false` 时降级 quiet poll（Timeline / Sessions list / Transcript append）。
- 已删除打包式 `loadConversationDetail`（双 RPC）；Timeline ∥ Inspector 独立加载（`timelineStatusByConversation` / `inspectorStatusByConversation`）。
- Session status：`sessionsById` + `mergeSessionEntity` / `patchSessionEntity` / `commitSessionEntity`；list 缓存为投影（`lib/session-list-projection.ts`）。
- In-flight：`lib/desktop-inflight.ts`（`singleFlightLoad`、resume Sets）；conversation dirty debounce 用模块级 `conversationRefreshTimers` Map。
- Store 拆分：`store/workspace-store.ts` 薄入口；`store/workspace/*` 按 L1–L6 模块（app 仍 `import { useWorkspaceStore } from "@/store/workspace-store"`）。

其它：

- **草稿** `ui-store.draftByConversationId`（按 conversation 隔离）
- **上次会话** `lastConversationByProject`（切 project 可恢复）
- **actionError** 操作失败；**error** 仅 boot/连接
- Project `needsAttention` / `runningAgents` 由 conversation 列表聚合回写

### Conversation 元数据

| 字段 | 含义 | 交互 |
|------|------|------|
| `title` | 可改名 | 顶栏标题双击内联编辑 → `update_conversation` |
| `priority` | `high` / `medium` / `low` / 未设置 | 顶栏标签点击循环 |
| `progress` | `todo` / `in_progress` / `in_review` / `done` | 顶栏标签点击循环；Board 移动写入 progress |
| `branch` / `worktree_path` | **创建时** git 快照 | 只读 chip；不跟随后续 checkout |
| Board 列 | 派生，非独立任务系统 | `done` 优先；`needs_you` 来自 suspended/approval 运行态（progress 仍为 `in_progress`） |

新建会话：`ProjectHeader` → **Create conversation** 弹窗（对标 Buzz create-channel）配置 title / optional priority / **required agent roster**，再 `create_conversation`（protocol：`priority?` + `agents[]`）写入 `conversation_agent_members`；progress 默认 `todo`。**成员资格是 @mention / start 的 SSOT**：空 roster 时 picker 为空且 send 拒绝；仅 roster 内 runtime（及其 profile / open session）可被 @；`start_agent_in_conversation` 对非成员返回错误。创建时**不**预启动 session（懒启动：首次 @ 或 bare send 再 start）。首次 `start_agent_in_conversation` 时若仍为 `todo` 则自动升为 `in_progress`。从 roster 移除：`daemon_remove_conversation_agent` → `minos_local_remove_conversation_agent`，关闭该 agent 悬挂 sessions（`roster_removed`）并取消相关 running delegations；被移除 agent 的 MCP 调用被拒绝。

### Agent 运行态与审批（Desktop 缺口修复）

| 现象 | 原因 | 行为 |
|------|------|------|
| Conversation 时间线只有用户消息 | 主时间线只读 `chat_messages`；tool/plan 在 thread `events` | 右侧 Session inspector / Sessions 页看 transcript；**不在** timeline 插运行中 banner |
| Conversation 气泡误显示 Approval required | 曾用 body 包含 `"approval"` / `Permission:` 推断 kind | **禁止** 从对话正文推断；approval 仅 session transcript 的 reverse-request（`request_id`） |
| Session 一直 Running 无更新 | 无 live 事件时只能靠 poll | **manager + ingest 推送**；transcript 初始 **tail 窗口** hydrate |
| Grok 卡住 | `x.ai/exit_plan_mode` plan approval 待决策 | ingest `approval/request` → UI `needs_approval` + transcript approval 卡 + `minos_local_approval_decision` |
| “View plan” 被截断（历史） | Desktop bridge 曾对 `planContent` 做 6000 字符 `truncate_str` | **plan body 完整透传**到 modal（`detail` 不截断）；其它 permission 参数仍可截断 |
| 状态仍是 running | Grok 等审批时 runtime state 仍为 Running | **ingest** 抬升 `needs_approval`；manager 的 running **不覆盖** elevation |
| Quiet poll 闪烁（历史） | 盲刷 listSessions + 误降级 | **默认 live push**；poll 仅 `livePush=false` 降级 |

**Session status 真相（Desktop）：**

- Daemon `thread_status_label` / manager：**不**发出 `needs_approval`（Running 含等审批）。
- UI 派生：ingest / transcript 中 pending approval → `needs_approval`。
- Live 路径 = 推送；quiet poll 不再作为常态。

查看 agent 详情：右侧 Inspector 点 session → **Open full transcript**，或顶栏 **Sessions** 标签。

## 开发命令

```bash
just dev-desktop       # pnpm tauri dev
just dev-desktop-ui    # 仅 Vite http://localhost:1420
just build-desktop
just check-desktop     # full gates: tsc + tests + biome lint + file-sizes (= pnpm check:all)
```

## 非目标（当前）

- 真实 auth / relay / daemon
- 客服 Inbox 语义、Jira 式任务系统
- 与 `apps/web` 共享组件
- 删除 TUI
