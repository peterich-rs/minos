# Desktop 应用 (apps/desktop) 架构文档

> Host 端桌面控制台：Tauri + React，短期目标是 **TUI 能力的可视化**（Project → Conversation → Agent session），可选 Project Board 俯视图。

## 概述

| 项 | 值 |
|----|-----|
| 源码路径 | `apps/desktop/` |
| 产品定位 | Host 本机指挥台（对标 TUI，不做登录/移动端） |
| 当前阶段 | **UI mock**：完整 IA + mock 数据，未接 daemon |
| 视觉 | 暖色多栏（参考 `res/desktop.jpeg` 气质，非客服 Inbox 语义） |
| 产品 spec | [2026-07-18-desktop-product-experience.md](superpowers/specs/2026-07-18-desktop-product-experience.md) |

## 技术栈

| 层 | 技术 | 用途 |
|----|------|------|
| 桌面壳 | Tauri 2 | WebView 窗口 + Rust 宿主 |
| UI | React 19 + TypeScript | 多栏产品界面 |
| 构建 | Vite 7 | dev/build |
| 样式 | Tailwind CSS 3.4 + `tailwindcss-animate` | 设计 token + enter/exit utilities |
| 交互原语 | Radix (Dialog / Dropdown / Tooltip / Slot) + CVA | shadcn 模式 headless 组件（`components/ui/*`） |
| 分栏 | `react-resizable-panels` v4 (`Group`/`Panel`/`Separator`) | Work：list \| timeline \| inspector |
| 命令面板 | `cmdk` + Radix Dialog | 全局 ⌘K 跳转 project / conversation / session / nav |
| Toast | `sonner` | 发送失败、daemon 断连/恢复、审批结果 |
| 动效 | `motion`（layout 导航指示）+ CSS `duration-150/200` | 克制动效；尊重 `prefers-reduced-motion` |
| 长列表 | `@tanstack/react-virtual` | Sessions transcript 虚拟化（与 stick-to-bottom 联调） |
| 状态 | Zustand 5 | nav / project / conversation / board |
| 图标 | Lucide React | 导航与工具栏 |
| 本机 API | `@tauri-apps/api` | `invoke` → Rust |

## 信息架构

```text
Sidebar
  Work | Attention | Agents | Host
  Projects list (select → Work)

Work → Project
  header: Conversations | Board
  Conversations view:
    list | timeline + @input | session inspector
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
| Agents | CLI inventory + personalized profiles | `list_clis`, `list_models`, agent profile CRUD; start session passes fixed `model` / `reasoning_effort` |
| Host | Ready / Local only / Linked + 诊断 | status + pairing |

## 目录结构

```
apps/desktop/
  src/
    lib/host-status.ts   # Ready · Local only / Linked / This Mac
    lib/mock-data.ts
    lib/toast.ts         # sonner wrappers
    lib/use-stick-to-bottom.ts
    store/ui-store.ts    # + commandPaletteOpen
    components/
      ui/                # Button · Dialog · Tooltip · Dropdown · Toaster (Radix+CVA)
      Avatar.tsx · StatusPill.tsx
      shell/
        AppShell.tsx · Sidebar.tsx · CommandPalette.tsx · ConnectionToasts.tsx
        WorkView.tsx · ProjectHeader.tsx
        ConversationList.tsx · Timeline.tsx · SessionInspector.tsx
        SessionsView.tsx · VirtualTranscriptList.tsx
        ProjectBoard.tsx
        AttentionView.tsx · AgentsView.tsx · HostView.tsx
  src-tauri/          # workspace 成员 minos-desktop
```

### 交互基础设施（桌面 UX 层）

| 能力 | 实现 |
|------|------|
| Approval / Question modal | Radix Dialog（Esc、focus trap、`aria-modal`） |
| Work 三栏可拖拽 | `react-resizable-panels`；列表折叠时退回 rail + flex |
| 全局跳转 | ⌘/Ctrl+K → `CommandPalette` |
| Daemon 连接反馈 | `ConnectionToasts` 监听 `connection.connected` 边沿 |
| Transcript 性能 | `VirtualTranscriptList` + 既有 stick-to-bottom / load-older |

## Rust 宿主 / Daemon 桥

`src-tauri` 是 root Cargo workspace 成员（`minos-desktop`），依赖 `minos-daemon`。

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

UI：`SessionsView` 审批 modal / 选项 chips。Claude 未接。

| Command | 作用 |
|---------|------|
| `daemon_connect` | 发现 → 失败则 managed start → 连接 |
| `daemon_status` | `connected` / `managed` / `endpoint`（实现细节） |
| `daemon_list_*` / `append` / `create` | 同 TUI `minos_local_*` |
| `daemon_create_project` | `minos_local_create_project`（选文件夹后创建） |
| `daemon_resume_thread` | `minos_local_resume_thread`（reattach；可选 `autoContinue`） |
| `daemon_send_user_message` | 发消息前应先 resume(reattach-only) |

### Host 产品状态（UI，非 wire 协议）

三层状态不要混成 `Daemon · managed`：

| 层 | 含义 | UI 落点 | v1 行为 |
|----|------|---------|---------|
| **Runtime (A)** | 本机 daemon 是否可用 | 侧栏品牌区圆点 + Ready/Unavailable | 连上 daemon → Ready |
| **Link (B)** | backend/relay 协作链路 | 品牌区 `· Local only` / `· Linked`；Host 页 | 仅本地 → **Local only**（不是 Offline） |
| **Project locus (C)** | 项目挂在哪台 Host | Project header pill / 列表（远程时） | **This Mac** |

派生逻辑：`src/lib/host-status.ts` → `deriveHostPresence` / `projectHostLabel`。

- 侧栏：`Ready · Local only`（绿）/ `Unavailable`（红）/ `Preview`（mock，琥珀）；点击进 Host
- Project 顶栏 pill：宿主标签 `This Mac`（替代原 `MANAGED`）；多设备后可显示设备名
- Host 页（高密度）：顶栏状态 chip + Reconnect；一块 **Runtime** 键值表（Machine / Status / Link / Process）；**Pairing** 单行占位；**Diagnostics** 默认折叠（endpoint / managed）
- 不重复 Summary 卡；不占大块空 QR；`managed` 仅诊断区

**Session 复用与 resume：**

- 无 `@agent#shortId` 时：同 conversation + 同 agent 复用最近非 Closed top-level session；否则 `start_agent_in_conversation`。
- `sendMessage`：`resumeThread(id, false)` 再 `sendUserMessage`（用户文本优先于 CONTINUE）。
- `loadConversationDetail`：对最多一个 `needsContinue` top-level session 调 `resumeThread(id, true)`。
- Session 状态 pill：`suspended` → “Paused”（不再误标为 needs_approval）。
- **Idle 重启不该变 Paused：** daemon 停机/脏恢复时，**仅 mid-flight** 线程落 `suspended`；原本 `idle` 保持 `idle`。对历史错误行：`Suspended{DaemonRestart}` + `needs_continue=false` 在 bridge 仍映射为 UI `idle`。

空项目态：主内容区为全幅 **Create project**（大 +），系统文件夹选择器 → create_project → 刷新列表并选中。

### Project views

`Conversations | Sessions | Board`

| View | 作用 |
|------|------|
| Conversations | 协作主时间线 + @agent |
| Sessions | Project 内 agent runs：**按 Conversation 折叠分组** + full transcript（`read_thread_raw_history`） |
| Board | Conversation 俯视图（由 progress + session 运行态派生） |

**Sessions 左侧列表（Codex-style）：**

- 顶层 = **Conversation** 文件夹（可折叠）；组内 = 该对话下的 top-level agent sessions，subagent 缩进挂在 parent 下
- 组排序 = 组内最近 `lastTsMs` DESC；header 显示 live 数 / attention / session count
- 每个 session 显示状态 pill；`running` / `needs_approval` 用 **spinner** 表示执行中
- 选中 session 时自动展开其 Conversation；切换 project 重置折叠态
- 分组逻辑：`src/lib/session-list-group.ts`

深链：Conversation inspector / 气泡上的 session → `openSessionTranscript` 切到 Sessions 并选中；Sessions 顶栏 **Back to conversation** 回 Conversations。

### Session transcript 消费（与 TUI 同契约）

Desktop **Sessions** 详情与 TUI AgentDetail 共用 daemon 投影，不解析 CLI 原生事件：

```text
minos_local_read_thread_raw_history / subscribe_ingest
  → LocalIngestFrame { ui_events: Vec<UiEventMessage> }
  → TranscriptAssembler (src-tauri/daemon.rs)   // 对齐 ChatState 语义
  → TranscriptItemDto { kind, text, title, detail, … }
  → SessionsView TranscriptItemView (React)
```

| UiEventMessage | TranscriptItem.kind | UI |
|----------------|---------------------|-----|
| TextDelta (user/assistant) | `user` / `assistant` | `❯` 前缀 / Markdown |
| ReasoningDelta | `reasoning` | 可折叠 Thought |
| ToolCallPlaced → Completed | `tool` → `tool_result`/`tool_error` | 动词 + bare target；**Edit/patch 默认展开** `DiffView`（unified / apply_patch 着色，非整页编辑器） |
| Subagent* | `status` | 一行 subagent 状态 |
| Raw(approval/*) | `approval` | 审批卡 |
| Raw(其它) | 丢弃 | 不进 timeline |

**Conversation 主时间线**只读 `chat_messages`（user / agent-result / post_conversation_update），**不含** session 全过程 tool 流水——与 TUI conversation 层一致。展示风格可借鉴 Grok，但数据路径对所有 CLI 统一。

### Conversation timeline（messenger 气泡）

| 项 | 行为 |
|----|------|
| 排序键 | 服务端 `message_seq` ASC（bridge reverse + 前端 `sortTimelineMessages` 防御）；**不用** `createdAtMs` 排序 |
| 字段 | `messageSeq` / `messageId` / `replyToMessageId` / `mentions` / `delegationId` 经 Tauri DTO 贯通 |
| 正文 | user + agent 气泡用 `MarkdownText`：`react-markdown` + `remark-gfm`（标题/列表/表格/fence/粗斜体/链接；默认不渲染 raw HTML） |
| 引用 | 有 `replyToMessageId` 时显示短引用条（委托 result → request 等） |
| Optimistic | 本地 `pending` 行；下一次 list 整表替换服务端真相并丢弃 pending |
| Live | `daemon://conversation` → debounce re-list；仍以 `message_seq` 序展示 |
| Subagent | 主时间线不展示 subagent thread 消息（daemon list 过滤）；细节在 Sessions transcript |

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
| Timeline | `conversationId` props/`key` | `loadConversationDetail`（依赖 `bootEpoch`） | `detailStatusByConversation` |
| Sessions list | `projectId` props/`key` | `loadProjectSessions`（依赖 `bootEpoch`） | `projectSessionsStatusByProject` |
| Transcript | `sessionId` props/`key` | `loadTranscript` | `transcriptStatusByThread` |
| Attention | — | `loadAttentionSessions` | `attentionStatus` |
| Agents | CLI cards + Create agent dialog + host profiles | `loadClis` + profile/model RPCs | `clisStatus` |
| Board | `projectId` | 吃 conversation list 缓存 | progress 单一真相（无 local override） |

**启动顺序：** `bootstrap` → projects → `WorkView` 用 `resolvedProjectId`（`ui.projectId` 或 `projects[0]`）→ `ConversationList` init load → 列表 `ready` 后 auto-select conversation → `Timeline` load detail。

**计数一致性：** 顶栏 conversation 数在 list `ready` 后只信 store 列表长度；禁止用未加载时的 `project.conversationCount` 掩盖空列表。

### Live push（对齐 TUI）

与 TUI 相同的三条 daemon JSON-RPC subscription，经 Tauri `emit` 到 webview：

| Wire | 事件 | Store |
|------|------|-------|
| `minos_local_subscribe_ingest` | `daemon://ingest` | `applyIngestEvent` — merge transcript；pending approval → `needs_approval` |
| `minos_local_subscribe_manager_events` | `daemon://manager` | `applyManagerEvent` — session status（**不**用 running 覆盖 needs_approval） |
| `minos_local_subscribe_conversation_events` | `daemon://conversation` | `applyConversationEvent` — 防抖 quiet re-list messages |

- Hydrate 仍用 `list*` / `read_transcript`；**live 路径靠推送**。
- `livePush===true` 时关闭 Timeline/Sessions 的 2–2.5s quiet poll（仅无推送时降级）。

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

新建会话：一键创建（默认 title `New conversation`，progress `todo`），不弹配置窗。首次 `start_agent_in_conversation` 时若仍为 `todo` 则自动升为 `in_progress`。

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
just check-desktop
```

## 非目标（当前）

- 真实 auth / relay / daemon
- 客服 Inbox 语义、Jira 式任务系统
- 与 `apps/web` 共享组件
- 删除 TUI
