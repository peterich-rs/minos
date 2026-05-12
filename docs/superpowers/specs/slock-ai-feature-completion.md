# Minos 对标 slock.ai 的完整功能规划

> 本文件基于当前代码仓库（`crates/`、`apps/macos`、`apps/mobile`、`apps/web`、`crates/minos-backend/migrations/`）的实际实现现状盘点，产出对标 [slock.ai](https://slock.ai) 的完整功能实现规划。**以代码为 source of truth**；`docs/superpowers/plans/` 与 `docs/superpowers/specs/` 中的历史文档可能与此目标有出入，仅作参考。

## 目录

1. Introduction
2. 目标、非目标与对标边界
3. Glossary
4. 角色与用户故事
5. 能力域盘点：端矩阵 + 现状 → 目标差值（Gap）
   - 5.1 账户与会话
   - 5.2 设备配对与 Host 管理
   - 5.3 Agent 运行时（codex / claude / gemini）
   - 5.4 Thread / UI Event 流式体验
   - 5.5 Host Skills 与 Workspace 管理
   - 5.6 Social / IM 协作
   - 5.7 Observability、离线与重连
   - 5.8 数据持久化与跨平台路径
6. 需求（EARS）
   - R1 账户与会话
   - R2 设备配对与 Host 管理
   - R3 Agent 运行时与 CLI 驱动
   - R4 Thread / UI 流式体验
   - R5 Host Skills 管理
   - R6 Social / IM
   - R7 Observability
   - R8 跨平台运行时与数据持久化
   - R9 Parser / Serializer（round-trip 要求）
   - R10 Web Admin 专项
7. 非功能性需求（NFR）
8. 非目标（Non-Goals）与延后项
9. 风险与开放问题
10. 附录 A：与历史文档的差异说明

---

## 1. Introduction

Minos 的目标是成为 slock.ai 风格的「多端 AI Coding 远程控制产品」：把本地 Mac 上的 `codex` / `claude` / `gemini` 等编码 CLI 托管成受保护的 agent host，让手机、浏览器等远程终端通过后端 relay 完成「发需求 → 看流式输出 → 审批 → 再续聊」的完整交互，同时配合轻量 IM/社交协作。

当前仓库的实现边界：

- **后端 (`crates/minos-backend`)**：axum + sqlx/SQLite 的 REST + WebSocket relay，已跑通 `/v1/auth`、`/v1/pairing`、`/v1/me/{hosts,peers,peer,profile}`、`/v1/threads`、`/v1/users`、`/v1/friends`、`/v1/friend-requests`、`/v1/conversations` 与 `/devices` WS（`Forward` / `Forwarded` / `Event` / `Ingest`）。17 条 migration 已落地 social IM 全链路表。
- **Host 端 (`apps/macos` + `crates/minos-daemon` + `crates/minos-agent-runtime`)**：macOS MenuBarExtra + UniFFI + XcodeGen；多 workspace codex app-server 管理、thread 持久化、reconciliation、cli-detect、host skills、host 端 `/v1/me/peers` 显示相对完整。
- **Mobile 端 (`apps/mobile` + `crates/minos-mobile` + `crates/minos-ffi-frb`)**：Flutter + flutter_rust_bridge v2，Riverpod + shadcn_ui。覆盖注册登录、QR 扫码配对、thread 列表/详情、流式 UI 事件、`start_agent`/`send_user_message`、好友/群聊/对话消息、日志和 request trace 面板。
- **Web 端 (`apps/web`)**：React 19 + Vite + TS，纯 HTTP + `ws-ticket` 模式；单文件 `App.tsx` 内嵌 console/social/agents 三个 workspace，`localStorage` 存 session；可登录/注册、列出 hosts、`pair`、列出 threads、读取 thread、`start_agent`/`send_user_message`/`close_thread`、IM 对话。

对标 slock.ai 仍有明显差距：host 端缺少「应用内登录账号」、多 host/多 mobile 的会话切换能力在三端分化、web admin 离「浏览器端完整控制台」仍差较多（无 pairing 二维码展示、无 host skills 管理、无 runtime agent 列表缓存、无 request trace）、mobile 端也缺少完整的 host 切换与 host skills 编辑入口。此外 E2EE、Android、公网生产部署与 Sparkle 自动更新明确**不在本次目标内**。

## 2. 目标、非目标与对标边界

### 2.1 对标能力矩阵（slock.ai → Minos）

| slock.ai 能力 | Minos 现状 | 目标 |
| --- | --- | --- |
| 账户登录（邮箱/密码）、会话刷新 | backend + mobile 已实现；web 通过 localStorage；host 端无账户 | 三端账户 + 登出 + refresh 统一；host 端不强制账户，但对接 host 上的 account_id 显示 |
| 远程控制一台/多台 Mac | backend multi-host 支持、mobile 已支持 host 切换；web 以 `activeHost` 变量近似支持；host 端显示多 mobile/account | 三端一致的「当前 host + 切换 host」心智 |
| 远程驱动 codex / claude / gemini | codex app-server 已接入；claude/gemini 仅在 `AgentName` 枚举与 cli-detect 中存在 | 至少把 claude/gemini 打到「可启动 agent、至少 raw ingest 不崩」的程度 |
| Thread 列表、流式消息、reasoning、tool call、工具审批 | UI 事件协议完整，流式 `text_delta`/`tool_call_placed`/`tool_call_completed`/`reasoning_delta` 已有；**工具审批** 仅 `minos-agent-runtime::approvals` 预留 | mobile + web 补齐审批交互 |
| Mid-turn 中断、thread 关闭、重新启动 | `minos_interrupt_thread` / `minos_close_thread` RPC 已暴露；mobile/web 均有入口 | 端到端可观测的 interrupt；reconnect 时 thread 状态正确恢复 |
| 多 workspace / 项目级分组 | host daemon 支持；mobile 以 workspace 字符串输入 | mobile/web 显示 workspace 列表与最近使用 |
| Host skills（slash/system skills）管理 | daemon RPC `list_host_skills`/`write_host_skill_config` 已有 | mobile/web 提供启停 UI；host MenuBar 给出入口 |
| 好友/群聊 IM（SD-like） | backend 全链路 + mobile + web 已跑通 | 补 host 端通知气泡 + mobile unread badge 与后台通知占位 |
| 审计/事件流 | backend `raw_events` 表存 + `UiEventMessage` 翻译 | web 侧给 admin 视角的原始事件日志查看；mobile 已有 request-trace 面板 |
| 多端同账号 | backend 设计允许，mobile 单设备/账户；web 与 mobile 同账号并行 | 明确「同 account 在 mobile + web 上并存」的心智与冲突策略 |

### 2.2 本规划的范围（In-Scope）

- 补齐三端已有 backend 接口但 UI 缺失的能力（尤其 web）。
- 在不改底层协议的前提下，补 host 端与账户/Profile 的弱绑定信息展示。
- `claude` / `gemini` 至少覆盖「启动、ingest、关闭」三步（具体子能力在 R3 明确分层）。
- Web 端从「单页 MVP」进化为完整 admin console：pairing 二维码、host skills、agent runtime 选择与诊断、social、observability。
- 统一 Observability：`request_trace` + `log_capture` 心智在 mobile 已有，web 和 host 需各自提供类似面板。

### 2.3 明确非目标（Out-of-Scope）

- **E2EE（X25519/Ed25519/AES-GCM）**：沿用现有传输层安全，不提供端到端加密（详见 §8）。
- **Android 生产化**：当前 Flutter 保留 `android/`，本轮不做生产签名/发布。
- **公网生产部署 HA**：backend 在 Cloudflare Tunnel + 单实例 SQLite 即可，不做多实例 / 分片。
- **Sparkle 自动更新 / TestFlight 发布自动化**：本轮不做。
- **OAuth / SSO / 双因素 / 邮箱校验**：沿用 argon2id 密码账号。
- **完整的审计合规**：仅提供技术可观测能力，不承诺企业合规。

## 3. Glossary

- **Minos_Backend**：`crates/minos-backend` 暴露的 HTTP/WS 服务（`/health`、`/devices` WS、`/v1/*`），是三端唯一的后端入口。
- **Minos_Daemon**：`crates/minos-daemon`，以库形式通过 UniFFI 驱动 macOS App；也有独立 CLI（`cargo run -p minos-daemon -- ...`）。
- **Host 端 / Agent_Host**：运行 `Minos_Daemon` + `Minos_AgentRuntime` 的机器（目前 macOS，DeviceRole = `AgentHost`）。
- **Mobile_Client**：iOS/Android Flutter 客户端（DeviceRole = `MobileClient`），通过 `crates/minos-mobile` 与 backend 通信。
- **Browser_Admin**：`apps/web` 浏览器控制台（DeviceRole = `BrowserAdmin`），与 backend 通过 HTTP + `ws-ticket` 通信。
- **Pairing_Qr_V2**：backend 组装的 QR 负载（`{v:2, host_display_name, pairing_token, expires_at_ms}`），ADR 0014。
- **Device_Secret**：host 长期 bearer，pair 后由 backend 经 `Event::Paired` 下发（仅 Host 端持久化；ADR 0020 mobile 走 JWT）。
- **Ws_Ticket**：短命 JWT，供 browser WS upgrade 使用；通过 `POST /v1/auth/ws-ticket` 获取。
- **UI_Event_Message**：`crates/minos-ui-protocol` 定义的统一 UI 事件（`text_delta`/`tool_call_placed`/`tool_call_completed`/`reasoning_delta`/`message_started`/`message_completed`/`thread_opened`/`thread_closed`/`thread_title_updated`/`error`/`raw`）。
- **Raw_Ingest**：`crates/minos-agent-runtime::RawIngest`，codex/claude/gemini CLI 的原始 JSON-RPC 通知，host → backend `Envelope::Ingest` 转发并入库 `raw_events`。
- **Host_Skill**：`.codex/skills/*.toml` 风格的宿主技能描述；daemon RPC 提供 `list_host_skills`、`write_host_skill_config`。
- **Account_Host_Pairing**：`account_host_pairings` 表，取代旧版 `pairings`；一个 account 可绑定多个 host，一个 host 可绑定多个 account（多对多）。
- **MINOS_HOME**：`$MINOS_HOME` 或默认 `~/.minos`，host/daemon 持久化根目录。

## 4. 角色与用户故事

### 4.1 Developer（终端用户 / 远程操作方）

- 作为开发者，我希望通过手机或浏览器，向我自己 Mac 上的 codex / claude / gemini 下发任务，以便在不开 Mac 的情况下也能推进工作。
- 作为开发者，我希望看到 assistant 的流式输出、reasoning、tool 调用与结果，而不是等一整次回包。
- 作为开发者，我希望能中断当前 turn、关闭已结束的 thread，或基于已有 thread 续聊。
- 作为开发者，我希望在多台 Mac 间快速切换「目标 host」，且看到每台 host 上可用的 agent 与 host skills。

### 4.2 Host_Owner（Mac 主人 / 自托管方）

- 作为 host 主人，我希望在 macOS 状态栏随时看到 backend 链路、账号、已配对的 mobile/browser。
- 作为 host 主人，我希望能撤销某一台 mobile/browser 的授权（forget peer）。
- 作为 host 主人，我希望知道 `codex`/`claude`/`gemini` 是否安装、版本、是否被成功启动。

### 4.3 Mobile_User（手机使用者）

- 作为 mobile 使用者，我希望 first-run 能扫 QR 完成配对，并支持同账号多 host。
- 作为 mobile 使用者，我希望在一个 tab 里看 agent thread，另一个 tab 里跟好友/群聊沟通。
- 作为 mobile 使用者，我希望能看到 request trace / 日志，以便出问题时自助诊断。

### 4.4 Web_Admin（浏览器 admin）

- 作为 web admin，我希望在浏览器里完成登录 → 配对 → 查看 hosts → 选 host → 启动 agent → 观察流式 thread 的全链路，不再需要 mobile 辅助。
- 作为 web admin，我希望能看到 host skills 并启停；也希望能导出/查看最近的原始事件用于诊断。

## 5. 能力域盘点：端矩阵 + 现状 → 目标差值

> 每个能力域下面的表格口径统一：`状态` 取值 `✅`（已实现）、`🟡`（部分实现）、`❌`（缺失）。`目标` 描述的是本次 spec 落地后希望达到的最终状态，跟 §6 的需求一一对应。

### 5.1 账户与会话

| 能力 | Backend | Host (macOS + daemon) | Mobile (Flutter) | Web (React) | 目标 / Gap |
| --- | --- | --- | --- | --- | --- |
| 邮箱/密码注册、登录 | ✅ `/v1/auth/register`、`/v1/auth/login`（argon2id、per-IP/per-email rate limit） | ❌ host 不需要账号，`AppState` 没有账户态 | ✅ 登录页 + `AuthStateFrame` | ✅ `App.tsx` 自管登录表单 | host 端展示 *绑定* 的 account_email（通过 `/v1/me/peers`），web 端需改用更结构化的 session store |
| Token 刷新 (JWT) | ✅ `/v1/auth/refresh`（per-account rate limit） | ❌ N/A | ✅ `refresh_session` + watch 状态机 | 🟡 仅 401 时重试（`runWithSessionRefresh`） | web 增加主动 refresh + 过期前刷新；host 端仍走 device-secret |
| Logout + refresh 吊销 | ✅ `/v1/auth/logout` | ❌ N/A | ✅ | ✅ | 需要确保 logout 时关闭 WS 并清理所选 host |
| WS 接入凭证 | ✅ `ws-ticket`（account-client only） + 标头鉴权（host-only） | ✅ device-secret 标头鉴权 | ✅ 标头 | ✅ `ws_ticket` | 无 |
| 多账号切换 | 🟡 backend 可多 account 并存，但未区分 session 隔离 | ❌ | ❌ 每台设备仅一个 session | 🟡 localStorage 单账号 | 明确「一个设备同一时刻只能有一个 account」；切换时清理 WS |
| Minos ID（可搜索 handle） | ✅ `/v1/me/profile/minos-id` | ❌ 未展示 | ✅ social_hub 页面可设置 | 🟡 `social-workspace` 内可设置 | host 端菜单里展示当前 bound account 的 `minos_id`（只读） |

### 5.2 设备配对与 Host 管理

| 能力 | Backend | Host | Mobile | Web | 目标 / Gap |
| --- | --- | --- | --- | --- | --- |
| Pairing token 颁发（QR v2） | ✅ `/v1/pairing/tokens`（host-auth） | ✅ `pairing_qr()` → `RelayQrPayload` → MenuBar QR | N/A | ❌ 无法从 web 自行生成（无 host-auth 通道） | Web 保持「消费已有 QR」而非颁发 |
| Pairing consume | ✅ `/v1/pairing/consume`（bearer + device headers） | ✅ 事件接收 `Event::Paired` 并写入 keychain | ✅ `pair_with_qr_json` | ✅ 对 JSON 的 fallback 输入 | web 提供更友好的「粘贴 QR JSON / 扫图像 / 输入 token」表单 |
| 列出 hosts（per account） | ✅ `/v1/me/hosts` | ❌ N/A | ✅ `list_paired_hosts` + Runtime 选择 | ✅ 基于 `listHosts()` | web 目前单一 activeHost，**缺少显示每 host 的 agent 可用性**；mobile 进入某 host 的「详情页」不够完整 |
| 列出 peers（per host） | ✅ `/v1/me/peers` | ✅ MenuBar 显示每台 mobile/browser | ❌ 不关心（mobile 不展示其他 mobile） | ❌ | host 端继续持有，web/mobile 不做 |
| Forget peer | ✅ `DELETE /v1/pairings/:host_device_id`、`DELETE /v1/me/peers/:mobile_device_id` | ✅ `forget_peer_device` | ✅ `forget_host` | 🟡 无 UI 入口 | web 补「删除当前 host 的配对」 |
| 切换 active host | — | — | ✅ `set_active_host` | ✅ 单个 `activeHost` 状态 | 三端心智统一：「当前 RPC 发到哪台 host」一定与 UI 选择一致 |
| 多 host 并发（同时有多个 WS thread） | ✅ backend registry 支持 | ✅ daemon 一台 | ❌ mobile 单一 active host RPC | ❌ web 同 | 本轮不做「同时向 2 台 host 发 RPC」，但保证切换快且可靠 |

### 5.3 Agent 运行时（codex / claude / gemini）

| 能力 | Backend | Host | Mobile | Web | 目标 / Gap |
| --- | --- | --- | --- | --- | --- |
| `list_clis` | — | ✅ daemon RPC（`minos_cli_detect`） | ✅ `list_clis` | ✅ 通过 `sendRpc` | 无新增 |
| `start_agent(codex)` | 转发 | ✅ 多 workspace `AgentManager` + `AppServerInstance` | ✅ | ✅ | 无 |
| `start_agent(claude)` / `start_agent(gemini)` | 转发 | ❌ `AgentManager` 目前只接 `codex_client`，没有 claude/gemini 的 `PtyAgent` 实现 | 🟡 UI 允许选择，但发下去会失败 | 🟡 同上 | 本轮做「最小可用」：调起 CLI、把 raw stdout/stderr 透传成 `UiEventMessage::Raw`，直到专门的协议适配完成 |
| `send_user_message` | 转发 | ✅ codex 已实 | ✅ | ✅ | 同 agent，本轮 claude/gemini 只保证 user prompt 传得下去 |
| `interrupt_thread` | 转发 | ✅ codex `state_machine::PauseReason::UserInterrupt` | ✅ 已暴露 | 🟡 UI 缺按钮 | mobile 已有入口；web 需要加按钮 |
| `close_thread` | 转发 | ✅ | ✅ | ✅ | 无 |
| Thread 列表（远端） | ✅ `/v1/threads/query`（scope by account） | 对应 daemon RPC | ✅ | ✅ | 需要明确列表口径：mobile 列当前账号所有 host 的 thread，还是「当前 active host」的 thread |
| 翻译器（codex/claude/gemini） | ✅ backend ingest 入库时调用 `translate_*` | — | — | — | claude/gemini 先走 `translate_*` 的 raw fallback，不要求逐字段翻译 |
| Thread reconciliation | ✅ `Event::IngestCheckpoint` + JSONL fallback | ✅ `Reconciliator` | ❌ mobile 无特殊处理（依赖 backend 下发 `UiEventMessage`） | ❌ 同 | host 保持现状；mobile/web 依赖 backend 推送 |

### 5.4 Thread / UI Event 流式体验

| 能力 | Backend | Host | Mobile | Web | 目标 / Gap |
| --- | --- | --- | --- | --- | --- |
| Thread 列表（带未结束状态） | ✅ | 通过 RPC 读本地 | ✅ `ThreadListPage` | ✅ `thread list` | web 端未显示 `end_reason`，可优化 |
| Thread 详情 + 流式渲染 | ✅ 通过 WS `Event::UiEventMessage` + `/v1/threads/read` 冷启动 | — | ✅ `ThreadViewPage` 已聚合 `text_delta` / tool call / reasoning | ✅ `transcriptFromEvents` 聚合器 | web 重新打开选中 thread 时仍走 HTTP 冷启动 + live 合并，需要补「滚动到底/到消息开始」和「reasoning 折叠/展开」 |
| Mid-turn interrupt | ✅ RPC | ✅ | ✅ 有按钮 | 🟡 无按钮 | web 补 |
| Tool approval | 🟡 `approvals` 模块骨架 | 🟡 仅占位 | ❌ 无 UI | ❌ 无 UI | 本轮目标：预留 UI 通道但不阻塞现有流（deferred 可接受） |
| Thread 与 workspace 绑定展示 | ✅（每 thread 带 workspace） | ✅ | 🟡 仅文本输入 | 🟡 仅文本输入 | mobile/web 展示最近用过的 workspace 列表（`MRU`） |
| Agent profile（slock.ai 上的「角色」） | ❌ backend 无概念 | ❌ | 🟡 Dart 侧 `agent_profile` | ✅ `lib/agent-profiles.ts` | 将 profile 限定为**客户端本地偏好**（CLI 选项、workspace 默认值），不引入 backend 字段 |

### 5.5 Host Skills 与 Workspace 管理

| 能力 | Backend | Host | Mobile | Web | 目标 / Gap |
| --- | --- | --- | --- | --- | --- |
| list / toggle host skills | 转发 | ✅ daemon RPC（`list_host_skills`、`write_host_skill_config`） | ✅ frb API 已暴露 | ❌ UI 未接线（仅 types 存在） | web 补完整 skills 面板；mobile 补易找的入口 |
| 多 workspace | — | ✅ daemon `AgentManager` 按 workspace 切 instance | 🟡 仅「输入 workspace 字符串」 | 🟡 同 | 三端提供最近 workspace MRU 列表（本地缓存） |
| Workspace 诊断 | 🟡 | 🟡 CLI `doctor` | ❌ | ❌ | host MenuBar 保持 `doctor` 链接；mobile/web 不强求 |

### 5.6 Social / IM 协作

| 能力 | Backend | Host | Mobile | Web | 目标 / Gap |
| --- | --- | --- | --- | --- | --- |
| Profile（`minos_id`） | ✅ | ❌ 不展示 | ✅ | ✅ | host 只读展示当前 account 的 `minos_id` |
| 搜索用户 (`/v1/users/search`) | ✅ | ❌ | ✅ | ✅ | 无 |
| 好友请求 / 接受 / 拒绝 | ✅ | ❌ | ✅ | ✅ | 无 |
| 好友列表 | ✅ | ❌ | ✅ | ✅ | 无 |
| 直聊 / 群聊 | ✅ | ❌ | ✅ | ✅ | 无 |
| @mention / reply / recall | ✅ | ❌ | ✅ | ✅ | 无 |
| Unread 聚合 | ✅ | ❌ | ✅ shell tab badge | 🟡 仅 conversation 面板 | web 在 tab 切换时显示 unread 数 |
| `SocialMessage` WS 推送 | ✅ | ❌ | ✅ broadcast subscribe | ✅ | 无 |
| Push 通知（APNs/FCM） | ❌ | ❌ | ❌ | ❌ | **明确不做**（§8） |

### 5.7 Observability、离线与重连

| 能力 | Backend | Host | Mobile | Web | 目标 / Gap |
| --- | --- | --- | --- | --- | --- |
| mars-xlog 文件日志 | ❌（仅 tracing stdout） | ✅ `~/Library/Logs/Minos` | ✅ `Documents/Minos/Logs/` | ❌ 无 | backend 在本轮新增 file-based xlog 输出到 `$MINOS_HOME/logs` |
| 内存 ring-buffer 日志（UI 面板） | ❌ | ❌ | ✅ `log_capture` + panel | ❌ | web 补最近日志 ring-buffer + 面板 |
| Request trace（method/status/耗时） | ❌ 未实现跨请求关联 | ❌ | ✅ `request_trace` | ❌ | web 补；host 可在 CLI `doctor` 中汇总 |
| 健康检查 | ✅ `/health` | ✅ `doctor` CLI | ❌ | ❌ | host 保留 CLI 即可 |
| 断线重连 / 背压 | ✅ backend 支持重连 | ✅ daemon `RelayClient` 多轮 backoff | ✅ `ReconnectController` + 前后台钩子 | 🟡 仅显式 `socket.close()`，没有 auto-retry | web 补短暂 backoff 重连 |
| 生命周期（前后台） | — | — | ✅ `notify_foregrounded/backgrounded` | ❌ | web 补 `visibilitychange` → 重连 |

### 5.8 数据持久化与跨平台路径

| 能力 | Backend | Host | Mobile | Web | 目标 / Gap |
| --- | --- | --- | --- | --- | --- |
| `$MINOS_HOME` | ❌（由 shell 层设置） | ✅ `paths::minos_home` | ✅ `app_paths.dart` | — | host/mobile 一致，backend 以 CLI/env 参数为准 |
| SQLite WAL | ✅ | ✅ | ❌（无本地 SQLite） | ❌ | 本轮不引入 mobile/web 本地 SQLite |
| Keychain / secure store | — | ✅ `device_secret_store` | ✅ `secure_pairing_store` | ❌（仅 localStorage） | web 仍用 localStorage，但明确只存 token，不存 device-secret |
| `raw_events` / UI event 重放 | ✅ | — | — | — | 无 |
| Device role 约束 | ✅ `DeviceRole { AgentHost, MobileClient, BrowserAdmin }` | ✅ | ✅ | ✅ | 无 |

## 6. 需求（EARS）

> 每条 `SHALL` 语句均遵循 EARS + INCOSE 规则：主语为 §3 定义的 System 名称，避免代词、避免模糊副词。`*` 表示对三端之一有额外覆盖要求。

### R1 账户与会话

**User Story**：作为 developer / web admin / mobile 使用者，我希望注册、登录、续约、登出，并在任意时刻明确知道当前登录的账号。

**Acceptance Criteria**

1. WHEN Developer 在 Web_Admin 或 Mobile_Client 上提交注册请求，THE Minos_Backend SHALL 对相同邮箱返回 HTTP 409 `email_taken`，对新邮箱返回 HTTP 200 并颁发 access/refresh token 与 `account_id`。
2. WHEN Developer 在 Web_Admin 或 Mobile_Client 上提交登录请求并凭据正确，THE Minos_Backend SHALL 返回 access token（JWT）、refresh token、`account_id`、`email`，且 THE Minos_Backend SHALL 在 1 秒内完成响应（p95，IP ≥ 1 req/s 时按 rate-limit 预算）。
3. WHILE 已登录，THE Mobile_Client SHALL 在 access token 剩余有效期 ≤ 60 秒时主动调用 `/v1/auth/refresh`。
4. WHILE 已登录，THE Web_Admin SHALL 在 access token 剩余有效期 ≤ 60 秒时主动调用 `/v1/auth/refresh`；IF refresh 失败，THEN THE Web_Admin SHALL 清理会话并跳回登录页。
5. WHEN Developer 在 Mobile_Client 或 Web_Admin 上执行登出，THE Minos_Backend SHALL 吊销对应 refresh token，并且 THE 该 Web_Admin / Mobile_Client SHALL 在 logout 响应返回后 500ms 内断开 `/devices` WebSocket。
6. THE Minos_Backend SHALL 在 `/v1/auth/register`、`/v1/auth/login` 日志中脱敏 `password` 字段（以 `<redacted>` 替代）。
7. WHERE Browser_Admin 需要建立 WS，THE Minos_Backend SHALL 通过 `POST /v1/auth/ws-ticket` 颁发 TTL ≤ 60 秒的 ws_ticket，且不接受 Host 角色调用该接口。
8. THE Host（Minos_Daemon）SHALL 在 MenuBar 上展示当前 `/v1/me/peers` 返回的每个 peer 的 `account_email` 与 `mobile_device_name`；IF `account_email` 为空，THEN THE Host SHALL 显示 `mobileDeviceName` 作为 fallback。

### R2 设备配对与 Host 管理

**User Story**：作为 developer，我希望在 mobile/web 上完成与多台 Mac 的配对，并在任意时刻明确知道「当前控制的 host 是谁」。

**Acceptance Criteria**

1. WHEN Host_Owner 在 Minos_Daemon 上点击「显示配对二维码」，THE Minos_Daemon SHALL 向 `/v1/pairing/tokens` 请求新 token 并渲染 `PairingQrPayload { v:2, host_display_name, pairing_token, expires_at_ms }`。
2. WHEN Mobile_Client 扫描 QR v2 并调用 `/v1/pairing/consume`，THE Minos_Backend SHALL 在同一事务内写入 `account_host_pairings` 行，并且 THE Minos_Backend SHALL 向该 Host 的活跃 WS 发送 `Event::Paired { peer_device_id, peer_name, your_device_secret }` 一次；IF Host 离线或队列满，THEN THE Minos_Backend SHALL 回滚 pair 行并返回 HTTP 500 `internal`。
3. WHEN Browser_Admin 提交一个 QR v2 JSON 或纯 token，THE Web_Admin SHALL 调用 `/v1/pairing/consume`，成功后自动将新 host 作为 `activeHost`。
4. WHEN Mobile_Client 的 `list_paired_hosts` 返回 ≥ 1 条记录，THE Mobile_Client SHALL 在 Partners Tab 展示每台 host 的 `host_display_name`，且 THE Mobile_Client SHALL 支持点击切换 `activeHost`。
5. WHEN Web_Admin 的 `listHosts()` 返回 ≥ 1 条记录，THE Web_Admin SHALL 在 console 顶部展示 host 切换器；选择后 `activeHost` 同步持久化到 `localStorage[minos.web.active-host]`。
6. WHEN Developer 在 Mobile_Client 或 Web_Admin 上对某 host 触发 forget，THE 对应客户端 SHALL 调用 `DELETE /v1/pairings/:host_device_id`，并且 THE Minos_Backend SHALL 向该 host 发送 `Event::Unpaired`。
7. WHEN Host_Owner 在 Minos_Daemon 上选择某个 peer 的删除按钮，THE Minos_Daemon SHALL 调用 `DELETE /v1/me/peers/:mobile_device_id`。
8. IF Web_Admin 当前 `activeHost` 在 `listHosts()` 响应中消失（例如被其它设备解除配对），THEN THE Web_Admin SHALL 自动将 `activeHost` 切到列表首项；IF 列表为空，THEN THE Web_Admin SHALL 提示「未有可控制的 host」。

### R3 Agent 运行时与 CLI 驱动

**User Story**：作为 developer，我希望能在 mobile/web 里选择 agent (`codex`/`claude`/`gemini`) 并启动一个 thread，首个 user prompt 必须传达到 host CLI。

**Acceptance Criteria**

1. THE Minos_Backend SHALL 透明转发任意 `Envelope::Forward{payload}`（JSON-RPC 2.0）到 caller 的 `account_host_pairings` 允许的 `target_device_id`，不解析 payload 内容。
2. WHEN Mobile_Client 或 Web_Admin 对当前 `activeHost` 发送 `minos_start_agent { agent, workspace, mode? }`，THE Minos_Daemon SHALL 返回 `StartAgentResponse { session_id, cwd }`；IF workspace 非绝对路径或不可读，THEN THE Minos_Daemon SHALL 返回 JSON-RPC 错误并不创建 thread。
3. WHERE `agent == Codex`，THE Minos_Daemon SHALL 使用 `AgentManager::start_or_join` 路径启动 codex app-server，并在同一 workspace 上复用 `AppServerInstance`。
4. WHERE `agent == Claude` OR `agent == Gemini`，THE Minos_Daemon SHALL 启动对应 CLI 子进程并将 stdout 行通过 `translate_claude` / `translate_gemini` 翻译成 `UiEventMessage`；IF 对应 CLI 未安装或 cli-detect 状态 ≠ `ok`，THEN THE Minos_Daemon SHALL 返回 `CliMissing { agent }` 错误。
5. WHEN Developer 对已存在 `session_id` 发送 `minos_send_user_message`，THE Minos_Daemon SHALL 在 200ms 内把 prompt 写入对应 CLI 的 stdin（或 codex JSON-RPC），且不改变 `thread_id`。
6. WHEN Developer 触发 `minos_interrupt_thread`，THE Minos_Daemon SHALL 将 thread 状态迁移为 `Suspended { UserInterrupt }`；IF 该 thread 已结束，THEN THE Minos_Daemon SHALL 返回 `thread_already_closed`。
7. WHEN Developer 触发 `minos_close_thread`，THE Minos_Daemon SHALL 关闭该 thread 并发送 `UiEventMessage::ThreadClosed { reason: UserStopped }`。
8. THE Minos_Backend SHALL 对每个 `Envelope::Ingest` 持久化到 `raw_events`，并对 `agent ∈ {codex, claude, gemini}` 调用 `translate_*`；IF 翻译失败，THEN THE Minos_Backend SHALL 回退为 `UiEventMessage::Raw { raw_kind, payload_json }` 继续转发，不阻塞 ingest。

### R4 Thread / UI 流式体验

**User Story**：作为 mobile / web 使用者，我希望看到 assistant 回复的流式文本、reasoning 与 tool call 结果，并且能随时中断。

**Acceptance Criteria**

1. WHEN Mobile_Client 或 Web_Admin 首次打开某 thread，THE 客户端 SHALL 调用 `POST /v1/threads/read { thread_id, limit: 200 }` 获取冷启动事件；`ui_events` 的顺序与 `raw_events.seq` 严格一致。
2. WHILE WS 处于 `connected`，THE Mobile_Client SHALL 订阅 `Event::UiEventMessage`，并将 `ts_ms` 严格单调递增的事件按 `thread_id` 派发给对应视图。
3. WHEN `UiEventMessage::TextDelta { message_id, text }` 到达，THE 客户端 SHALL 将 `text` 追加到目标 message 的 assistant 气泡；顺序以 `seq` 为准而非到达顺序。
4. WHEN `UiEventMessage::ToolCallPlaced / ToolCallCompleted` 到达，THE Mobile_Client 和 Web_Admin SHALL 展示工具名、参数 JSON、结果摘要，并以视觉方式区分 `is_error = true` 的失败结果。
5. WHEN `UiEventMessage::ReasoningDelta` 到达，THE Mobile_Client 和 Web_Admin SHALL 以可折叠（默认折叠）形式渲染 reasoning 内容。
6. WHEN `UiEventMessage::ThreadClosed { reason }` 到达，THE 客户端 SHALL 停止在该 thread 上的 composer 输入并显示关闭原因。
7. THE Mobile_Client 和 Web_Admin SHALL 在 thread 详情中提供「中断当前 turn」按钮；禁用条件为 thread 已 `Closed` 或无 `MessageStarted`-且-未 `MessageCompleted` 的 assistant message。
8. THE Web_Admin SHALL 在 thread 列表中对每个 `ThreadSummary` 展示 `end_reason`（若非空）。
9. WHERE Developer 配置了本地 agent_profile（`apps/web` 的 `lib/agent-profiles.ts` / `apps/mobile` 的 `agent_profile_store`），THE 对应客户端 SHALL 在「发起新 thread」时使用 profile 中保存的 `runtime_agent` 与默认 `workspace` 作为预填，但不将 profile 上传到 Minos_Backend。

### R5 Host Skills 管理

**User Story**：作为 developer，我希望在远端客户端（mobile / web）查看 host 上已安装的 skills，并按 workspace 启停单个 skill。

**Acceptance Criteria**

1. WHEN Mobile_Client 或 Web_Admin 调用 `minos_list_host_skills { workspace }`，THE Minos_Daemon SHALL 返回每个 workspace 的 `HostSkillsEntry { cwd, errors, skills[] }`；IF `workspace` 为 `null`，THEN THE Minos_Daemon SHALL 返回当前所有已知 workspace。
2. WHEN Developer 在 Mobile_Client 或 Web_Admin 对某 `HostSkillSummary` 切换 `enabled`，THE 客户端 SHALL 调用 `minos_write_host_skill_config { workspace, path, enabled }`，并在响应返回后 300ms 内刷新 skills 列表。
3. THE Web_Admin SHALL 在侧栏或 dialog 中展示 skills，并能按 `scope`（global / workspace）分组。
4. IF Minos_Daemon 返回非空 `HostSkillError`，THEN THE 客户端 SHALL 显示 `path` 与 `message`，并允许 Developer 复制错误文本。

### R6 Social / IM

**User Story**：作为 developer，我希望与其它 Minos 用户通过 minos_id 好友关系进行直聊或群聊。

**Acceptance Criteria**

1. WHEN Developer 在 Mobile_Client 或 Web_Admin 搜索 `minos_id`，THE 客户端 SHALL 调用 `POST /v1/users/search` 并过滤掉自身。
2. WHEN Developer 发送好友请求、接受、拒绝、撤回消息、@mention，THE Minos_Backend SHALL 通过 `Event::SocialMessage { conversation_id, message }` 广播给所有在线的会员 mobile/web。
3. WHILE 某 conversation 有未读消息，THE Mobile_Client SHALL 在 Shell Tab 的 badge 上显示聚合未读数。
4. WHILE 某 conversation 有未读 @mention，THE Web_Admin SHALL 在 Social Tab 上显示 `@` 角标。
5. WHEN Developer 长按并选择「撤回」自己发送的消息，THE Minos_Backend SHALL 将 `recalled_at_ms` 写入该消息行，并通过 WS 广播，THE 其他 member 的客户端 SHALL 以「此消息已撤回」展示。
6. THE Minos_Backend SHALL 拒绝非成员访问 `/v1/conversations/:conversation_id/...`，并在 404 时返回 `not_found`。

### R7 Observability

**User Story**：作为开发者或 host owner，我希望在出问题时能查到 request trace、日志、并把它们导出给协作者。

**Acceptance Criteria**

1. THE Minos_Daemon SHALL 以 mars-xlog 将 tracing 事件写入 `~/Library/Logs/Minos/daemon-*.xlog`（或 `$MINOS_HOME/logs`）；日志字段至少包含 `device_id`、`peer_device_id`、`rpc_method`（当存在时）。
2. THE Mobile_Client SHALL 提供日志面板（已有 `LogViewerPage`）支持最近 N 条日志的实时追尾与导出。
3. THE Mobile_Client SHALL 提供 request trace 面板（已有 `RequestTracePanel`）支持查看最近 HTTP/RPC 的 method、target、status、duration、request/response summary。
4. THE Web_Admin SHALL 提供等价的日志 + request trace 面板（ring buffer 保留最近 ≥ 500 条 HTTP/RPC trace）。
5. THE Minos_Backend SHALL 在每个 HTTP 请求日志中输出 `method`、`path`、`status`、`latency_ms`、`account_id`（若已认证），且 THE Minos_Backend SHALL **不** 记录 `password` / `refresh_token` / `device_secret` 的原始值。
6. THE Minos_Backend SHALL 在 `/health` 上返回 `ok` + backend 版本；IF 数据库连接不可达，THEN THE Minos_Backend SHALL 返回 HTTP 503。

### R8 跨平台运行时与数据持久化

**User Story**：作为 host owner / developer，我希望运行时文件落在可预测位置，且支持环境变量覆盖以方便测试。

**Acceptance Criteria**

1. THE Minos_Daemon SHALL 在启动时解析 `$MINOS_HOME`，默认回退到 `~/.minos`；SQLite、device_secret、workspaces 全部放在该目录下。
2. THE Minos_Daemon SHALL 在 daemon SQLite 连接上启用 WAL 模式。
3. THE Mobile_Client SHALL 将 pairing 状态（`PersistedPairingState`：`device_id` / `access_token` / `access_expires_at_ms` / `refresh_token` / `account_id` / `account_email`）通过 Dart 侧安全存储（iOS Keychain）持久化。
4. THE Web_Admin SHALL 仅在 `localStorage` 保存 `minos.web.session`（access/refresh token + account metadata）、`minos.web.device-id`、`minos.web.active-host`、`minos.web.workspace`，且 THE Web_Admin SHALL 在 logout 时清空上述键。
5. IF 运行环境缺少 `MINOS_JWT_SECRET`（backend）或 `MINOS_BACKEND_URL`（host/mobile/web），THEN 对应 System SHALL 在启动/bootstrap 阶段报错并拒绝进入正常流程。

### R9 Parser / Serializer（round-trip 要求）

**User Story**：作为协议维护者，我希望 `Envelope`、`UiEventMessage`、`PairingQrPayload` 这类跨网络类型保证 `parse(serialize(x)) == x`。

**Acceptance Criteria**

1. THE Minos_Protocol SHALL 提供 `Envelope` 的 serde 序列化与反序列化；FOR ALL 合法 `Envelope` 值，`serde_json::from_str(serde_json::to_string(x)) == x`（已在 `crates/minos-protocol/src/envelope.rs` 有 unit test，继续持有）。
2. THE Minos_UI_Protocol SHALL 对 `UiEventMessage` 全部 variant 保持 JSON round-trip 属性，包括 `Raw { raw_kind, payload_json }` 兜底项。
3. THE Minos_Protocol SHALL 提供 `PairingQrPayload` 的序列化与反序列化；Web_Admin、Mobile_Client SHALL 使用同一解析器，不得自行实现 ad-hoc 解析。
4. WHERE 添加新的 UI 事件 variant，THE Minos_UI_Protocol SHALL 为该 variant 补充 round-trip 测试用例后才合入。

### R10 Web Admin 专项

**User Story**：作为 web admin，我希望在浏览器里完成 slock.ai 控制台常见的 host/agent/thread/社交/skills 全流程。

**Acceptance Criteria**

1. THE Web_Admin SHALL 在单一登录会话内支持以下入口：host 切换、pairing（粘贴 QR JSON 或 token）、agent thread 列表、thread 详情（含中断）、host skills 列表、social 会话、最近日志、最近 request trace。
2. WHERE Browser_Admin 所在标签被 `visibilitychange` 置为 `hidden` 超过 30 秒，THE Web_Admin SHALL 关闭 WS；WHEN 重新 `visible`，THE Web_Admin SHALL 重新调用 `/v1/auth/ws-ticket` 并重建 `RelaySocket`。
3. WHEN WS 状态变为 `error` 或 `closed` 且当前有活跃 thread 订阅，THE Web_Admin SHALL 以线性退避（1s → 2s → 5s，最多 3 次）自动重连；IF 仍失败，THEN THE Web_Admin SHALL 显示「relay offline」并暴露手动重试按钮。
4. THE Web_Admin SHALL 保持 `App.tsx` 的三个 workspace tab（console / agents / social）的心智不变，但 `console` tab 内需要在左侧列出 host 与其「在线状态 / agent 可用性」，右侧显示 thread 列表 + 详情 + composer。
5. IF Developer 选中某 thread 且 WS 处于 `closed`，THEN THE Web_Admin SHALL 仍显示冷启动 `/v1/threads/read` 的历史内容，并明确标注「离线」。

## 7. 非功能性需求（NFR）

| 类别 | 要求 |
| --- | --- |
| 性能：thread 列表 | `/v1/threads/query` 在 `limit=50` 时 p95 < 200 ms（单实例，典型 SQLite 规模） |
| 性能：ingest 写入 | backend 对单个 `Envelope::Ingest` 从接收到 fan-out 完成 p95 < 50 ms（同机进程） |
| 稳定性：reconnect | mobile/web 在网络中断 ≤ 60 秒时应重连成功，不丢失已 `raw_events` 写入的 UI 事件（host 端通过 `IngestCheckpoint` 补齐） |
| 安全：密码存储 | backend 使用 argon2id；不以明文记录密码；refresh token 以 SHA-256 哈希入库 |
| 安全：rate limit | register/login/refresh 保持现有 per-IP/per-email/per-account 限流 |
| 可观测：日志字段 | 每个跨进程日志至少携带 `device_id` 或 `account_id` 之一 |
| 可用：离线阅读 | mobile/web 断网时仍可查看已加载的 thread 历史 |
| 兼容：wire 版本 | 保持 `v:1` 不变；新增 variant 通过 `EventKind` / `UiEventMessage` 扩展（ADR 0011） |
| 可维护：契约测试 | `crates/minos-protocol/tests/envelope_golden.rs`、`crates/minos-ui-protocol` round-trip 测试继续 CI-block |

## 8. 非目标（Non-Goals）与延后项

以下能力**本次 spec 不规划实现**；在后续独立 spec 中按需求重新评估：

- E2EE：端到端加密（X25519 / Ed25519 / AES-GCM）。
- Android 生产化发布（release 签名 / Play Store）。
- OAuth / SSO / 邮箱校验 / 2FA / 密码重置。
- APNs / FCM 推送。
- Mac app Sparkle / TestFlight 自动更新管线。
- 多实例 HA backend（shard、LB、Redis、跨 region）。
- Workspace 级 Git 操作（commit / branch / PR）。
- 多租户 / team 概念（仅账号 + 好友关系）。
- Agent profile 上云（目前仅客户端本地）。
- Tool approval 的全量 UI（仅预留，不强求本轮交付）。
- Mobile 本地持久化 thread / UI 事件数据库（继续依赖 backend 读 + live 广播）。

## 9. 风险与开放问题

1. **claude / gemini 的 wire 协议未定型**：不同 CLI 的 stdout 结构差异较大，R3.4 的 fallback 仅保证「能跑」；后续需要独立 spec 定义 PTY agent 协议（`docs/superpowers/specs/pty-agent-claude-gemini.md` 是历史占位，可复用）。
2. **多 host 同账号并行控制的语义**：当前 mobile/web 都以单一 `activeHost` 为 RPC 目标；如果未来需要「同时向 2 台 host 发 prompt」，需要改 UI 和 backend registry 的 per-session account 广播策略（§5.2 第 6 列已说明本轮不做）。
3. **Web localStorage 的安全边界**：access/refresh token 在 localStorage 对 XSS 敏感；本轮不引入 HttpOnly cookie；需要在 §7 安全条目里明确「Web 只作 admin 本地环境使用，不对外暴露」。
4. **Backend 持久化 mars-xlog**：R7.1 要求新增文件日志，需验证在容器化部署下 `$MINOS_HOME` 或等价路径可写。
5. **Approval / sensitive op 占位**：R3 没有强制 approval UI；一旦 slock.ai 对标的 approvals 功能推进，需要在独立 spec 中重绘 thread 详情的交互。
6. **Web WS 重连的幂等性**：R10.3 的线性退避需要与 backend `Event::ServerShutdown` 的重连策略一致（ADR 0011）。

## 10. 附录 A：与历史文档的差异说明

- `docs/superpowers/specs/minos-architecture-and-mvp-design.md` 提到的 Tailscale/P2P、Sparkle、Tier A/B 概念已不再是当前事实 source；本 spec 以代码实现（Cloudflare Tunnel + backend relay）为准。
- `docs/superpowers/plans/02-macos-app-and-uniffi.md` 等 plan 文档记录的是实施路径，并非规划目标；本 spec 优先采信 `crates/` 和 `apps/` 的最新代码。
- ADR 0020「server-centric auth and account pairs」已落地：iOS 不持有 `DeviceSecret`，pairing 走多对多 `account_host_pairings` 表；本 spec 所有 R2 条目围绕这套事实。
- Mobile 端 `AgentProfile` 与 Web 端 `lib/agent-profiles.ts` 属于**客户端本地偏好**，不对应 backend 表；历史上曾讨论 cloud-sync profile，本轮明确**不做**（§8）。
- Social IM（friends / conversations / messages / mentions / reply / recall）是 backend 已完整落地的新能力，历史 MVP 文档未覆盖；本 spec 首次把它纳入全功能对标矩阵。
