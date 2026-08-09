# Minos 项目架构总览

> 本文档为 Minos 项目的全局架构总览，涵盖所有子系统及其交互关系。

## 项目定位

Minos 是一个 **以 Conversation 协作为核心** 的远程 AI 编码协作产品（Slack / 企微式 IM + 对话内 bot）：在 Project 下的时间线中，人与人、人与 **Agent（bot 成员）** 协作；手机 / 浏览器 / Desktop 经云端 Hub 同步消息、@、未读与审批 Attention。Agent **不是**真人登录账号，而是 conversation 一等参与者；其执行身体在用户 Mac/Linux Host 上的 `codex` / `claude` / `gemini` / `opencode` 等 CLI。

**产品主轴是消息驱动的聊天协作**；HostCommand / `/ws/host` 是 bot runtime 传输，不是协作主协议。决策见 [ADR 0021](adr/0021-agent-as-conversation-bot-participant.md)；投递模型见 [agent-participant-delivery](superpowers/specs/2026-08-09-agent-participant-delivery.md)；消息体系 SSOT 见 [architecture-messaging.md](architecture-messaging.md)。

## 顶层架构

```
┌──────────────────────────────────────────────────────────────────────┐
│  人类客户端 (Account participants)                                     │
│  Mobile · Web · Desktop Account UI                                    │
│  REST + /ws/client  →  Hub IM（发消息 / 收推送 / @人·@bot）            │
├──────────────────────────────────────────────────────────────────────┤
│  minos-backend  [VPS hub]                                             │
│  Conversation SSOT · participant delivery · Agent inbox · Outbox      │
│  /ws/client (人) · /ws/host (机器 runtime) · HTTP /v1/*               │
├──────────────────────────────────────────────────────────────────────┤
│  Host runtime (bot 身体)  minos-daemon  [user Mac/Linux]               │
│  CLI Agent Runtime · /ws/host ingest/commands · Local RPC (Desktop/TUI)│
└──────────────────────────────────────────────────────────────────────┘
```

Desktop 同机双角色：Account UI 走 `/ws/client` 聊天；内嵌 daemon 走 `/ws/host` 执行。产品 Online 以 **Account sync** 为主，Host/agent readiness 为次。

生产部署（runtime-only VPS，不在机器上 clone 源码）：[ops/vps-deploy.md](ops/vps-deploy.md)。

长期产品与身份方向（Supabase exchange、Host 同账号链接、Web 对齐 Desktop UI、Mobile 云端查看）：

- L0 纲领：[superpowers/specs/2026-07-30-cloud-identity-clients-long-term.md](superpowers/specs/2026-07-30-cloud-identity-clients-long-term.md)
- L1/L2 执行与依赖图：[superpowers/specs/2026-07-30-program/](superpowers/specs/2026-07-30-program/README.md)

## 仓库结构

```
Minos/
├── crates/                          # Rust workspace crates
│   ├── minos-domain/                # 核心域类型（ID、错误、枚举）
│   ├── minos-protocol/              # 线协议（JSON-RPC、Envelope、Realtime）
│   ├── minos-transport/             # 传输层（WS client、backoff）
│   ├── minos-cli-detect/            # CLI agent 检测
│   ├── minos-prompt-runtime/        # Session 提示词编译（bundle + digest）
│   ├── minos-agent-runtime/         # Agent 运行时（多进程管理）
│   ├── minos-chat-store/            # 聊天持久化（SQLite）
│   ├── minos-acp-protocol/          # ACP 协议类型（Gemini）
│   ├── minos-codex-protocol/        # Codex app-server 协议类型
│   ├── minos-ui-protocol/           # UI 事件协议（统一事件形状）
│   ├── minos-backend/               # 后端服务（HTTP + WS + Worker；含 host_link）
│   ├── minos-daemon/                # Host 守护进程（Host Link RPC）
│   ├── minos-mobile/                # 移动端 Rust 核心
│   ├── minos-tui/                   # 终端 UI
│   └── minos-ffi-frb/               # FRB 绑定（→ Dart）
├── apps/
│   ├── mobile/                      # Flutter 移动应用（iOS/Android）
│   ├── web/                         # Web 管理控制台（React + Vite）
│   └── desktop/                     # Host 桌面壳（Tauri + React；主 Host GUI）
├── xtask/                           # 构建/代码生成编排
├── docs/                            # 架构文档 + ADR + 运维手册
├── schemas/                         # JSON Schema（Codex 协议）
├── deploy/                          # 部署配置
│   ├── docker-compose.yml           # 本地 dev（勿用于公网）
│   └── prod/                        # VPS 生产清单（compose / Caddy / backup）
└── scripts/                         # 辅助脚本
```

## Crate 依赖关系

```
                    minos-domain  (叶节点: ID、错误、Agent枚举、角色)
                   /     |     \     \      \        \
                  /      |      \     \      \        \
    minos-cli-detect  minos-ui-protocol  minos-chat-store
            |              |                   |
            |              |                   |
      minos-agent-runtime <--------------------+
      /      |       \         \
minos-codex-protocol --+        \
minos-acp-protocol  --+--> minos-agent-runtime
minos-prompt-runtime -+         |
                                |
minos-protocol ----> minos-transport
      |
      +--> minos-ffi-frb --> minos-mobile
      |
    minos-backend
    minos-daemon
```

关键约束:
- `minos-domain` 无 workspace 内部依赖（纯值类型叶节点）
- `minos-acp-protocol` 和 `minos-codex-protocol` 是独立协议镜像，无内部依赖
- `minos-prompt-runtime` 无 workspace 内部依赖（纯 compiler；sha2/serde）
- `minos-agent-runtime` 依赖 `minos-prompt-runtime` 做 system prompt 编译；不依赖 `minos-protocol` 做协议扇出（薄管道设计；UI 投影类型可经 ui-protocol）
- `minos-ffi-frb` 是移动端 FRB 聚合 shim；Host GUI 通过 Desktop/TUI 直连 `minos-daemon`（无 Swift UniFFI）

## 详细文档入口

| 子系统 | 文档 |
|--------|------|
| 后端服务 | [docs/architecture-backend.md](architecture-backend.md) |
| Host 守护进程 | [docs/architecture-daemon.md](architecture-daemon.md) |
| 终端 UI | [docs/architecture-tui.md](architecture-tui.md) |
| 移动端 | [docs/architecture-mobile.md](architecture-mobile.md) |
| Web 应用 | [docs/architecture-web.md](architecture-web.md) |
| Desktop 应用 | [docs/architecture-desktop.md](architecture-desktop.md) |
| Desktop 自动更新 | [docs/desktop-auto-update.md](desktop-auto-update.md) |
| Grok ACP 投影 | [docs/architecture-grok-acp-projection.md](architecture-grok-acp-projection.md) |
| 共享 Crate | [docs/architecture-shared-crates.md](architecture-shared-crates.md) |
| 消息架构体系（Server + 全端） | [docs/architecture-messaging.md](architecture-messaging.md) |
| 业务流程 | [docs/architecture-business-flow.md](architecture-business-flow.md) |
| CI / 本地门禁 | [docs/ci-gates.md](ci-gates.md) |

## 后端关键 HTTP 路径

后端（`minos-backend`）对外暴露的稳定入口（详见 [architecture-backend.md](architecture-backend.md)）：

| 路径 | 用途 |
|------|------|
| `/health/live` | 进程存活探针 |
| `/health/ready` | 依赖就绪探针 |
| `/v1/auth/supabase` | Supabase JWT → Minos session（唯一人类账户入口） |
| `/v1/agent-sessions` | Agent 会话列表/创建 |

## 技术栈

| 层 | 技术 |
|----|------|
| 后端 | Rust + Tokio + Axum + Tower |
| 数据库 | PostgreSQL (生产) / SQLite (开发) |
| 缓存/消息 | Redis (生产) / In-memory (开发) |
| Desktop Host | Tauri + React + minos-daemon local RPC |
| 移动端 | Flutter 3.44.0 + flutter_rust_bridge v2 |
| Web | React 19 + TypeScript 6 + Vite 8 + shadcn/ui |
| Desktop | Tauri 2 + React 19 + TypeScript + Vite + Tailwind |
| TUI | Rust + Ratatui 0.29 + Crossterm 0.28 |
| 认证 | JWT (HS256) + Argon2id + 刷新令牌轮转 |
| 协议 | JSON-RPC 2.0 over WebSocket |
| 构建 | Just + Cargo xtask + Cargokit |
