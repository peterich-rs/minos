# Daemon 常驻生命周期 + Agent Mention 会话作用域修复

> 日期: 2026-06-23
> 状态: Issue 1 已实现；Issue 2 设计中
> 类型: Bug 修复 + 架构改进
> 关联: `2026-06-18-conversation-centric-hierarchy-design.md`

## 1. 问题

### 1.1 Issue 1: `@Agent#hashid` 跨会话泄漏

在 conversation 中输入 `@` 选择已有 agent session 时，候选列表显示了**不属于当前 conversation** 的 agent session hashid。

**根因:** 修复前 `room_agent_mention_candidates()` (`crates/minos-tui/src/ui/mod.rs:250`) 从 `self.threads`（全局线程列表，包含所有 conversation 的 agent）而非 `self.conversation_agent_sessions`（当前 conversation 的 session 列表）获取 "existing" 候选。`thread_id_for_agent_short_id()` (`submission.rs:414`) 同样从全局 `self.threads` 解析，保持了一致的错误。

### 1.2 Issue 2: TUI 重启后 agent session 全部关闭

用户关闭 TUI 再打开后，之前用过的 agent session 处于 Closed 状态，无法继续使用。

**根因:** 当 TUI 没有发现外部 daemon 时，`start_managed_daemon_for_tui()` (`main.rs:241`) 在 TUI 进程**内部**运行 daemon（共享 Tokio runtime）。TUI 退出时 `main.rs:423-424` 调用 `managed_daemon.stop()` 杀掉 daemon 及其所有 agent thread。下次启动时新 daemon 从零开始，旧 thread 状态丢失或变为 Closed/Suspended。

### 1.3 目标行为

Daemon 是常驻服务，TUI 是可随时连接/断开的视图层：

| 操作 | TUI | Daemon | Agent Threads |
|------|-----|--------|----------------|
| **Ctrl+Q** | 退出 | **存活**，继续运行 | 状态不变 |
| **Ctrl+C** (无可中断 thread) | 退出 | **停止**（优雅关闭所有 agent） | Closed |
| **Ctrl+C** (有可中断 thread) | 不退出 | 存活 | 中断当前 thread |
| **`kill <pid>` / SIGTERM** | 不受影响（重连或退出） | 停止 | Closed |
| **TUI 重启** | 连接到存活 daemon | 状态连续 | 保持之前状态 |

## 2. 设计

### 2.1 Issue 1 修复：Agent Mention 会话作用域

#### 2.1.1 候选列表来源切换

`room_agent_mention_candidates()` (`ui/mod.rs:250`) 根据当前 nav level 判断 "existing" 候选来源：

- **在 conversation 内** (`nav_level().conversation_id().is_some()`): 从 `self.conversation_agent_sessions` 取顶层、未关闭候选
- **不在 conversation 内**: 只展示 installed agents。Conversations 列表底部输入框用于创建新 conversation，不能展示其他 conversation 的 session hash。

> 注意：当前 TUI `NavLevel` 只有 `Projects / Conversations / Conversation / AgentDetail`，尚未建模独立的 `Agent`/`Agents` 层。不要把 Conversations 层当作全局 Agent 层；它没有 active conversation，因此不能暴露 existing session hash。

`ThreadSummaryEntry` (`backend/mod.rs:70`) 缺少 `state` 字段。利用 `ended_at_ms: Option<i64>` 作为 closed 判断依据（`Some` = 已结束 = 跳过），或者给 `ThreadSummaryEntry` 添加 `state` 字段。**推荐添加 `state` 字段**，因为 `ended_at_ms` 语义不够精确（Suspended thread 不会有 `ended_at_ms` 但也不应出现在 mention 候选中——实际上 Suspended thread 可以接受消息所以应出现）。

#### 2.1.2 短 ID 解析对称更新

`thread_id_for_agent_short_id()` (`submission.rs:414`) 同样需要根据 nav level 从对应列表解析。如果当前在 conversation 内，只搜索 `conversation_agent_sessions`。

#### 2.1.3 影响范围

- `crates/minos-tui/src/ui/mod.rs` — `room_agent_mention_candidates()` 添加 nav level 参数或内部判断
- `crates/minos-tui/src/app/submission.rs` — `thread_id_for_agent_short_id()` 对称更新
- `crates/minos-tui/src/backend/mod.rs` — `ThreadSummaryEntry` 可能添加 `state` 字段
- `crates/minos-tui/src/backend/daemon.rs` / `embedded.rs` — `list_conversation_agent_sessions` 实现需要返回 state；daemon 端必须采用与 `get_thread` 相同的 live-manager-state 优先、DB row fallback 策略（见 `crates/minos-daemon/src/agent.rs:763-782`），否则运行中线程的 state 可能滞后，mention 过滤 closed/open 会不准
- `crates/minos-protocol/` — `ThreadSummary` 如果缺少 state 也需添加

### 2.2 Issue 2: Daemon 常驻化

#### 2.2.1 TUI 通过独立子进程启动 Daemon

**当前:** `start_managed_daemon_for_tui()` 在 TUI 进程内调用 `DaemonHandle::start_with_local_rpc()`，daemon 作为 in-process task 运行。

**改为:** 通过 `std::process::Command` 启动 `minos-daemon start --local-rpc` 作为**独立子进程**，使用 `setsid`（Unix）脱离 TUI 进程组：

```rust
// spawn 只负责启动子进程；等待 discovery 由调用方负责，
// 以便在 spawn 前清理 stale discovery 文件、捕获 pre-spawn mtime。
fn spawn_daemon_subprocess(discovery_path: &Path) -> Result<()> {
    // 优先使用 PATH 中的 minos-daemon；备选：解析同 workspace 编译产物路径
    let mut cmd = std::process::Command::new("minos-daemon");
    cmd.args(["start", "--local-rpc"]);
    
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        unsafe { cmd.pre_exec(|| { libc::setsid(); Ok(()) }); }
    }
    
    cmd.stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    Ok(())
}
```

调用方 (`connect_or_start_daemon_backend`) 在 spawn 之前必须：
1. 清理 stale discovery 文件（避免读到旧 daemon 留下的 URL）
2. 捕获 pre-spawn discovery 文件 mtime
3. spawn 后等待 discovery 文件 mtime 变新（证明是新 daemon 写入的）
4. 读到 URL 后 connect 成功才算 daemon 启动完成

关键点：
- `setsid()` 让子进程脱离 TUI 的 session group，TUI 退出不会导致 daemon 收到 SIGHUP
- TUI 不持有 `DaemonHandle`，只作为客户端连接
- `connect_or_start_daemon_backend()` 返回 `(backend, None)` —— 不再有 managed handle 需要在退出时 stop

#### 2.2.2 移除 TUI 退出时的 daemon stop

**当前 `main.rs:421-425`:**
```rust
app.shutdown().await;
restore_terminal(&mut terminal)?;
if let Some(handle) = managed_daemon {
    handle.stop().await?;
}
```

**改为:** 根据 quit mode 区分：

```rust
app.shutdown(quit_mode).await;
restore_terminal(&mut terminal)?;
// Ctrl+Q: 什么都不做，daemon 存活
// Ctrl+C (hard quit): 发送 stop RPC 给 daemon
if quit_mode == QuitMode::HardShutdown {
    if let Some(daemon_endpoint) = app.daemon_endpoint() {
        let _ = stop_daemon_via_rpc(&daemon_endpoint).await;
    }
}
```

#### 2.2.3 Quit Mode 区分

引入 `QuitMode` 枚举：

```rust
pub enum QuitMode {
    /// Ctrl+Q: TUI 退出，daemon 存活
    Soft,
    /// Ctrl+C (无可中断 thread): TUI 退出 + daemon 停止
    HardShutdown,
}
```

当前 `Effect::Quit` (Ctrl+Q) → `QuitMode::Soft`
当前 `Effect::InterruptOrQuit` 的 fallback 分支 (Ctrl+C 无可中断 thread) → `QuitMode::HardShutdown`

`App` 记录 `quit_mode: Option<QuitMode>`，`main.rs` 读取后决定是否 stop daemon。

#### 2.2.4 Ctrl+C 向 daemon 转发 stop

Hard shutdown 时，TUI 通过已有的 local RPC 连接调用一个新方法 `shutdown_daemon`，daemon 收到后执行 `DaemonHandle::stop()`。

`LocalDaemonRpc` trait 已有 `#[rpc(..., namespace = "minos_local")]`（见 `crates/minos-protocol/src/local_rpc.rs:92`），jsonrpsee 会自动给所有方法加上 `minos_local_` 前缀。因此 trait 内只需声明短名 `shutdown_daemon`，调用端使用全名 `minos_local_shutdown_daemon`：

```rust
// trait 声明 — 用短名，jsonrpsee 自动加 namespace 前缀
#[method(name = "shutdown_daemon")]
async fn shutdown_daemon(&self) -> RpcResult<()>;
```

```rust
// TUI 调用端 — 用全名
client
    .request::<(), _>(
        "minos_local_shutdown_daemon",
        jsonrpsee::core::params::ArrayParams::new(),
    )
    .await?;
```

daemon 端实现：调用自身的 `stop()`，触发优雅关闭序列。

#### 2.2.5 PID 文件 + 崩溃恢复

Daemon 启动时写 PID 文件到 `$MINOS_HOME/run/tui-daemon.pid`。

TUI 启动时 `connect_or_start_daemon_backend()` 流程增强：

1. 读取 discovery 文件获取 URL
2. 尝试连接
3. **连接失败时**：读取 PID 文件，检查进程是否存活
   - PID 不存活：删除 stale discovery 文件和 PID 文件，重新 spawn daemon
   - PID 存活但连接失败：可能是 daemon 卡住，记录警告，可强制重启（或提示用户手动处理）
4. 连接成功：正常使用

Daemon `stop()` 时删除 PID 文件（与 discovery 文件一起清理）。

#### 2.2.6 Daemon 关闭路径

Daemon 的关闭只通过两条路径触发，均走 `shutdown_daemon` RPC method（wire name `minos_local_shutdown_daemon`）：

1. **TUI Ctrl+C (HardShutdown)** — TUI 通过已有 WebSocket 连接调用 RPC
2. **系统信号 (SIGINT/SIGTERM)** — daemon `start` 命令自身的 `wait_for_termination()` 已处理

不提供 `minos-daemon stop` CLI 子命令（避免 `minos-daemon` 二进制可用性问题）。需要手动停止 daemon 时，用户可直接 `kill <pid>`（PID 可从 `$MINOS_HOME/run/tui-daemon.pid` 读取）。

## 3. 变更清单

### Issue 1: Agent Mention 会话作用域

| 文件 | 变更 |
|------|------|
| `crates/minos-tui/src/ui/mod.rs:250` | `room_agent_mention_candidates()` 在 conversation 内时从 `conversation_agent_sessions` 取候选 |
| `crates/minos-tui/src/app/submission.rs:414` | `thread_id_for_agent_short_id()` 对称更新，在 conversation 内时只搜索 `conversation_agent_sessions` |
| `crates/minos-tui/src/backend/mod.rs:70` | `ThreadSummaryEntry` 添加 `state` 字段（或使用 `ended_at_ms` 过滤） |
| `crates/minos-protocol/` | `ThreadSummary` 如需添加 `state` 字段则同步更新 |
| `crates/minos-tui/src/backend/daemon.rs` | `list_conversation_agent_sessions` 返回 state（live-manager 优先） |
| `crates/minos-tui/src/backend/embedded.rs` | 同上 |
| `crates/minos-tui/src/app_tests/` | 添加测试：conversation 内 mention 只显示当前 conversation 的 agent |

### Issue 2: Daemon 常驻化

| 文件 | 变更 |
|------|------|
| `crates/minos-tui/src/main.rs:241-265` | `start_managed_daemon_for_tui()` 改为 `spawn_daemon_subprocess()`，用 `setsid` 启动独立进程 |
| `crates/minos-tui/src/main.rs:267-300` | `connect_or_start_daemon_backend()` 移除 `Option<Arc<DaemonHandle>>` 返回值，改为纯连接逻辑 + 崩溃恢复 |
| `crates/minos-tui/src/main.rs:421-425` | 根据 `quit_mode` 决定是否 stop daemon |
| `crates/minos-tui/src/app/lifecycle.rs` | `shutdown()` 接受 `QuitMode` 参数；添加 `quit_mode` 字段和 setter |
| `crates/minos-tui/src/app/event_loop.rs:116-118` | `Effect::Quit` 设置 `QuitMode::Soft` |
| `crates/minos-tui/src/app/event_loop.rs:617` | Ctrl+C fallback 设置 `QuitMode::HardShutdown` |
| `crates/minos-protocol/src/local_rpc.rs` | 新增 `shutdown_daemon` RPC method 定义（trait 内短名，wire name 为 `minos_local_shutdown_daemon`） |
| `crates/minos-daemon/src/local_rpc.rs` | 实现 `shutdown_daemon` RPC |
| `crates/minos-daemon/src/handle.rs:330-357` | `stop()` 同时删除 PID 文件 |
| `crates/minos-daemon/src/handle.rs` | `start_with_local_rpc()` 写 PID 文件 |
| `crates/minos-tui/src/main.rs` | 新增 `wait_for_new_discovery()`（基于 mtime 防止读到 stale discovery）+ spawn 前清理 stale discovery + PID 文件崩溃恢复逻辑 |
| `crates/minos-tui/src/app_tests/` | 更新测试：Ctrl+Q 不 stop daemon，Ctrl+C stop daemon |

### 不变的部分

- Daemon 本身的 agent 管理逻辑（`AgentManager`, `close_thread`, `resume_thread` 等）不变
- `ThreadState` 状态机不变（Closed 仍是终态——但 daemon 常驻后用户不会因 TUI 重启遇到意外的 Closed）
- `shutdown()` 的 embedded mode guard 不变（embedded mode 仍关闭所有 thread）
- Discovery 文件格式和路径不变
- Relay client 重连逻辑不变

## 4. 测试策略

### Issue 1

- 单元测试：`room_agent_mention_candidates()` 在 conversation nav level 下只返回 `conversation_agent_sessions` 中的 thread
- 单元测试：`thread_id_for_agent_short_id()` 在 conversation 内只解析当前 conversation 的 session
- 单元测试：Conversations/new-conversation input 不返回任何 existing session hash

### Issue 2

- 单元测试：`spawn_daemon_subprocess()` 使用 `setsid` 启动子进程（mock Command 或集成测试）
- 单元测试：`QuitMode::Soft` 不触发 daemon stop，`QuitMode::HardShutdown` 触发
- 单元测试：PID 文件崩溃恢复逻辑（stale PID → 清理 + respawn）
- 单元测试：stale discovery 防护——spawn 前删除旧 discovery 文件，`wait_for_new_discovery` 只接受 mtime 更新的新文件（模拟旧 daemon 崩溃后留有 discovery 文件的场景）
- 集成测试：Ctrl+Q 退出后 daemon 存活，TUI 重连后状态连续

## 5. 风险与注意事项

1. **`minos-daemon` 二进制可用性**：TUI 通过 `spawn_daemon_subprocess()` 启动 `minos-daemon start --local-rpc`，需要该二进制在 PATH 中或能解析到同 workspace 的编译产物。如果不可用，TUI 应报清晰错误提示用户安装或手动启动 daemon。

2. **setsid 平台兼容性**：`setsid` 是 Unix 特有。Windows 需要使用 `CREATE_BREAKAWAY_FROM_JOB` 标志。当前 Minos 主要支持 macOS（根据 AGENTS.md），但需要处理跨平台 fallback。

3. **discovery 文件竞争**：多个 TUI 实例同时启动可能竞争创建 daemon。需要原子化 spawn + discovery 文件检查（用文件锁或 `O_EXCL` 创建）。

4. **Ctrl+C 的双重语义**：用户可能习惯 Ctrl+C 退出 TUI。改为 Ctrl+C = 全局 shutdown 后，需要清晰的 UI 提示（如底部状态栏显示 "Ctrl+Q: quit | Ctrl+C: shutdown"）。

5. **Embedded mode 兼容**：Embedded mode（无 daemon）仍需要保持现有行为——TUI 退出时关闭所有 thread。`shutdown()` 的 embedded guard 保留。

## 6. 实现顺序

建议分两个独立 PR：

**PR 1: Issue 1 — Agent Mention 会话作用域修复**（小，低风险）
1. 给 `ThreadSummaryEntry` / `ThreadSummary` 添加 `state` 字段（如需要）
2. 修改 `room_agent_mention_candidates()` 和 `thread_id_for_agent_short_id()`
3. 添加单元测试

**PR 2: Issue 2 — Daemon 常驻化**（大，架构变更）
1. 新增 `shutdown_daemon` RPC method（trait 内短名）+ daemon 端实现
2. TUI 改用 `spawn_daemon_subprocess()` 启动 daemon
3. 引入 `QuitMode`，更新 Ctrl+Q/Ctrl+C 语义
4. 添加 PID 文件 + 崩溃恢复
5. 更新所有受影响测试
