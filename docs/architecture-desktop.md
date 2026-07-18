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
| 样式 | Tailwind CSS 3.4 | 设计 token |
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
| Agents | CLI detect 假数据 | `detect_clis` |
| Host | relay/daemon 文案 | status + pairing |

## 目录结构

```
apps/desktop/
  src/
    lib/mock-data.ts
    store/ui-store.ts
    components/
      Avatar.tsx · StatusPill.tsx
      shell/
        AppShell.tsx · Sidebar.tsx
        WorkView.tsx · ProjectHeader.tsx
        ConversationList.tsx · Timeline.tsx · SessionInspector.tsx
        ProjectBoard.tsx
        AttentionView.tsx · AgentsView.tsx · HostView.tsx
  src-tauri/          # 独立 Cargo 包 minos-desktop（非 root workspace）
```

## Rust 宿主 / Daemon 桥

`src-tauri` 是 root Cargo workspace 成员（`minos-desktop`），依赖 `minos-daemon`。

### 启动策略（对齐 TUI）

1. 读 `~/.minos/run/tui-daemon-rpc.json`（若存在）并 `minos_local_health`
2. 失败 / 无 discovery / stale port → **进程内托管** `DaemonHandle::start_with_local_rpc`（`127.0.0.1:0` + 写 discovery）
3. 连接使用 **binder 返回的 `local_rpc_url()`**（不依赖再读 discovery，避免竞态/陈旧端口）
4. `DaemonHandle` 由 `DaemonBridge` 持有；`connect` 经全局锁串行，避免 StrictMode 双启动

| Command | 作用 |
|---------|------|
| `daemon_connect` | 发现 → 失败则 managed start → 连接 |
| `daemon_status` | `connected` / `managed` / `endpoint` |
| `daemon_list_*` / `append` / `create` | 同 TUI `minos_local_*` |
| `daemon_create_project` | `minos_local_create_project`（选文件夹后创建） |
| `daemon_resume_thread` | `minos_local_resume_thread`（reattach；可选 `autoContinue`） |
| `daemon_send_user_message` | 发消息前应先 resume(reattach-only) |

**Session 复用与 resume：**

- 无 `@agent#shortId` 时：同 conversation + 同 agent 复用最近非 Closed top-level session；否则 `start_agent_in_conversation`。
- `sendMessage`：`resumeThread(id, false)` 再 `sendUserMessage`（用户文本优先于 CONTINUE）。
- `loadConversationDetail`：对最多一个 `needsContinue` top-level session 调 `resumeThread(id, true)`。
- Session 状态 pill：`suspended` → “Paused”（不再误标为 needs_approval）。

空项目态：主内容区为全幅 **Create project**（大 +），系统文件夹选择器 → create_project → 刷新列表并选中。

### Project views

`Conversations | Sessions | Board`

| View | 作用 |
|------|------|
| Conversations | 协作主时间线 + @agent |
| Sessions | Project 内所有 agent runs 列表 + full transcript（`read_thread_raw_history`） |
| Board | Conversation 俯视图（由 progress + session 运行态派生） |

深链：Conversation inspector / 气泡上的 session → `openSessionTranscript` 切到 Sessions 并选中；Sessions 顶栏 **Back to conversation** 回 Conversations。

### Agent transcript UX（对齐 TUI AgentDetail）

Sessions 主区是 **Grok-style 日志 transcript**，不是 messenger 气泡：

| 项 | 行为 |
|----|------|
| User | `❯ ` 前缀 + 正文（无右侧气泡） |
| Assistant | 裸 markdown（无 avatar / role 标签） |
| Reasoning | `Thinking…` / `Thought`；展开后 `│` quote bar |
| Tool | `{Verb} {bare target}`（`Read path` / `Ran cmd`）；展开看 detail；错误后缀 `failed`；diff 显示 `+n/-m` |
| Bridge 字段 | `title` = tool name；`text` = bare target；`detail` = args→output |
| Approvals | 可操作卡片 + modal（Allow / Deny / plan 三态） |

**Stick-to-bottom（`useStickToBottom`，对齐 TUI `auto_scroll`）：**

- 默认 following：内容增长（含 in-place stream 与 ResizeObserver）时 **即时** `scrollTop = scrollHeight`（不用 smooth 排队）。
- 用户上滚离开底部阈值（~80px）→ unfollow；回到底部或点 **Jump to latest** → re-follow。
- 未 following 时 **禁止** 程序化滚到底（避免与用户读历史冲突）。
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
| Agents | — | `loadClis` | `clisStatus` |
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
| Conversation 时间线只有用户消息 | 主时间线只读 `chat_messages`；tool/plan 在 thread `events` | 运行中 banner 引导打开 **Sessions** transcript |
| Session 一直 Running 无更新 | 无 live 事件时只能靠 poll | **manager + ingest 推送**；transcript 初始 **tail 窗口** hydrate |
| Grok 卡住 | `x.ai/exit_plan_mode` plan approval 待决策 | ingest `approval/request` → UI `needs_approval` + transcript approval 卡 + `minos_local_approval_decision` |
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
