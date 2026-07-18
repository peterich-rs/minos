# Minos Desktop 产品体验设计

> 日期: 2026-07-18  
> 状态: 草案（产品视角）  
> 类型: Desktop IA / UX  
> 视觉参考: `res/desktop.jpeg`（气质与密度，非业务隐喻）  
> 域对齐: conversation-centric 层级（Project → Conversation → Agent session）  
> 实现入口: `apps/desktop`（Tauri + React）

---

## 1. 一句话定位

**Minos Desktop 是跑在 Mac Host 上的「本机 AI 编码指挥台」**：管理本机项目与对话、编排多个 coding agent、处理审批与运行态，并把手机/Web 远程控制所依赖的 host 能力在本机可视化。

它**不是**：

- 客服 Inbox / 工单系统（效果图的业务隐喻）
- 第二个 Web 管理后台（那是远程账户视角）
- 单纯的 ChatGPT 桌面套壳（单会话、无 project/agent 编排）

它**是**：

- TUI 的图形化主界面（能力对标，体验升级）
- macOS menu bar app 的「展开态」（配对/状态可收进托盘，主窗口做重工作）
- 与 Mobile 共享同一套域语言（Project / Conversation / Agent session）

---

## 2. 效果图能借什么、必须丢掉什么

### 2.1 保留（视觉与交互语法）

| 模式 | 为什么有用 |
|------|------------|
| 多栏固定工作台 | 编码协作需要列表 + 主时间线 + 上下文同时可见 |
| 浅色暖底、圆角壳、轻阴影 | 长时间盯屏友好，区别于 IDE 深色疲劳 |
| 左侧分层导航 + 中栏列表 + 主内容 + 右侧 inspector | 信息架构清晰，可映射我们的层级 |
| 气泡 + 输入栏 | 对话是主交互，不是文件树 |
| 状态色点 / 未读徽标 / 轻量筛选 | 运行态与待办要一眼可见 |
| 选中高亮（粉/强调色） | agent / conversation 选中反馈 |

### 2.2 丢掉或重写（业务隐喻）

| 效果图概念 | 问题 | Minos 替换 |
|------------|------|------------|
| Inbox / Mentions / Unassigned / Spam | 客服队列语义 | **Projects / Conversations / Needs attention** |
| Customer care 订单详情 | CRM 字段 | **Conversation / Agent / Workspace 元数据** |
| Team discussion: Design/Dev/Management | 真人组织架构房间 | **Project 下的 Conversations**（可多人/多 agent） |
| Agents 像外部 SaaS 集成 | 像接了 ChatGPT 插件 | **本机 runtime 列表**（codex/claude/gemini/opencode/grok） |
| Assignee / Priority / Labels 工单 | 客服 SLA | **审批队列 / Agent 状态 / Skills·Model** |
| Upgrade 按钮 | SaaS 变现 | 去掉或改为 Host/Relay 状态 |

**原则：抄布局密度与气质，不抄「客服产品」的信息架构。**

---

## 3. 用户与场景

### 3.1 主用户（Primary）

**本机开发者 / Host 操作者**，坐在装了 Minos daemon 的 Mac 前：

1. 在某个 repo（Project）里开对话，让 1～N 个 agent 写代码  
2. `@codex` / `@claude` 分工，看主时间线与每个 agent 的细节  
3. 处理权限审批（写文件、跑命令）  
4. 看 agent 是否在跑、卡在 tool、失败了  
5. 偶尔管理配对（手机要控这台 Mac）

### 3.2 次要用户

- 同一 Host 上协作的另一人（远期）  
- 远程手机用户不直接「用 Desktop」，但 Desktop 要让 Host 在线、可配对、可观测  

### 3.3 Jobs to be Done（按频率）

| 频率 | Job | 失败时的代价 |
|------|-----|--------------|
| 极高 | 进 Project → 找/建 Conversation → 发消息 / @agent | 产品不可用 |
| 高 | 看 agent 是否在工作、流式输出、工具调用 | 不信任系统 |
| 高 | 处理审批（允许/拒绝） | 任务卡住 |
| 中 | 打开某个 agent session 看完整 transcript | 排障失败 |
| 中 | 切换 Project / workspace | 搞错目录改坏别的 repo |
| 低 | 配对手机、看 relay 状态 | 远程断联 |
| 低 | 装/检测 CLI runtime、看 skills | 启动失败 |

---

## 4. 域语言（桌面与 TUI / Mobile 对齐）

统一用语，UI 文案与导航只用这些：

| 术语 | 含义 | 用户怎么理解 |
|------|------|--------------|
| **Host** | 这台 Mac + daemon | 「我的电脑」 |
| **Project** | 绑定 workspace 的工作单元 | 「这个仓库/任务空间」 |
| **Conversation** | Project 下的对话容器 | 「一场协作讨论」 |
| **Timeline** | Conversation 主时间线 | 「大家和 agents 说了什么」 |
| **Agent** | 某类 runtime（codex…） | 「哪种 AI 工人」 |
| **Agent session** | Conversation 内一次 agent run | 「这个工人这次活」 |
| **Approval** | 待用户确认的权限请求 | 「它想动你的电脑，允不允许」 |
| **Workspace** | 本机路径 | 「代码在哪个文件夹」 |

导航栈（与 TUI 一致）：

```text
Projects
  → Conversations (within project)
       → Conversation workspace
            → Agent session detail (optional drill-in)
```

---

## 5. 桌面信息架构（推荐）

### 5.1 总览线框

```text
┌────────┬──────────────┬─────────────────────┬──────────────────┐
│ Icon   │ Context      │ Main                 │ Inspector        │
│ rail   │ list         │ (changes by mode)    │ (context-aware)  │
│        │              │                      │                  │
│ Home   │ Projects /   │ Conversation         │ Members /        │
│ Work   │ Convos /     │ timeline + input     │ Sessions /       │
│ Agents │ Attention    │  或 Agent transcript  │ Workspace /      │
│ Host   │              │  或 Agents overview  │ Approvals /      │
│        │              │                      │ Runtime config   │
└────────┴──────────────┴─────────────────────┴──────────────────┘
         ▲ always present when in Work mode
```

### 5.2 Icon rail（一级导航，4 项就够）

| 项 | 名称 | 职责 |
|----|------|------|
| 1 | **Work** | 默认首页：Project → Conversation 主路径 |
| 2 | **Attention** | 需要人介入：审批、失败 session、@你、断连 |
| 3 | **Agents** | 本机 runtime 目录：已安装 CLI、模型/skills、健康状态（不是客服 bot 列表） |
| 4 | **Host** | 这台机器：Relay 连接、配对 QR、日志路径、daemon 版本 |

少即是多。效果图里的 Board / Insights / People 对 Minos v1 **不做**。

### 5.3 Work 模式（核心，占 90% 使用时间）

三栏 + 可选第四栏：

#### A. Context list（左中栏）

**层 1 — Projects**

- 列表项：名称、workspace 短路径、活跃 conversation 数、是否有 running agent  
- 操作：新建 Project（名 + workspace 选目录）、搜索  

**层 2 — Conversations**（进入 Project 后）

- 列表项：title、last message preview、时间、未读/运行中角标  
- 操作：新建 Conversation、筛选（全部 / 有运行中 agent / 有待审批）  
- 面包屑：`Project name › Conversations`

#### B. Main — Conversation workspace

默认主视图：

1. **顶栏**  
   - Conversation title（**双击内联编辑**，Enter 提交 / Esc 取消）  
   - 创建时 git branch 快照 chip（可选 worktree path）；不跟随 Project 后续切分支  
   - Priority / Progress 标签（点击循环；Priority 可清空）  
   - 状态：`3 agents · 1 running · 1 needs approval`  
   - Progress 枚举：`todo | in_progress | in_review | done`（任务级）；Board **Needs you** 列是运行态（审批等），拖过去时 progress 写 `in_progress`，**不用** `in_review`  
   - Board 列派生：`done` 优先固定 Done；有待审批 → Needs you；running / in_progress / in_review → Running；否则 Backlog  


2. **主时间线（Timeline）**  
   - user / agent 消息（对齐 conversation messages）  
   - agent 最终回复进时间线；细节 tool stream 可折叠或点进 session  
   - `@agent` 提及高亮  

3. **输入栏（核心交互）**  
   - 占位：`Message the team…  use @codex or @claude to start an agent`  
   - `@` 弹出本机已安装 agents + 当前 conversation 内可续聊的 session（`@codex#a1b2`）  
   - 发送 / 停止当前 run（interrupt）  
   - 附件/路径补全可后置  

#### C. Inspector（右栏，随选中变化）

| 选中对象 | Inspector 内容 |
|----------|----------------|
| Conversation（默认） | **Participants**：Humans + Agents；**Sessions** 树（含 subagent）；**Workspace**；快捷「Start agent」 |
| Agent session | Online/Running/Idle/Failed；model/runtime；当前 tool；Open transcript；Stop |
| 空 | Project 级：workspace、default agent、最近 activity |

右侧 **不要** 放订单 ID / Customer email 一类字段。

### 5.4 Agent session detail（钻入，非默认）

从 Inspector 点 session 或时间线里的 agent 卡片进入：

- Main 切换为该 session 的 **完整 transcript**（stream、tool、diff、reasoning）  
- 顶栏可「返回 Conversation」  
- 输入：对顶层 session 可续聊（路由与 TUI `@agent#short` 一致）；subagent 只读  

这对应 TUI 的 `AgentDetail`，桌面用「同窗切换」而不是新开客服工单页。

### 5.5 Attention 模式

统一「要你动手」的队列，比散落在各 conversation 更好扫：

| 类型 | 示例 | 主 CTA |
|------|------|--------|
| Approval | codex 要写 `src/foo.rs` | Allow / Deny（可记一次会话策略后续再做） |
| Failed run | claude session error | Open session / Retry（若支持） |
| Stuck / waiting | 等待用户问题（agent question） | Answer |
| Host/Relay | 断连、配对过期 | Reconnect / Show QR |

Badge 打在 Icon rail 的 Attention 上。  
**这是效果图 Inbox 唯一值得继承的「待办感」，语义必须是 engineering attention，不是客服工单。**

### 5.6 Agents 模式（目录，不是聊天）

- 检测结果：codex / claude / gemini / opencode / grok 是否 installed  
- 每条：版本/path 摘要、默认 model（若可知）、Open skills 路径  
- CTA：文档链接、重新检测  
- **不在这里聊天**；聊天永远发生在 Project → Conversation  

Create Agent 面板（`res/create-agent-panel.png`）应改写为：

- **Start agent in conversation**（选 runtime + 可选 model/effort + env）  
- 或 **Project default agent**  
而不是「创建一个永远在线的客服机器人档案」。

### 5.7 Host 模式

合并今日 menu bar 职责：

- Relay：Connected / Disconnected  
- Pairing：QR + 倒计时 + 重新生成  
- Daemon version、日志目录  
- 开机启动等系统项（后置）  

主窗有 Host 页后，menu bar 可瘦身为状态指示 + 打开主窗。

---

## 6. 关键用户流程（Happy path）

### 6.1 首次打开

1. Desktop 启动 → 确保 daemon 在跑（已有则连，没有则托管，行为对齐 TUI）  
2. 若未配对且用户需要远程：Host 页提示配对；**不阻塞本地编码**  
3. 若无 Project：空态引导「Create your first project」→ 选文件夹  

### 6.2 日常编码协作

1. Work → 选 Project  
2. 选或新建 Conversation  
3. 输入：`@codex 把登录改成 JWT`  
4. 主时间线出现 user 消息 + agent 开始跑  
5. Inspector 里 session 变 Running；需要时点进看 tool stream  
6. 弹出 Approval → Attention badge + 内联卡片 → Allow  
7. Agent 完成；时间线出现摘要回复  
8. 可再 `@claude 帮我写测试` 拉第二个 agent（同 conversation）  

### 6.3 多 agent / teamwork

- Conversation 是「房间」；多个 agent session 是「房间里的工人」  
- 委托/MCP 完成事件回到同一时间线（与现有 teamwork 模型一致）  
- UI 用 session 树 + 时间线引用表达，不另造「Team discussion」频道类型  

---

## 7. 布局与视觉产品原则

1. **Conversation 是重心**：主区域默认永远是时间线，不是列表海。  
2. **Project 是安全边界**：workspace 路径在顶栏常显，防止改错目录。  
3. **运行态优先于装饰**：Running / Needs approval / Failed 的可见性 > 漂亮插画。  
4. **深度可及、默认不吵**：tool 细节默认折叠或进 session；主时间线保持可读。  
5. **键盘与鼠标同等**：桌面用户会要 `Cmd+K` 跳 Project/Conversation、`@` 选 agent、`Esc` 返回；与 TUI 快捷键可概念对齐，不必键位一一相同。  
6. **一套域语言跨端**：Desktop / TUI / Mobile 用同一名词，降低心智税。  
7. **本地优先**：Desktop 主数据来自 daemon local RPC；不是浏览器登录后的云 Inbox。  

视觉上继续用效果图的：

- canvas 暖灰、surface 米白、ink 石色、accent 粉作选中/重要 CTA  
- 大圆角 shell、细分割线、轻阴影  

但组件命名与文案全面 Minos 化。

---

## 8. MVP 范围建议

### Phase 0 — 已完成倾向

- 多栏壳 + 视觉 token（当前 scaffold）  
- **下一步：把 mock 文案/模块改成 Minos IA 骨架（仍可 mock 数据）**

### Phase 1 — 可用的本机指挥台

- 接 daemon：projects / conversations / messages / agent sessions  
- Work 模式完整：列表 + 时间线 + 输入 + @agent 启动  
- Inspector：sessions 列表 + workspace  
- 基础流式展示与错误 toast  

### Phase 2 — 人机协作闭环

- Approvals（Attention + 时间线内联）  
- Agent session detail transcript  
- Interrupt / 失败态  
- Host：relay 状态 + 配对 QR  

### Phase 3 — 精致与增强

- Cmd+K、完整快捷键  
- Skills / model / create-agent 高级配置  
- Subagent 树可视化打磨  
- 通知、托盘、窗口状态记忆  
- 与 menu bar app 体验合并  

**明确不做（v1）**：客服字段、SaaS Upgrade、Insights 看板、独立「Team member 社交」页（除非以后做多人 Host 协作）。

---

## 9. 与其它表面的分工

| 表面 | 角色 |
|------|------|
| **Desktop** | Host 主 UI：编码协作、审批、本机可观测 |
| **TUI** | 同能力的终端形态；高级用户 / SSH / 无图形环境；可长期保留 |
| **macOS menu bar** | 轻量：状态 + 打开 Desktop + 紧急配对；避免再做第二套完整 UI |
| **Mobile** | 远程遥控：路上审批、看进度、轻量发指令 |
| **Web** | 账户/设备/远程管理；不是 Host 本机指挥台 |

---

## 10. 成功标准（产品）

用户在 **5 分钟内** 能完成：

1. 打开 Desktop  
2. 选中或创建一个 Project（指向真实 repo）  
3. 开 Conversation  
4. `@` 一个已安装 agent 并看到它开始跑  
5. 完成一次审批（若触发）  
6. 在时间线看到结果  

同时：

- 不需要理解「Inbox / Spam / Customer」  
- 不需要先登录浏览器账号才能本地用（本地 daemon 路径）  
- 名词与 TUI 文档一致，支持文档/口口相传  

---

## 11. 开放问题（需产品拍板）

1. **Desktop 是否默认托管 daemon**（对齐 TUI）还是必须依附已有 menu bar/daemon？  
2. **Account 登录**是否出现在 Desktop v1？建议 v1 本地优先，登录仅服务「同步远程/多设备」后置。  
3. **人类多人同 Host 协作**是否在 Desktop v1 暴露？建议先单操作者 + 多 agent。  
4. **效果图右侧 Details 的信息密度**：session 元数据够不够，是否要内嵌简单 diff 预览？  
5. **品牌名在壳上的呈现**：窗口标题 Minos、rail 只用 mark。  

---

## 12. 结论

- 效果图是 **优秀的视觉与多栏参考**，不是 **业务原型**。  
- Minos Desktop 应建成 **Project → Conversation → Agent session 指挥台**，用 Attention 承接「待办」，用 Host 承接「这台机器」。  
- 交互主轴是 **@agent 编码协作 + 审批 + 运行态**，不是客服收件箱。  
- 实现上：先改 scaffold 的 IA/文案对齐本文，再接 daemon；避免在错误隐喻上堆功能。
