# Minos 项目架构总览

> 本文档为 Minos 项目的全局架构总览，涵盖所有子系统及其交互关系。

## 项目定位

Minos 是一个远程 AI 编码控制系统：在 Mac 上运行 host 端，通过手机（iOS/Android）或浏览器远程驱动 `codex` / `claude` / `gemini` / `opencode` 等 CLI agent。

## 顶层架构

```
┌──────────────────────────────────────────────────────────────────────┐
│                           客户端层 (Clients)                           │
│  macOS (SwiftUI) · Mobile (Flutter) · Web · TUI · Desktop (Tauri)     │
│       UniFFI            FRB          WS/REST  local RPC → daemon      │
├──────────────────────────────────────────────────────────────────────┤
│                     后端服务 (minos-backend)                          │
│  HTTP /v1/*  ·  WebSocket Gateway  ·  Domain/UC  ·  Worker Plane      │
│              PostgreSQL/SQLite  ·  Redis (prod)                       │
├──────────────────────────────────────────────────────────────────────┤
│                     Host 端 (minos-daemon)                            │
│  Agent Runtime · Relay Client · Local RPC (TUI / Desktop)             │
└──────────────────────────────────────────────────────────────────────┘
```

## 仓库结构

```
Minos/
├── crates/                          # Rust workspace (12 个 crate)
│   ├── minos-domain/                # 核心域类型（ID、错误、枚举）
│   ├── minos-protocol/              # 线协议（JSON-RPC、Envelope、Realtime）
│   ├── minos-transport/             # 传输层（WS client、backoff）
│   ├── minos-pairing/               # 配对状态机
│   ├── minos-cli-detect/            # CLI agent 检测
│   ├── minos-agent-runtime/         # Agent 运行时（多进程管理）
│   ├── minos-chat-store/            # 聊天持久化（SQLite）
│   ├── minos-acp-protocol/          # ACP 协议类型（Gemini）
│   ├── minos-codex-protocol/        # Codex app-server 协议类型
│   ├── minos-ui-protocol/           # UI 事件协议（统一事件形状）
│   ├── minos-backend/               # 后端服务（HTTP + WS + Worker）
│   ├── minos-daemon/                # Host 守护进程
│   ├── minos-mobile/                # 移动端 Rust 核心
│   ├── minos-tui/                   # 终端 UI
│   ├── minos-ffi-uniffi/            # UniFFI 绑定（→ Swift）
│   └── minos-ffi-frb/               # FRB 绑定（→ Dart）
├── apps/
│   ├── macos/                       # macOS 状态栏应用（SwiftUI + UniFFI）
│   ├── mobile/                      # Flutter 移动应用（iOS/Android）
│   ├── web/                         # Web 管理控制台（React + Vite）
│   └── desktop/                     # Host 桌面壳（Tauri + React，替代 TUI 进行中）
├── xtask/                           # 构建/代码生成编排
├── docs/                            # 架构文档 + ADR + 运维手册
├── schemas/                         # JSON Schema（Codex 协议）
├── deploy/                          # 部署配置
└── scripts/                         # 辅助脚本
```

## Crate 依赖关系

```
                    minos-domain  (叶节点: ID、错误、Agent枚举、角色)
                   /     |     \     \      \        \
                  /      |      \     \      \        \
    minos-pairing   minos-cli-detect  minos-ui-protocol  minos-chat-store
         |                    |              |                   |
         |                    |              |                   |
         |              minos-agent-runtime <--------------------+
         |              /      |       \
    minos-ffi-uniffi <-+-- minos-codex-protocol
         |             \-- minos-acp-protocol
         |
    minos-protocol ----> minos-transport
         |                    |
         +--> minos-ffi-frb --> minos-mobile
         |
    minos-backend
    minos-daemon
```

关键约束:
- `minos-domain` 无 workspace 内部依赖（纯值类型叶节点）
- `minos-acp-protocol` 和 `minos-codex-protocol` 是独立协议镜像，无内部依赖
- `minos-agent-runtime` 不依赖 `minos-protocol` 或 `minos-ui-protocol`（薄管道设计）
- FFI crate（`minos-ffi-uniffi`、`minos-ffi-frb`）是聚合 shim，re-export 多个 crate 的类型

## 详细文档入口

| 子系统 | 文档 |
|--------|------|
| 后端服务 | [docs/architecture-backend.md](architecture-backend.md) |
| Host 守护进程 | [docs/architecture-daemon.md](architecture-daemon.md) |
| 终端 UI | [docs/architecture-tui.md](architecture-tui.md) |
| 移动端 | [docs/architecture-mobile.md](architecture-mobile.md) |
| macOS 应用 | [docs/architecture-macos.md](architecture-macos.md) |
| Web 应用 | [docs/architecture-web.md](architecture-web.md) |
| Desktop 应用 | [docs/architecture-desktop.md](architecture-desktop.md) |
| Desktop 自动更新 | [docs/desktop-auto-update.md](desktop-auto-update.md) |
| Grok ACP 投影 | [docs/architecture-grok-acp-projection.md](architecture-grok-acp-projection.md) |
| 共享 Crate | [docs/architecture-shared-crates.md](architecture-shared-crates.md) |
| 业务流程 | [docs/architecture-business-flow.md](architecture-business-flow.md) |
| CI / 本地门禁 | [docs/ci-gates.md](ci-gates.md) |

## 后端关键 HTTP 路径

后端（`minos-backend`）对外暴露的稳定入口（详见 [architecture-backend.md](architecture-backend.md)）：

| 路径 | 用途 |
|------|------|
| `/health/live` | 进程存活探针 |
| `/health/ready` | 依赖就绪探针 |
| `/v1/auth/register` | 用户注册 |
| `/v1/auth/login` | 用户登录 |
| `/v1/agent-sessions` | Agent 会话列表/创建 |

## 技术栈

| 层 | 技术 |
|----|------|
| 后端 | Rust + Tokio + Axum + Tower |
| 数据库 | PostgreSQL (生产) / SQLite (开发) |
| 缓存/消息 | Redis (生产) / In-memory (开发) |
| macOS | SwiftUI + XcodeGen + UniFFI |
| 移动端 | Flutter 3.44.0 + flutter_rust_bridge v2 |
| Web | React 19 + TypeScript 6 + Vite 8 + shadcn/ui |
| Desktop | Tauri 2 + React 19 + TypeScript + Vite + Tailwind |
| TUI | Rust + Ratatui 0.29 + Crossterm 0.28 |
| 认证 | JWT (HS256) + Argon2id + 刷新令牌轮转 |
| 协议 | JSON-RPC 2.0 over WebSocket |
| 构建 | Just + Cargo xtask + Cargokit |
