# AGENTS.md

# Core Engineering Principles

These principles are the highest-priority default for all project work. When they appear to conflict, prefer the earlier principle and the current target architecture.

- Start from first principles: derive conclusions from the actual code, product constraints, and basic facts. Stay skeptical, inspect the system, and research viable designs before applying industry conventions.
- Complete authorized work end to end: execute every planned and feasible step before handoff. A progress update never replaces continued execution; pause only for a genuine blocker that needs user action or a material discovery that invalidates the plan, then report it and replan. Otherwise continue through implementation, verification, and documentation, and hand off only when the task is complete.
- Delete obsolete paths. Ship only current code.
- Use the simplest code that meets current needs.
- Build in layers: ship the smallest working slice of the target architecture first, then extend a working product.
- Keep modules separate and responsibilities clear.
- Prefer mature libraries that simplify or stabilize the system.
- Check existing dependencies, documentation, and types before adding or changing code.
- Design for the long term.
- Study proven products and adopt their patterns where they fit Minos.

# Project Architecture Reference

Minos 是一个远程 AI 编码控制系统：Mac 运行 host 端，通过手机或浏览器远程驱动 codex/claude/gemini/opencode CLI agent。各子系统的详细架构文档如下：

| 文档 | 覆盖范围 |
|------|---------|
| [docs/architecture-overview.md](docs/architecture-overview.md) | 项目总览、顶层架构、仓库结构、crate 依赖图、技术栈 |
| [docs/architecture-backend.md](docs/architecture-backend.md) | 后端服务 (minos-backend)：HTTP API、WebSocket 网关、认证、配对、实时扇出、数据库层、Agent 会话、后台 Worker |
| [docs/architecture-daemon.md](docs/architecture-daemon.md) | Host 守护进程 (minos-daemon)：relay 连接、agent 管理、本地 SQLite 持久化、Host Link、JSON-RPC 服务器 |
| [docs/architecture-mobile.md](docs/architecture-mobile.md) | 移动端 (Flutter + minos-mobile)：四层架构、Riverpod 状态管理、FRB 桥接、认证/配对/会话流程、WebSocket 重连 |
| [docs/architecture-web.md](docs/architecture-web.md) | Web 管理控制台 (React + Vite)：Zustand 状态、WebSocket 实时、工作区页面 |
| [docs/architecture-desktop.md](docs/architecture-desktop.md) | Host 桌面壳 (Tauri + React)：主 Host GUI（已移除 Swift macOS app 与 minos-tui） |
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

# Current-State and Target-Architecture Policy

Minos is under active development and has no historical release that requires support. Code, schemas, protocols, data, generated bindings, tests, and documentation target the latest architecture only.

- Do not add compatibility layers, dual reads or writes, legacy migrations, old-shape feature flags, or adapters unless the user explicitly requires compatibility.
- Prefer a clean breaking change. Change the canonical schema, contract, and tests together; do not preserve obsolete rows, payloads, fixtures, or branches.
- Every plan and implementation describes the completed target architecture: module boundaries, data model, lifecycle or state machine, removal list, and acceptance invariants.
- A phased delivery is valid only when each phase is a releasable slice of that target architecture. It must compile, be verifiable, and leave no temporary behavior or migration debt behind.
- Fix shared-contract, lifecycle, and SSOT defects at their ownership boundary. Do not hide them with UI guards, retries, silent defaults, soft deduplication, placeholders, or other local workarounds.

# Agent Workflow

## Before changing code

- Read the relevant types, dependencies, architecture documentation, existing implementations, and nearest tests before designing a change.
- For non-trivial work (three or more steps, cross-module changes, or architectural decisions), create a plan that includes its verification and deletion work. If evidence invalidates the plan, stop and revise it.
- Use established product patterns when they suit Minos, but adopt them through the existing architecture rather than copying unnecessary complexity.
- Choose mature dependencies only when they materially simplify or stabilize the system; check existing dependencies first.

## Implementing changes

- Keep responsibilities and ownership boundaries explicit. Prefer direct code and the smallest correct change over a small diff that leaves a broken invariant.
- Remove code, tests, fixtures, documentation, polling, fallbacks, or decision branches made obsolete by the change in the same change set.
- For a bug report, trace the full affected path: caller and callee, contracts, persistence, runtime lifecycle, UI projection, and existing evidence. Fix the root cause autonomously and add a focused regression test for the corrected invariant.
- Use parallel agents for independent research or isolated phases when that reduces total work without splitting ownership. Give each agent a bounded outcome and integrate its result against the same target architecture.
- Reconsider non-trivial designs before implementation. If a solution feels like a workaround, identify the correct layer and implement the clean design instead.

## Documentation and observability

- Treat code as the source of truth. After a material implementation change, update every affected document, example, command, path, configuration, and behavior description; remove stale or duplicate documentation.
- State when the relevant documentation was checked and already accurate.
- Comment only non-obvious control flow, protocol boundaries, data-shape conversion, and concurrency decisions.
- Add structured logs at lifecycle and failure boundaries. Include stable identifiers (for example `project_id`, `session_id`, workspace path, method, and error) without logging secrets or sensitive payloads.

## Verification and completion

- Do not declare work complete without evidence. Compare behavior with the base branch when relevant, run the applicable quality gates, inspect meaningful logs or errors, and report the results.
- Review completed work adversarially: trace the broader call chain, state, lifecycle, and failure modes for unexpected behavior. Find the root cause rather than only the reported symptom, then proactively inspect and correct analogous paths to preserve whole-codebase correctness.
- Unit tests cover isolated business rules, state changes, parsing, validation, serialization, and error handling. Mock external systems; do not label UI, network, database, filesystem, device, or end-to-end flows as unit tests.
- Keep integration and UI coverage separate, and run the relevant test command before handoff.
- Treat CI failures in the changed area as work to resolve, not as optional follow-up.
