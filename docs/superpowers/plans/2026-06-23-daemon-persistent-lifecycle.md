# Daemon 常驻化 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the Minos daemon a persistent service that survives TUI restart. Ctrl+Q exits the TUI but leaves the daemon running. Ctrl+C triggers full shutdown (TUI + daemon). Users experience continuous agent state across TUI restarts.

**Architecture:** The TUI spawns the daemon as a detached child process (`setsid`) instead of running it in-process. The daemon's lifecycle is decoupled from the TUI: it survives TUI exit and is only stopped via Ctrl+C (RPC shutdown), system signals, or manual `kill <pid>`. A PID file enables crash recovery.

**Tech Stack:** Rust, tokio, jsonrpsee, std::process::Command, Unix `setsid`

---

## File Structure

| File | Responsibility | Change |
|------|---------------|--------|
| `crates/minos-protocol/src/local_rpc.rs` | Local RPC trait | Add `shutdown_daemon` method |
| `crates/minos-daemon/src/local_rpc.rs` | RPC server impl | Implement `shutdown_daemon` |
| `crates/minos-daemon/src/handle.rs` | Daemon handle | Write/remove PID file in start/stop |
| `crates/minos-tui/src/main.rs` | TUI entry point | Spawn daemon as subprocess, quit mode handling |
| `crates/minos-tui/src/app/lifecycle.rs` | App lifecycle | `QuitMode` enum, `quit_mode` field |
| `crates/minos-tui/src/app/event_loop.rs` | Event handling | Set quit mode on Ctrl+Q vs Ctrl+C |
| `crates/minos-tui/src/effect.rs` | Effect enum | (No change needed — `Quit` and `InterruptOrQuit` already exist) |

---

## Task 1: Add `shutdown_daemon` RPC method

**Files:**
- Modify: `crates/minos-protocol/src/local_rpc.rs:92-214` (trait definition)
- Modify: `crates/minos-daemon/src/local_rpc.rs` (impl)

- [ ] **Step 1: Add `shutdown_daemon` to the RPC trait**

File: `crates/minos-protocol/src/local_rpc.rs`

Add to the `LocalDaemonRpc` trait (after the `subscribe_manager_events` subscription):

```rust
    #[method(name = "shutdown_daemon")]
    async fn shutdown_daemon(&self) -> jsonrpsee::core::RpcResult<()>;
```

- [ ] **Step 2: Implement `shutdown_daemon` in `LocalRpcImpl`**

File: `crates/minos-daemon/src/local_rpc.rs`

The impl needs access to a shutdown trigger. Add a `shutdown_tx: Option<tokio::sync::oneshot::Sender<()>>` field to `LocalRpcImpl`. The daemon's `start` command awaits this signal alongside SIGINT/SIGTERM.

First, update `LocalRpcImpl` struct:

```rust
pub struct LocalRpcImpl {
    pub started_at: Instant,
    pub runner: Arc<dyn CommandRunner>,
    pub agent: Arc<AgentGlue>,
    pub ingest_broadcaster: broadcast::Sender<LocalIngestFrame>,
    pub manager_event_broadcaster: broadcast::Sender<LocalManagerEvent>,
    pub shutdown_tx: std::sync::Mutex<Option<tokio::sync::oneshot::Sender<()>>>,
}
```

Implement the method:

```rust
async fn shutdown_daemon(&self) -> jsonrpsee::core::RpcResult<()> {
    let tx = self
        .shutdown_tx
        .lock()
        .expect("shutdown_tx lock poisoned")
        .take();
    if let Some(tx) = tx {
        let _ = tx.send(());
    }
    Ok(())
}
```

- [ ] **Step 3: Wire `shutdown_tx` through `start_local_rpc_server`**

File: `crates/minos-daemon/src/local_rpc.rs`

Update `start_local_rpc_server` signature to accept a shutdown sender and pass it to `LocalRpcImpl`:

```rust
pub async fn start_local_rpc_server(
    config: LocalRpcConfig,
    runner: Arc<dyn CommandRunner>,
    agent: Arc<AgentGlue>,
    shutdown_tx: tokio::sync::oneshot::Sender<()>,
) -> Result<(jsonrpsee::server::ServerHandle), MinosError> {
    // ...existing code...
    let impl_ = LocalRpcImpl {
        started_at: Instant::now(),
        runner,
        agent: agent.clone(),
        ingest_broadcaster: ingest_tx.clone(),
        manager_event_broadcaster: mgr_evt_tx.clone(),
        shutdown_tx: std::sync::Mutex::new(Some(shutdown_tx)),
    };
    // ...rest unchanged...
}
```

- [ ] **Step 4: Update `DaemonHandle::start_with_local_rpc` to create and store shutdown channel**

File: `crates/minos-daemon/src/handle.rs`

In `start_with_local_rpc` (around line 219-227), create the oneshot channel and pass the sender to `start_local_rpc_server`. Store the receiver on `DaemonInner`:

```rust
// In DaemonInner struct, add:
shutdown_rx: tokio::sync::Mutex<Option<tokio::sync::oneshot::Receiver<()>>>,

// In start_with_local_rpc:
let local_rpc_handle = if let Some(lr_config) = local_rpc_config {
    let runner = Arc::new(minos_cli_detect::RealCommandRunner::new(
        subprocess_env.clone(),
    ));
    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel();
    let handle = start_local_rpc_server(lr_config, runner, agent.clone(), shutdown_tx).await?;
    // Store shutdown_rx for the start command to await
    inner_shutdown_rx = Some(shutdown_rx);
    Some(handle)
} else {
    None
};
```

Expose a method on `DaemonHandle` to take the shutdown receiver:

```rust
pub async fn take_shutdown_signal(&self) -> Option<tokio::sync::oneshot::Receiver<()>> {
    self.inner.shutdown_rx.lock().await.take()
}
```

- [ ] **Step 5: Update `minos-daemon start` to await the shutdown signal**

File: `crates/minos-daemon/src/main.rs` — the `start` function (around line 421-428)

Currently:
```rust
wait_for_termination().await?;
```

Change to also await the RPC shutdown signal:

```rust
let shutdown_rx = handle.take_shutdown_signal().await;
tokio::select! {
    _ = wait_for_termination_signal() => {},
    _ = async {
        if let Some(rx) = shutdown_rx { rx.await.ok() } else { std::future::pending::<()>().await }
    } => {},
}
```

Where `wait_for_termination_signal()` is the renamed body of the existing `wait_for_termination()` (just the signal listening part, without calling `handle.stop()`).

- [ ] **Step 6: Compile and verify**

Run: `cargo check -p minos-protocol -p minos-daemon`
Expected: clean compilation

- [ ] **Step 7: Run daemon tests**

Run: `cargo test -p minos-daemon --quiet`
Expected: all pass

- [ ] **Step 8: Commit**

```bash
git add -A
git commit -m "feat: add shutdown_daemon RPC method for remote graceful shutdown"
```

---

## Task 2: Add PID file management to daemon

**Files:**
- Modify: `crates/minos-daemon/src/handle.rs` (write PID on start, remove on stop)
- Modify: `crates/minos-daemon/src/local_rpc.rs` (PID file path from config)

- [ ] **Step 1: Add PID file writing in `start_local_rpc_server`**

File: `crates/minos-daemon/src/local_rpc.rs`

After `write_discovery_file()` (line 348), add PID file writing:

```rust
write_discovery_file(&config.discovery_path, local_addr);
write_pid_file(&config.discovery_path);
```

Add the helper function:

```rust
fn write_pid_file(discovery_path: &std::path::Path) {
    let pid_path = discovery_path.with_file_name("tui-daemon.pid");
    let pid = std::process::id();
    if let Err(e) = std::fs::write(&pid_path, pid.to_string()) {
        tracing::warn!(
            target: "minos_daemon::local_rpc",
            error = %e,
            path = %pid_path.display(),
            "failed to write PID file",
        );
    }
}

fn remove_pid_file(discovery_path: &std::path::Path) {
    let pid_path = discovery_path.with_file_name("tui-daemon.pid");
    if let Err(e) = std::fs::remove_file(&pid_path) {
        if e.kind() != std::io::ErrorKind::NotFound {
            tracing::warn!(
                target: "minos_daemon::local_rpc",
                error = %e,
                path = %pid_path.display(),
                "failed to remove PID file",
            );
        }
    }
}
```

- [ ] **Step 2: Remove PID file in `DaemonHandle::stop()`**

File: `crates/minos-daemon/src/handle.rs:343-354`

After the discovery file removal block, add PID file removal:

```rust
if let Some(path) = &self.inner.local_rpc_discovery_path {
    // existing discovery file removal...
    
    // Remove PID file
    let pid_path = path.with_file_name("tui-daemon.pid");
    if let Err(error) = std::fs::remove_file(&pid_path) {
        if error.kind() != std::io::ErrorKind::NotFound {
            tracing::warn!(
                target: "minos_daemon::handle",
                error = %error,
                path = %pid_path.display(),
                "failed to remove PID file",
            );
        }
    }
}
```

- [ ] **Step 3: Compile and test**

Run: `cargo check -p minos-daemon && cargo test -p minos-daemon --quiet`
Expected: clean compilation, all tests pass

- [ ] **Step 4: Commit**

```bash
git add -A
git commit -m "feat: write and clean up PID file for daemon crash recovery"
```

---

## Task 3: Add `QuitMode` to distinguish Ctrl+Q and Ctrl+C

**Files:**
- Modify: `crates/minos-tui/src/app/lifecycle.rs` (add `QuitMode` enum, `quit_mode` field)
- Modify: `crates/minos-tui/src/app/event_loop.rs:114-120` (set quit mode)
- Modify: `crates/minos-tui/src/app/event_loop.rs:591-619` (Ctrl+C fallback sets HardShutdown)

- [ ] **Step 1: Define `QuitMode` enum and add to `App`**

File: `crates/minos-tui/src/app/lifecycle.rs`

Add the enum near the top of the file:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuitMode {
    /// Ctrl+Q: TUI exits, daemon survives.
    Soft,
    /// Ctrl+C (no interrupt target): TUI exits + daemon shuts down.
    HardShutdown,
}
```

Add field to `App` struct:

```rust
pub struct App {
    // ...existing fields...
    quit_mode: Option<QuitMode>,
}
```

Add accessor methods:

```rust
pub fn quit_mode(&self) -> Option<QuitMode> {
    self.quit_mode
}

pub fn set_quit_mode(&mut self, mode: QuitMode) {
    self.quit_mode = Some(mode);
}
```

Initialize `quit_mode: None` in the constructor.

- [ ] **Step 2: Set `QuitMode::Soft` on `Effect::Quit`**

File: `crates/minos-tui/src/app/event_loop.rs:114-119`

```rust
Effect::Quit => {
    self.quit_mode = Some(QuitMode::Soft);
    self.should_quit = true;
    false
}
```

Add import: `use super::QuitMode;` (or `use crate::app::QuitMode;` depending on module structure).

- [ ] **Step 3: Set `QuitMode::HardShutdown` on Ctrl+C fallback**

File: `crates/minos-tui/src/app/event_loop.rs:617`

Change the last line of `handle_ctrl_c()`:

```rust
// Before: self.should_quit = true;
// After:
self.quit_mode = Some(QuitMode::HardShutdown);
self.should_quit = true;
false
```

- [ ] **Step 4: Update `shutdown()` signature to accept `QuitMode`**

File: `crates/minos-tui/src/app/lifecycle.rs:254-270`

The current `shutdown()` only closes embedded threads. Keep that logic, but the `QuitMode` will be consumed by `main.rs` (not by `shutdown()` itself). Leave `shutdown()` unchanged — it already does the right thing (no-op for daemon backend).

- [ ] **Step 5: Compile**

Run: `cargo check -p minos-tui`
Expected: clean compilation

- [ ] **Step 6: Run tests**

Run: `cargo test -p minos-tui --quiet`
Expected: all pass

- [ ] **Step 7: Commit**

```bash
git add -A
git commit -m "feat: add QuitMode to distinguish soft quit from hard shutdown"
```

---

## Task 4: Spawn daemon as detached subprocess

**Files:**
- Modify: `crates/minos-tui/src/main.rs:241-300` (replace `start_managed_daemon_for_tui` + update `connect_or_start_daemon_backend`)

- [ ] **Step 1: Write `spawn_daemon_subprocess()` function**

File: `crates/minos-tui/src/main.rs`

Replace `start_managed_daemon_for_tui()` with a function that spawns the daemon as a separate process:

```rust
fn spawn_daemon_subprocess(discovery_path: &std::path::Path) -> Result<()> {
    let mut cmd = std::process::Command::new("minos-daemon");
    cmd.args(["start", "--local-rpc"]);

    // Detach from TUI process group so daemon survives TUI exit
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        unsafe {
            cmd.pre_exec(|| {
                // Create new session, detach from controlling terminal
                if libc::setsid() < 0 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
    }

    cmd.stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());

    cmd.spawn().context("failed to spawn minos-daemon subprocess")?;

    tracing::info!(
        target: "minos_tui",
        "spawned minos-daemon as detached subprocess"
    );
    Ok(())
}
```

Add `use anyhow::Context;` if not already imported.

- [ ] **Step 2: Write `wait_for_discovery_file()` helper**

File: `crates/minos-tui/src/main.rs`

```rust
async fn wait_for_discovery_file(
    discovery_path: &std::path::Path,
    timeout: std::time::Duration,
) -> Result<String> {
    let start = std::time::Instant::now();
    loop {
        if start.elapsed() > timeout {
            anyhow::bail!(
                "daemon did not start within {:?} (discovery file not found at {})",
                timeout,
                discovery_path.display()
            );
        }
        match resolve_daemon_url(None) {
            Ok(url) => return Ok(url),
            Err(_) => tokio::time::sleep(std::time::Duration::from_millis(200)).await,
        }
    }
}
```

- [ ] **Step 3: Write `check_stale_daemon()` for crash recovery**

File: `crates/minos-tui/src/main.rs`

```rust
fn check_stale_daemon(discovery_path: &std::path::Path) {
    let pid_path = discovery_path.with_file_name("tui-daemon.pid");
    let Ok(pid_str) = std::fs::read_to_string(&pid_path) else {
        return;
    };
    let Ok(pid) = pid_str.trim().parse::<i32>() else {
        let _ = std::fs::remove_file(&pid_path);
        return;
    };

    // Check if process is alive
    #[cfg(unix)]
    {
        // kill(pid, 0) returns Ok if process exists, Err if not
        let alive = unsafe { libc::kill(pid, 0) } == 0;
        if !alive {
            tracing::warn!(
                target: "minos_tui",
                pid,
                "stale daemon PID file detected (process not running); cleaning up"
            );
            let _ = std::fs::remove_file(&pid_path);
            let _ = std::fs::remove_file(discovery_path);
        }
    }
}
```

- [ ] **Step 4: Rewrite `connect_or_start_daemon_backend()`**

File: `crates/minos-tui/src/main.rs:267-300`

Change the return type — no more `Option<Arc<DaemonHandle>>`:

```rust
async fn connect_or_start_daemon_backend(
    override_url: Option<String>,
) -> Result<Arc<dyn crate::backend::AgentBackend>> {
    let explicit_url = override_url.is_some();
    let discovery_path = resolve_daemon_discovery_path()?;

    // Check for stale daemon (crashed without cleaning up)
    check_stale_daemon(&discovery_path);

    match resolve_daemon_url(override_url.clone()) {
        Ok(url) => match crate::backend::DaemonBackend::connect(&url).await {
            Ok(backend) => return Ok(Arc::new(backend)),
            Err(error) if explicit_url => return Err(error),
            Err(error) => {
                tracing::warn!(
                    target: "minos_tui",
                    error = %error,
                    "failed to connect to discovered daemon; spawning new daemon"
                );
            }
        },
        Err(error) if explicit_url => return Err(error),
        Err(error) => {
            tracing::warn!(
                target: "minos_tui",
                error = %error,
                "daemon discovery unavailable; spawning new daemon"
            );
        }
    }

    // Spawn daemon as detached subprocess
    spawn_daemon_subprocess(&discovery_path)?;
    let url = wait_for_discovery_file(&discovery_path, std::time::Duration::from_secs(15)).await?;
    let backend = crate::backend::DaemonBackend::connect(&url).await?;
    Ok(Arc::new(backend))
}
```

- [ ] **Step 5: Update `main()` to use new return type**

File: `crates/minos-tui/src/main.rs:320-428`

Remove the `managed_daemon` variable entirely. The daemon backend section becomes:

```rust
let backend: Arc<dyn crate::backend::AgentBackend> = match cli.backend {
    BackendKind::Embedded => Arc::new(
        crate::backend::EmbeddedBackend::new(
            workspace.clone(),
            max_instances,
            std::time::Duration::from_secs(300),
            mcp_permissions,
        )
        .await?,
    ),
    BackendKind::Daemon => connect_or_start_daemon_backend(cli.daemon_url.clone()).await?,
};
```

The shutdown section at the end becomes:

```rust
app.shutdown().await;
restore_terminal(&mut terminal)?;

// Handle quit mode for daemon backend
if matches!(app.quit_mode(), Some(QuitMode::HardShutdown)) {
    if let crate::backend::BackendConnectionState::Connected { endpoint } =
        backend.connection_state()
    {
        tracing::info!(
            target: "minos_tui",
            endpoint = %endpoint,
            "hard shutdown requested; sending shutdown to daemon"
        );
        // Best-effort: send shutdown RPC. Daemon may already be gone.
        let _ = send_daemon_shutdown(&endpoint).await;
    }
}

Ok(())
```

- [ ] **Step 6: Write `send_daemon_shutdown()` helper**

File: `crates/minos-tui/src/main.rs`

```rust
async fn send_daemon_shutdown(endpoint: &str) -> Result<()> {
    use jsonrpsee::ws_client::WsClientBuilder;

    let client = WsClientBuilder::default()
        .build(endpoint)
        .await
        .context("failed to connect to daemon for shutdown")?;
    client
        .request::<(), _>("minos_local_shutdown_daemon", jsonrpsee::core::params::ArrayParams::new())
        .await
        .context("shutdown_daemon RPC failed")?;
    tracing::info!(target: "minos_tui", "daemon shutdown acknowledged");
    Ok(())
}
```

- [ ] **Step 7: Add `libc` dependency if not present**

File: `crates/minos-tui/Cargo.toml`

Check if `libc` is already a dependency. If not, add:

```toml
[target.'cfg(unix)'.dependencies]
libc = "0.2"
```

- [ ] **Step 8: Compile**

Run: `cargo check -p minos-tui`
Expected: clean compilation

- [ ] **Step 9: Commit**

```bash
git add -A
git commit -m "feat: spawn daemon as detached subprocess that survives TUI exit"
```

---

## Task 5: Update tests for new daemon lifecycle

**Files:**
- Modify: `crates/minos-tui/src/app_tests/navigation_and_lifecycle.rs` (update shutdown test)
- Modify: `crates/minos-tui/src/main.rs` (update `#[cfg(test)]` tests)

- [ ] **Step 1: Update `shutdown_does_not_close_threads_for_daemon_backend` test**

File: `crates/minos-tui/src/app_tests/navigation_and_lifecycle.rs`

This test previously verified that daemon-mode shutdown doesn't close threads. It should still pass since `shutdown()` is unchanged. Verify it still compiles and passes.

Run: `cargo test -p minos-tui shutdown_does_not_close -- --nocapture`
Expected: PASS

- [ ] **Step 2: Add test for `QuitMode` propagation**

Add a test verifying that `Effect::Quit` sets `QuitMode::Soft` and Ctrl+C fallback sets `QuitMode::HardShutdown`:

```rust
#[tokio::test]
async fn quit_effect_sets_soft_mode() {
    let (mut app, _backend) = App::test_app(TestBackend::default()).await;
    app.execute_effect(Effect::Quit).await;
    assert!(app.should_quit());
    assert_eq!(app.quit_mode(), Some(QuitMode::Soft));
}
```

- [ ] **Step 3: Update main.rs tests that reference `managed_daemon`**

The `test_cli` and `validate_backend_args` tests in `main.rs` don't reference `managed_daemon`, but verify no other test does.

Search: `grep -r "managed_daemon" crates/minos-tui/src/`
Fix any references.

- [ ] **Step 4: Run full TUI test suite**

Run: `cargo test -p minos-tui --quiet`
Expected: all pass

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "test: update tests for daemon subprocess lifecycle and QuitMode"
```

---

## Task 6: Final verification

- [ ] **Step 1: Full workspace compile**

Run: `cargo check --workspace`
Expected: clean

- [ ] **Step 2: Run all relevant test suites**

Run: `cargo test -p minos-tui -p minos-daemon -p minos-protocol -p minos-agent-runtime --lib -j1 --quiet`
Expected: all pass

- [ ] **Step 3: Manual integration test**

1. Start TUI with `--backend daemon`
2. Start an agent session in a conversation
3. Press Ctrl+Q to quit TUI
4. Verify daemon is still running: `ps aux | grep minos-daemon`
5. Restart TUI
6. Verify the agent session still appears with its previous state
7. Press Ctrl+C to quit (hard shutdown)
8. Verify daemon was stopped: `ps aux | grep minos-daemon` (should be gone)

- [ ] **Step 4: Manual crash recovery test**

1. Start TUI with `--backend daemon`
2. Force-kill the daemon process: `kill -9 $(cat ~/.minos/run/tui-daemon.pid)`
3. Restart TUI
4. Verify TUI detects stale daemon, cleans up, and spawns a fresh one

- [ ] **Step 5: Final commit if any cleanup**

```bash
git add -A
git commit -m "chore: daemon lifecycle verification complete"
```
