# AGENTS.md

# Project Architecture Reference

Minos 是一个远程 AI 编码控制系统：Mac 运行 host 端，通过手机或浏览器远程驱动 codex/claude/gemini/opencode CLI agent。各子系统的详细架构文档如下：

| 文档 | 覆盖范围 |
|------|---------|
| [docs/architecture-overview.md](docs/architecture-overview.md) | 项目总览、顶层架构、仓库结构、crate 依赖图、技术栈 |
| [docs/architecture-backend.md](docs/architecture-backend.md) | 后端服务 (minos-backend)：HTTP API、WebSocket 网关、认证、配对、实时扇出、数据库层、Agent 会话、后台 Worker |
| [docs/architecture-daemon.md](docs/architecture-daemon.md) | Host 守护进程 (minos-daemon)：relay 连接、agent 管理、本地 SQLite 持久化、Host Link、JSON-RPC 服务器 |
| [docs/architecture-tui.md](docs/architecture-tui.md) | 终端 UI (minos-tui)：Ratatui 布局、事件系统、嵌入式/daemon 后端、群聊协调、MCP agent 间协作 |
| [docs/architecture-mobile.md](docs/architecture-mobile.md) | 移动端 (Flutter + minos-mobile)：四层架构、Riverpod 状态管理、FRB 桥接、认证/配对/会话流程、WebSocket 重连 |
| [docs/architecture-web.md](docs/architecture-web.md) | Web 管理控制台 (React + Vite)：Zustand 状态、WebSocket 实时、工作区页面 |
| [docs/architecture-desktop.md](docs/architecture-desktop.md) | Host 桌面壳 (Tauri + React)：主 Host GUI，替代已移除的 Swift macOS app / 逐步替代 TUI |
| [docs/architecture-grok-acp-projection.md](docs/architecture-grok-acp-projection.md) | Grok ACP tool content/raw_output 双通道 → Minos UI 投影清单 |
| [docs/architecture-shared-crates.md](docs/architecture-shared-crates.md) | 共享 crate：domain、protocol、transport、cli-detect、agent-runtime、chat-store、acp-protocol、codex-protocol、ui-protocol、ffi-frb |
| [docs/architecture-messaging.md](docs/architecture-messaging.md) | Server + 全端消息体系：Conversation-first IM、@/Approval Attention、撤回/reaction、读写 fanout、Durable/Stream/Ingest/Command、Outbox 与水位 |
| [docs/superpowers/specs/2026-08-02-hub-collaboration-message-ssot.md](docs/superpowers/specs/2026-08-02-hub-collaboration-message-ssot.md) | 协作气泡 Hub SSOT 收敛：退役 Desktop dual-write、Agent 最终气泡单写者、Sync/Outbox 阶段 0–5 |
| [docs/superpowers/specs/2026-08-03-im-reliability-program/README.md](docs/superpowers/specs/2026-08-03-im-reliability-program/README.md) | **IM 可靠性总计划**：客户端 Sync + 后端投递/编排终态；任务图 TASKS.md |
| [docs/superpowers/specs/2026-08-03-im-reliability-program/next-track-b6-c5-c6.md](docs/superpowers/specs/2026-08-03-im-reliability-program/next-track-b6-c5-c6.md) | 下一轨 B6/C5/C6 终态：reaction 幂等、Intent Outbox、visibility、状态债 |
| [docs/superpowers/specs/2026-08-03-realtime-surface-model.md](docs/superpowers/specs/2026-08-03-realtime-surface-model.md) | 全局实时面模型：通道分级、订阅拓扑、带宽优先级、新增功能 checklist |
| [docs/superpowers/specs/2026-08-03-im-reliability-program/closeout-and-backlog.md](docs/superpowers/specs/2026-08-03-im-reliability-program/closeout-and-backlog.md) | IM Reliability 收口（B7/G2–G4）+ 产品 residual + Realtime 后续分层 |
| [docs/superpowers/specs/2026-08-03-client-im-sync-engine.md](docs/superpowers/specs/2026-08-03-client-im-sync-engine.md) | 客户端 IM Sync Engine 终态（Desktop + Mobile Outbox / Timeline / Inbox） |
| [docs/superpowers/specs/2026-08-03-backend-im-delivery-orchestration.md](docs/superpowers/specs/2026-08-03-backend-im-delivery-orchestration.md) | 后端投递与 Agent 编排终态（Outbox 车道、Push、CompletionWatch、Session 生命周期） |
| [docs/architecture-business-flow.md](docs/architecture-business-flow.md) | 完整业务流程：注册 → 配对 → 实时连接 → Agent 会话 → 流式交互 → 审批 → 重连恢复 |
| [docs/ci-gates.md](docs/ci-gates.md) | CI / 本地质量门禁矩阵：backend、backend-pg、mobile、web、desktop 职责划分与 just/xtask 入口 |
| [docs/ops/vps-deploy.md](docs/ops/vps-deploy.md) | 生产 VPS 部署：Caddy + GHCR 镜像 + Postgres/Redis，runtime-only（不 clone 源码） |
| [docs/ops/r2-media.md](docs/ops/r2-media.md) | 聊天附件 / media blob：Cloudflare R2（或本地目录）配置与 `/v1/media/*` API |
| [docs/ops/vps-dev-binary.md](docs/ops/vps-dev-binary.md) | Dev/agent 二进制旁路：linux amd64 构建 + rsync + systemd，与 Docker 数据面共存 |
| [docs/superpowers/specs/2026-07-30-cloud-identity-clients-long-term.md](docs/superpowers/specs/2026-07-30-cloud-identity-clients-long-term.md) | 长期方案 L0：Supabase exchange、Host 链接、Desktop/Web UI SSOT、Mobile 云端角色 |
| [docs/superpowers/specs/2026-07-30-program/](docs/superpowers/specs/2026-07-30-program/README.md) | 执行体系 L1/L2：分域设计 + 任务依赖图 TASKS.md |

# Development-State Compatibility Policy

Minos 当前处于主动开发阶段，没有需要支持的历史发布版本。所有代码、schema、协议、数据库结构和文档都应以最新目标架构为准。

## High Priority Rule
- Do not add backward-compatibility layers, dual-read/dual-write paths, legacy migrations, feature flags for old protocol shapes, or adapter code for obsolete in-repo versions unless the user explicitly requests compatibility.
- Prefer clean breaking changes over compatibility scaffolding when changing internal APIs, wire schemas, storage schemas, or generated bindings.
- When a data model changes, update the canonical schema/migration/tests directly to the new shape instead of preserving old rows or old payload formats.
- Remove obsolete code paths, tests, fixtures, and documentation during the same change when they no longer match the latest architecture.
- Review feedback that asks for old-version migration or compatibility should be treated as out of scope by default; document that the project intentionally targets latest-only development state.
- Keep implementation plans focused on the final desired architecture, not incremental legacy support, unless a task explicitly names a released version or compatibility requirement.

## Final-Architecture Planning Rule（终态规划，禁止临时补丁路线）

所有非琐碎实现、重构、bugfix 与书面 plan **只按功能完成后的目标代码结构**设计与验收。不以「改造范围小」「先止血再还债」「短期/中期/临时方案」作为默认路径。

### 必须遵守
- **Plan 与 PR 描述目标态**：模块边界、数据模型、状态机、删除清单、验收不变量；不把「先加 if 绕过」写成正式方案。
- **一次改对所有权边界**：根因在共享契约 / 生命周期 / SSOT 时，扩大 diff 到正确边界，而不是在 UI 或调用点叠守卫。
- **删除与新增同 PR**：被替代的轮询、软去重、空壳 fallback、死代码 decision 分支、文档谎言在同一变更中移除。
- **多 Agent 可承担大改**：范围以终态正确性为准，不以单人单次 PR 大小裁剪架构。
- **禁止用临时层「过渡」**：例如双读路径、兼容 feature flag、时间驱动撞竞态、body 软去重代替统一 id、仅打日志的 stub job 冒充生命周期。

### 明确禁止写入 plan / 实现的话术与做法
- 「先短期修一下，以后再重构」
- 「加一个 0/400/1200ms 重试先让它绿」
- 「兼容旧 id / 旧字段双路径」
- 「stub 先 count 一下，未来再 sweep」
- 「决策枚举先留着，caller 以后再接」

### 与 Bugfix / Elegance 的关系
- 与 §5 Demand Elegance、§6 Autonomous Bug Fixing 一致：hack 只在边界**证明确属**该层不变量时允许；默认实现终态结构。
- 若必须分 Phase 落地，每个 Phase 交付的是**终态子系统的可合并切片**（可独立编译、测试、删除旧路径），不是「临时行为 + 遗留债」。

# Workflow Orchestration

## 1. Plan Mode Default
- Enter plan mode for ANY non-trivial task (3+ steps or architectural decisions)
- If something goes sideways, STOP and re-plan immediately — don't keep pushing
- Use plan mode for verification steps, not just building
- Write detailed specs upfront to reduce ambiguity
- Plans must obey **Final-Architecture Planning Rule**: target structure only; no short-term patch roadmaps as the design

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
- Skip this for simple, obvious fixes — don't over-engineer, but do not use "minimal fix" as permission to patch only the visible symptom
- Challenge your own work before presenting it

## 6. Autonomous Bug Fixing
- When given a bug report: just fix it. Don't ask for hand-holding.
- Diagnose the full affected code path before editing: reproduce or inspect the failing behavior, trace caller/callee boundaries, storage/schema/protocol contracts, runtime state, UI presentation, and existing tests/docs that define the intended behavior.
- Fix the root cause at the correct ownership boundary. Do not hide symptoms with local guards, UI-only rewrites, broad fallbacks, silent defaults, retries, compatibility shims, or defensive special cases unless that boundary is demonstrably where the invariant belongs.
- Prefer the smallest correct change, not the smallest diff. A larger change is required when the bug is caused by a broken shared invariant, wrong data model, incorrect lifecycle ordering, or missing contract propagation.
- Preserve and strengthen existing architecture: integrate with current abstractions and state flows, keep code clear and direct, remove obsolete wrong paths, and avoid new redundant layers.
- Add regression coverage that would have failed for the real bug. Tests should assert the corrected invariant or end-to-end behavior at the narrowest reliable boundary.
- Point at logs, errors, failing tests, schema constraints, or code-path evidence — then resolve them.
- Go fix failing CI tests without being told how.

## 7. Unit Test Discipline
- Unit tests must target isolated logic only: business rules, state changes, parsing, validation, serialization, and error handling
- Do not include UI flows, integration paths, real network/database/filesystem/device behavior, or end-to-end scenarios in unit tests
- Mock or fake external dependencies; keep tests fast, deterministic, and focused
- If UI/integration coverage is needed, label it separately and do not mix it into unit tests
- Run the relevant unit test command before closing and report the command/result

## 8. Observability and Comments
- Add concise comments for non-obvious control flow, protocol boundaries, data-shape conversions, and concurrency decisions; avoid comments that restate the code.
- Add structured logs at key lifecycle and failure points, especially RPC boundaries, background tasks, persistence writes, retries, and user-visible state transitions.
- Logs must include enough stable fields to locate the failing object or operation, such as project_id, session_id, workspace path, method name, and error; do not log secrets, tokens, or full sensitive payloads.
