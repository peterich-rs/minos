# Minos 项目架构总览

> 本文档为 Minos 项目的全局架构总览，涵盖所有子系统及其交互关系。

## 项目定位

Minos 是一个远程 AI 编码控制系统：在 Mac 上运行 host 端，通过手机（iOS/Android）或浏览器远程驱动 `codex` / `claude` / `gemini` / `opencode` 等 CLI agent。

## 顶层架构

```
┌─────────────────────────────────────────────────────────────────┐
│                        客户端层 (Clients)                        │
│  ┌──────────┐  ┌──────────────┐  ┌────────────┐  ┌───────────┐ │
│  │ macOS App │  │ Flutter 移动端 │  │ Web Admin  │  │  TUI 终端  │ │
│  │ (SwiftUI) │  │   (Dart/FRB)  │  │ (React/TS) │  │ (Ratatui) │ │
│  └─────┬─────┘  └──────┬───────┘  └─────┬──────┘  └─────┬─────┘ │
│        │ UniFFI         │ FRB             │ WS/REST        │ RPC  │
├────────┼────────────────┼─────────────────┼────────────────┼──────┤
│        │    ┌───────────┴─────────────────┴────────────────┤     │
│        │    │           后端服务 (minos-backend)            │     │
│        │    │  ┌─────────────┐  ┌───────────────────────┐  │     │
│        └───>│  │ HTTP API    │  │ WebSocket Gateway      │  │<────┘
│             │  │  /v1/*      │  │  /ws/client, /ws/host │  │
│             │  └─────────────┘  └───────────────────────┘  │
│             │  ┌─────────────┐  ┌───────────────────────┐  │
│             │  │ Domain/UC   │  │ Worker Plane           │  │
│             │  │ Auth,Pairing│  │ Outbox,Timeout,GC,Push │  │
│             │  └─────────────┘  └───────────────────────┘  │
│             └──────────────────────────────────────────────┘     │
│                         │              │                         │
│              ┌──────────┴──────┐  ┌────┴─────┐                  │
│              │ PostgreSQL/SQLite│  │  Redis   │                  │
│              │   (持久层)       │  │ (缓存层) │                  │
│              └─────────────────┘  └──────────┘                  │
├─────────────────────────────────────────────────────────────────┤
│                     Host 端 (minos-daemon)                       │
│  ┌────────────────┐  ┌───────────────┐  ┌──────────────────┐   │
│  │ Agent Runtime  │  │  Relay Client │  │  Local RPC Server │   │
│  │ (Codex/Claude/ │  │  (WS → 后端)  │  │  (JSON-RPC → TUI)│   │
│  │  Gemini/OC)    │  │               │  │                  │   │
│  └────────────────┘  └───────────────┘  └──────────────────┘   │
└─────────────────────────────────────────────────────────────────┘
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
│   └── web/                         # Web 管理控制台（React + Vite）
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
| 共享 Crate | [docs/architecture-shared-crates.md](architecture-shared-crates.md) |
| 业务流程 | [docs/architecture-business-flow.md](architecture-business-flow.md) |

## 技术栈

| 层 | 技术 |
|----|------|
| 后端 | Rust + Tokio + Axum + Tower |
| 数据库 | PostgreSQL (生产) / SQLite (开发) |
| 缓存/消息 | Redis (生产) / In-memory (开发) |
| macOS | SwiftUI + XcodeGen + UniFFI |
| 移动端 | Flutter 3.41.6 + flutter_rust_bridge v2 |
| Web | React 19 + TypeScript 6 + Vite 8 + shadcn/ui |
| TUI | Rust + Ratatui 0.29 + Crossterm 0.28 |
| 认证 | JWT (HS256) + Argon2id + 刷新令牌轮转 |
| 协议 | JSON-RPC 2.0 over WebSocket |
| 构建 | Just + Cargo xtask + Cargokit |
