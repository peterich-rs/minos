# Web 应用 (apps/web) 架构文档

> 本文档详细描述 Web 管理控制台的架构。

## 概述

Minos Web 应用是一个基于 React + TypeScript 的浏览器管理控制台，用于管理已配对的 Mac host、控制 agent、管理社交关系和对话。中文界面。

**源码路径**: `apps/web/`

## 技术栈

| 技术 | 版本 | 用途 |
|------|------|------|
| React | 19 | UI 框架 |
| TypeScript | 6 | 类型系统 |
| Vite | 8 | 构建工具 |
| Zustand | 5 | 状态管理 |
| Desktop UI chrome | SSOT | Vite/tsconfig alias `@/shared` → `apps/desktop/src/shared`；shared peer 钉到 web `node_modules`（CI 只装 web） |
| Desktop tokens | SSOT | `@import` `design-system.css`；ink/surface/primary CSS 变量与 Desktop 相同 |
| Tailwind CSS | 3.4 | 主题 map 与 Desktop 对齐；content 含 `../desktop/src/shared/**` |
| Lucide React | - | 图标 |
| Supabase JS | 2 | 可选 IdP → Minos exchange |

登录后主界面为 `src/cloud/CloudShell`。**UI 必须与 Desktop 一致**：同一 `ShellFrame` + `AppRail` / `WorkChrome` / `ComposerChrome` / `MessageChrome` / `AttentionChrome` / `PageHeader`。数据可 mock，**不得**另写一套平行 className 壳。旧 `components/*-workspace` 暂不挂载。详见 program D03 与 [desktop-buzz-reference.md](./desktop-buzz-reference.md) §SSOT。

## 目录结构

```
src/
  main.tsx                        # ReactDOM 入口
  App.tsx                         # AuthScreen | CloudShell
  index.css                       # @import Desktop design-system + Tailwind
  cloud/
    CloudShell.tsx                # ShellFrame + nav outlet
    CloudSidebar.tsx              # AppRail + mock projects
    CloudWorkView.tsx             # WorkChrome + MessageChrome + ComposerChrome
    CloudAttentionView.tsx        # PageHeader + AttentionChrome
    CloudHostsView.tsx / CloudSettingsView.tsx
    mock-data.ts
  lib/
    minos.ts · store.ts · supabase.ts · relay-socket.ts · …
  components/
    auth-screen.tsx               # 登录（Supabase / legacy）
    ui/                           # 遗留 shadcn（auth 等）；产品壳走 Desktop shared
```

## 认证流程

1. `AuthScreen` 调用 `registerBrowserAccount()` 或 `loginBrowserAccount()`
2. 后端返回 `{account, access_token, refresh_token}`
3. Session 存储在 Zustand + localStorage (`minos.web.session`)
4. `runWithSessionRefresh()` 包装所有 API 调用，401 自动刷新 token 重试

## 实时连接 (WebSocket)

### RelaySocket

1. `RelayManager`（`App.tsx`）在 session 存在时创建 `RelaySocket`
2. 调用 `createWsTicket()` 获取 WS ticket
3. 连接 `${wsBase}/ws/client?ticket=...`
4. 接收两类实时事件:
   - **`ui_event_message`**: Agent 线程更新（文本 delta、工具调用等）
   - **`social_message`**: 社交消息（好友/群聊）
5. 自动重连（指数退避: 1s, 2s, 5s）
6. 可见性驱动: 隐藏 30s 后关闭，聚焦时重连

### RPC over WebSocket

- `RelaySocket.sendRpc()` 发送 `forward` envelope 到特定 host 设备
- Payload 是 JSON-RPC 2.0（`{jsonrpc, id, method, params}`）
- 方法: `minos_start_agent`, `minos_send_user_message`, `minos_interrupt_session` 等

## 状态管理 (Zustand)

单一全局 store + `persist` 中间件:
- Auth session
- Navigation route
- Hosts（已配对 Mac）
- Relay connection
- Threads（Agent 线程）
- Chat records（聊天记录）
- Social events（社交事件）
- Composer text（编辑器文本）

## 工作区页面

| 路由 | 组件 | 功能 |
|------|------|------|
| `chat` | `ChatWorkspace` | 线程列表 + 实时对话 + 消息编辑器 |
| `tasks` | `TasksWorkspace` | 看板（运行/完成/错误）+ Agent 配置 CRUD |
| `friends` | `FriendsWorkspace` | 好友/请求/对话/消息（三栏布局） |
| `devices` | `DevicesWorkspace` | 已配对 Host 卡片 + 技能管理 |
| `profile` | `ProfileWorkspace` | 显示名/Minos ID/改密/登出 |
| `settings` | `SettingsWorkspace` | 主题/连接状态/后端 URL |

### ChatWorkspace

- 线程列表侧栏（搜索）
- 对话面板（空状态/建议/实时消息）
- 消息编辑器 + 中断按钮
- 工具调用可视化（可展开详情）

### TasksWorkspace

- 看板布局（running/done/error 列）
- Agent 配置文件管理（名称、模型、运行时、推理强度、Host 绑定）
- 配置仅存储在 localStorage

### FriendsWorkspace

- 三栏: 个人资料+好友+搜索 / 请求+对话 / 活跃对话
- 完整消息功能: 回复、撤回（5 分钟内）、提及
- 群聊创建

### DevicesWorkspace

- 已配对 Mac host 卡片
- "配对新 Mac" 对话框（配对令牌输入）
- Host skills 面板（扫描目录、开关 skill）

## 后端 API (`lib/minos.ts`)

1195 行 API 客户端，覆盖:

| 类别 | 端点 |
|------|------|
| 认证 | supabase exchange, refresh, logout |
| 实时 | ws-ticket |
| 配对 | list-hosts, confirm |
| 资料 | self, minos-id, display-name, search |
| 好友 | query, requests, accept, reject |
| 对话 | query, direct, group, members, messages, recall |
| Agent | sessions/list, read-turns |

## UI 设计系统

- **shadcn/ui** (new-york variant) + Radix UI 原语
- **暗/亮模式**: CSS 变量驱动
- **字体**: Space Grotesk (sans), IBM Plex Mono (mono)
- **动画**: Framer Motion（页面切换、布局动画）
- **自定义 CSS**: `.glass`（毛玻璃）、`.gradient-surface`
- **中文界面**: 所有 UI 标签为简体中文

## 与系统的连接

Web 应用是 **浏览器管理控制台**:
- 与 macOS host 连接同一后端
- 通过配对令牌与 Mac host 配对
- 通过 relay WebSocket RPC 控制 Mac 上的 agent
- 通过 REST API 管理社交图谱
- Agent 配置文件仅存储在浏览器本地
- 后端 URL 通过 `VITE_MINOS_BACKEND_URL` 配置
