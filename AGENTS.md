# AGENTS.md

# Project Architecture Reference

Minos 是一个远程 AI 编码控制系统：Mac 运行 host 端，通过手机或浏览器远程驱动 codex/claude/gemini/opencode CLI agent。各子系统的详细架构文档如下：

| 文档 | 覆盖范围 |
|------|---------|
| [docs/architecture-overview.md](docs/architecture-overview.md) | 项目总览、顶层架构、仓库结构、crate 依赖图、技术栈 |
| [docs/architecture-backend.md](docs/architecture-backend.md) | 后端服务 (minos-backend)：HTTP API、WebSocket 网关、认证、配对、实时扇出、数据库层、Agent 会话、后台 Worker |
| [docs/architecture-daemon.md](docs/architecture-daemon.md) | Host 守护进程 (minos-daemon)：relay 连接、agent 管理、本地 SQLite 持久化、配对 QR、JSON-RPC 服务器、UniFFI 暴露 |
| [docs/architecture-tui.md](docs/architecture-tui.md) | 终端 UI (minos-tui)：Ratatui 布局、事件系统、嵌入式/daemon 后端、群聊协调、MCP agent 间协作 |
| [docs/architecture-mobile.md](docs/architecture-mobile.md) | 移动端 (Flutter + minos-mobile)：四层架构、Riverpod 状态管理、FRB 桥接、认证/配对/会话流程、WebSocket 重连 |
| [docs/architecture-macos.md](docs/architecture-macos.md) | macOS 应用 (SwiftUI + UniFFI)：状态栏应用、DaemonDriving 协议、bootstrap、QR 渲染 |
| [docs/architecture-web.md](docs/architecture-web.md) | Web 管理控制台 (React + Vite)：Zustand 状态、WebSocket 实时、工作区页面 |
| [docs/architecture-shared-crates.md](docs/architecture-shared-crates.md) | 12 个共享 crate：domain、protocol、transport、pairing、cli-detect、agent-runtime、chat-store、acp-protocol、codex-protocol、ui-protocol、ffi-uniffi、ffi-frb |
| [docs/architecture-business-flow.md](docs/architecture-business-flow.md) | 完整业务流程：注册 → 配对 → 实时连接 → Agent 会话 → 流式交互 → 审批 → 重连恢复 |

# Development-State Compatibility Policy

Minos 当前处于主动开发阶段，没有需要支持的历史发布版本。所有代码、schema、协议、数据库结构和文档都应以最新目标架构为准。

## High Priority Rule
- Do not add backward-compatibility layers, dual-read/dual-write paths, legacy migrations, feature flags for old protocol shapes, or adapter code for obsolete in-repo versions unless the user explicitly requests compatibility.
- Prefer clean breaking changes over compatibility scaffolding when changing internal APIs, wire schemas, storage schemas, or generated bindings.
- When a data model changes, update the canonical schema/migration/tests directly to the new shape instead of preserving old rows or old payload formats.
- Remove obsolete code paths, tests, fixtures, and documentation during the same change when they no longer match the latest architecture.
- Review feedback that asks for old-version migration or compatibility should be treated as out of scope by default; document that the project intentionally targets latest-only development state.
- Keep implementation plans focused on the final desired architecture, not incremental legacy support, unless a task explicitly names a released version or compatibility requirement.

# Workflow Orchestration

## 1. Plan Mode Default
- Enter plan mode for ANY non-trivial task (3+ steps or architectural decisions)
- If something goes sideways, STOP and re-plan immediately — don't keep pushing
- Use plan mode for verification steps, not just building
- Write detailed specs upfront to reduce ambiguity

## 2. Subagent Strategy
- Use subagents liberally to keep main context window clean
- Offload research, exploration, and parallel analysis to subagents
- For complex problems, throw more compute at it via subagents
- One whole phase task per subagent for focused execution

## 3. Autonomous Documentation Maintenance
- Maintain documentation proactively after ANY implementation change — do not wait to be asked
- Before closing a task, sync all affected docs to the current code
- Treat the codebase as the single source of truth and align docs to it
- Remove obsolete, irrelevant, and duplicate content to keep docs clean
- Rewrite unclear sections when needed; do not just append patches onto stale documentation
- Ensure examples, commands, file paths, configs, and behavior descriptions reflect reality
- If documentation is already correct, explicitly confirm that it was checked

## 4. Verification Before Done
- Never mark a task complete without proving it works
- Diff behavior between main and your changes when relevant
- Ask yourself: "Would a staff engineer approve this?"
- Run tests, check logs, demonstrate correctness

## 5. Demand Elegance (Balanced)
- For non-trivial changes: pause and ask "is there a more elegant way?"
- If a fix feels hacky: know everything you know, implement the elegant solution
- Skip this for simple, obvious fixes — don't over-engineer
- Challenge your own work before presenting it

## 6. Autonomous Bug Fixing
- When given a bug report: just fix it. Don't ask for hand-holding
- Point at logs, errors, failing tests — then resolve them
- Zero context switching required from the user
- Go fix failing CI tests without being told how

## 7. Unit Test Discipline
- Unit tests must target isolated logic only: business rules, state changes, parsing, validation, serialization, and error handling
- Do not include UI flows, integration paths, real network/database/filesystem/device behavior, or end-to-end scenarios in unit tests
- Mock or fake external dependencies; keep tests fast, deterministic, and focused
- If UI/integration coverage is needed, label it separately and do not mix it into unit tests
- Run the relevant unit test command before closing and report the command/result

## 8. Observability and Comments
- Add concise comments for non-obvious control flow, protocol boundaries, data-shape conversions, and concurrency decisions; avoid comments that restate the code.
- Add structured logs at key lifecycle and failure points, especially RPC boundaries, background tasks, persistence writes, retries, and user-visible state transitions.
- Logs must include enough stable fields to locate the failing object or operation, such as project_id, thread_id, workspace path, method name, and error; do not log secrets, tokens, or full sensitive payloads.
