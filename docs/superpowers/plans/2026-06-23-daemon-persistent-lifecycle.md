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

Add to the `LocalDaemonRpc` trait (after the `subscribe_manager_events` subscription). Note: the trait already has `#[rpc(server, client, namespace = "minos_local")]` at line 92, so jsonrpsee auto-prefixes every method with `minos_local_`. Declare the **short** name here; the wire name callers use will be `minos_local_shutdown_daemon`:

```rust
    // trait 内用短名；wire name 自动变成 minos_local_shutdown_daemon
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

The `start_with_local_rpc` function needs to create an oneshot channel, pass the sender to `start_local_rpc_server`, and store the receiver on `DaemonInner` so the `start` command can later await it.

**Step 4a: Update imports**

File: `crates/minos-daemon/src/handle.rs`, top of file.

The file currently imports (lines 10-26):
```rust
use std::sync::{Arc, Mutex as StdMutex};
...
use tokio::sync::watch;
```

Add `oneshot` and `Mutex as TokioMutex` to the existing `tokio::sync` import:

```rust
// Before (line 16):
use tokio::sync::watch;

// After:
use tokio::sync::{watch, Mutex as TokioMutex, oneshot};
```

`Arc` and `StdMutex` stay on the `std::sync` import (line 11) — do not remove them.

**Step 4b: Add the `shutdown_rx` field to `DaemonInner`**

File: `crates/minos-daemon/src/handle.rs:28-60`.

The existing `DaemonInner` is a private struct (no derives). Add the field at the end, after `local_rpc_discovery_path`:

```rust
struct DaemonInner {
    relay: Arc<RelayClient>,
    link_rx: watch::Receiver<minos_domain::RelayLinkState>,
    peer_rx: watch::Receiver<minos_domain::PeerState>,
    peer: Arc<StdMutex<Option<PeerRecord>>>,
    peers: Arc<StdMutex<Vec<HostPeerSummary>>>,
    #[allow(dead_code)]
    mac_name: String,
    last_error: Arc<StdMutex<Option<MinosError>>>,
    agent: Arc<AgentGlue>,
    rt_handle: Handle,
    #[allow(dead_code)]
    local_rpc_handle: Option<jsonrpsee::server::ServerHandle>,
    local_rpc_discovery_path: Option<PathBuf>,
    // NEW: shutdown signal receiver. `tokio::sync::Mutex` (not std) because
    // `take_shutdown_signal` awaits while holding the lock. `Option` because
    // the daemon may run without a local RPC server.
    shutdown_rx: TokioMutex<Option<oneshot::Receiver<()>>>,
}
```

> Why `tokio::sync::Mutex` here but `std::sync::Mutex` on `LocalRpcImpl.shutdown_tx` (Step 2)? `shutdown_daemon`'s RPC handler takes the tx under a sync lock with **no await** inside the guard — std Mutex is correct and cheaper there. `take_shutdown_signal` is `async` and the caller may await between locking and consuming — a std Mutex would be unsound across await points, so we use tokio Mutex here.

**Step 4c: Create the channel and construct `DaemonInner` with it**

File: `crates/minos-daemon/src/handle.rs` — `start_with_local_rpc`, lines 219-243.

The current code (lines 219-227) builds the local RPC handle and then constructs `DaemonInner` (lines 229-243). Declare a local `shutdown_rx` that starts as `None`, populate it only when `local_rpc_config` is `Some`, then move it into `DaemonInner`. Concretely, replace lines 219-243 with:

```rust
// Declare the receiver up-front; stays None when there's no local RPC.
let mut shutdown_rx: Option<oneshot::Receiver<()>> = None;

let local_rpc_handle = if let Some(lr_config) = local_rpc_config {
    let runner = Arc::new(minos_cli_detect::RealCommandRunner::new(
        subprocess_env.clone(),
    ));
    let (shutdown_tx, rx) = oneshot::channel();
    shutdown_rx = Some(rx);
    let handle = start_local_rpc_server(lr_config, runner, agent.clone(), shutdown_tx).await?;
    Some(handle)
} else {
    None
};

Ok(Arc::new(Self {
    inner: Arc::new(DaemonInner {
        relay,
        link_rx,
        peer_rx,
        peer: peer_store,
        peers: peers_store,
        mac_name,
        last_error,
        agent,
        rt_handle: Handle::current(),
        local_rpc_handle,
        local_rpc_discovery_path,
        // NEW: store the shutdown receiver for the `start` command to await.
        shutdown_rx: TokioMutex::new(shutdown_rx),
    }),
}))
```

> The local variable is named `shutdown_rx` (not `inner_shutdown_rx`). It is moved into `DaemonInner.shutdown_rx` at the construction site. There are no other readers of this local.

**Step 4d: Expose a method on `DaemonHandle` to take the shutdown receiver**

```rust
pub async fn take_shutdown_signal(&self) -> Option<oneshot::Receiver<()>> {
    self.inner.shutdown_rx.lock().await.take()
}
```

This matches the `shutdown_tx: std::sync::Mutex<Option<...>>` shape used on `LocalRpcImpl` (Step 2), but uses `tokio::sync::Mutex` here because the consumer (`take_shutdown_signal`) is async and awaiting inside a `std::sync::Mutex` guard across an `.await` point would be unsound.

- [ ] **Step 5: Update `minos-daemon start` to await the shutdown signal**

File: `crates/minos-daemon/src/main.rs` — the `start` function, lines 427-429.

Currently:
```rust
wait_for_termination().await?;
println!("status:     stopping");
handle.stop().await?;
Ok(())
```

`wait_for_termination()` (line 589) already only waits for SIGINT/SIGTERM — it does **not** call `handle.stop()`. We keep it as-is and add a `tokio::select!` arm for the RPC shutdown receiver so either trigger wins:

```rust
let shutdown_rx = handle.take_shutdown_signal().await;
tokio::select! {
    res = wait_for_termination() => {
        res?;
    }
    _ = async {
        match shutdown_rx {
            Some(rx) => { rx.await.ok(); }
            None => std::future::pending::<()>().await,
        }
    } => {}
}
println!("status:     stopping");
handle.stop().await?;
Ok(())
```

> `handle.stop()` is still called unconditionally after the select resolves, preserving the existing cleanup path for both signal and RPC shutdown.

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

The current `shutdown()` only closes embedded sessions. Keep that logic, but the `QuitMode` will be consumed by `main.rs` (not by `shutdown()` itself). Leave `shutdown()` unchanged — it already does the right thing (no-op for daemon backend).

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

- [ ] **Step 2: Write `wait_for_new_discovery()` helper**

File: `crates/minos-tui/src/main.rs`

**Stale-discovery hazard:** if a previous daemon crashed, the old discovery file may still be on disk. A naive poll that returns as soon as `resolve_daemon_url()` succeeds would read back the *old* URL and never wait for the freshly-spawned daemon to write its own. To avoid this:

1. The caller (`connect_or_start_daemon_backend`) **deletes** the stale discovery file before spawning (see Step 4).
2. This helper records the pre-spawn mtime of the discovery path (or `None` if absent) and only returns once a discovery file appears **with an mtime newer than** the snapshot — i.e. written by the new daemon.
3. The returned URL is not trusted until the caller connects successfully.

```rust
/// Wait for the *new* daemon to publish its discovery file.
///
/// `pre_spawn_mtime` is the mtime captured *before* `spawn_daemon_subprocess`
/// (or `None` if the file did not exist). We only accept a discovery file
/// whose mtime is strictly newer, so we never read back a stale URL left by
/// a crashed daemon.
async fn wait_for_new_discovery(
    discovery_path: &std::path::Path,
    pre_spawn_mtime: Option<std::time::SystemTime>,
    timeout: std::time::Duration,
) -> Result<String> {
    let start = std::time::Instant::now();
    loop {
        if start.elapsed() > timeout {
            anyhow::bail!(
                "daemon did not start within {:?} (no fresh discovery file at {})",
                timeout,
                discovery_path.display()
            );
        }
        // Only trust the file if it is newer than what we saw before spawn.
        let fresh = match std::fs::metadata(discovery_path) {
            Ok(meta) => match meta.modified() {
                Ok(mtime) => pre_spawn_mtime.map_or(true, |prev| mtime > prev),
                Err(_) => false,
            },
            Err(_) => false,
        };
        if fresh {
            if let Ok(url) = resolve_daemon_url(None) {
                return Ok(url);
            }
        }
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
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

Change the return type — no more `Option<Arc<DaemonHandle>>`.

**Stale-discovery prevention:** before spawning, (a) run `check_stale_daemon` (PID-based), then (b) snapshot the current discovery-file mtime and (c) delete the stale discovery file so the new daemon starts from a clean slate. After spawn, `wait_for_new_discovery` waits for a file newer than the snapshot, and **the daemon is only considered started once `DaemonBackend::connect` succeeds**:

```rust
async fn connect_or_start_daemon_backend(
    override_url: Option<String>,
) -> Result<Arc<dyn crate::backend::AgentBackend>> {
    let explicit_url = override_url.is_some();
    let discovery_path = resolve_daemon_discovery_path()?;

    // Phase 1: try to connect to an existing daemon (no spawn).
    match resolve_daemon_url(override_url.clone()) {
        Ok(url) => match crate::backend::DaemonBackend::connect(&url).await {
            Ok(backend) => return Ok(Arc::new(backend)),
            Err(error) if explicit_url => return Err(error),
            Err(error) => {
                tracing::warn!(
                    target: "minos_tui",
                    error = %error,
                    "failed to connect to discovered daemon; will spawn a new one"
                );
            }
        },
        Err(error) if explicit_url => return Err(error),
        Err(error) => {
            tracing::warn!(
                target: "minos_tui",
                error = %error,
                "daemon discovery unavailable; will spawn a new daemon"
            );
        }
    }

    // Phase 2: spawn a fresh daemon. Clean up stale state first.
    check_stale_daemon(&discovery_path);

    // Snapshot the pre-spawn mtime so wait_for_new_discovery can reject a
    // stale discovery file left behind by the dead daemon.
    let pre_spawn_mtime = std::fs::metadata(&discovery_path)
        .ok()
        .and_then(|m| m.modified().ok());

    // Delete any stale discovery file so we never read back the old URL.
    if pre_spawn_mtime.is_some() {
        let _ = std::fs::remove_file(&discovery_path);
    }

    spawn_daemon_subprocess(&discovery_path)?;

    // Wait for the NEW daemon's discovery file (mtime strictly newer than
    // pre_spawn_mtime), then connect. A successful connect is the only
    // signal that the daemon is actually up.
    let url = wait_for_new_discovery(
        &discovery_path,
        pre_spawn_mtime,
        std::time::Duration::from_secs(15),
    )
    .await?;
    let backend = crate::backend::DaemonBackend::connect(&url).await?;
    Ok(Arc::new(backend))
}
```

> **Why the connect-after-wait matters:** `wait_for_new_discovery` returning only proves a discovery file was written. If the daemon dies between writing discovery and accepting connections, the subsequent `DaemonBackend::connect` will fail and surface a real error instead of silently succeeding with a dead endpoint.

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

- [ ] **Step 1: Update `shutdown_does_not_close_sessions_for_daemon_backend` test**

File: `crates/minos-tui/src/app_tests/navigation_and_lifecycle.rs`

This test previously verified that daemon-mode shutdown doesn't close sessions. It should still pass since `shutdown()` is unchanged. Verify it still compiles and passes.

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

- [ ] **Step 4: Add unit test for stale-discovery protection in `wait_for_new_discovery`**

File: `crates/minos-tui/src/main.rs` (test module)

Verify that `wait_for_new_discovery` does **not** accept a discovery file whose mtime is older than `pre_spawn_mtime` (simulating a stale file left by a crashed daemon), and only returns once a file newer than the snapshot appears. Use a temp dir:

```rust
#[tokio::test]
async fn wait_for_new_discovery_rejects_stale_file() {
    let dir = tempfile::tempdir().unwrap();
    let discovery_path = dir.path().join("tui-daemon-rpc.json");

    // Simulate a stale discovery file written by a crashed daemon.
    std::fs::write(&discovery_path, r#"{"url":"ws://127.0.0.1:9999"}"#).unwrap();
    let stale_mtime = std::fs::metadata(&discovery_path).unwrap().modified().unwrap();

    // pre_spawn_mtime == stale_mtime: must NOT immediately return the old URL.
    // Spawn a background task that writes a NEW file after a short delay.
    let path_clone = discovery_path.clone();
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        // Bump mtime by writing new content (OS will set a newer mtime).
        std::fs::write(&path_clone, r#"{"url":"ws://127.0.0.1:12345"}"#).unwrap();
    });

    // Use a short timeout; if it read the stale file it would return instantly
    // with the old URL.
    let url = wait_for_new_discovery(
        &discovery_path,
        Some(stale_mtime),
        std::time::Duration::from_secs(5),
    )
    .await
    .expect("should detect the new discovery file");

    assert!(
        url.contains("12345"),
        "should read the NEW daemon's URL, not the stale one"
    );
}
```

> Note: `tempfile` must be a dev-dependency of `minos-tui`. Check `crates/minos-tui/Cargo.toml` — if missing, add it under `[dev-dependencies]`.

- [ ] **Step 5: Run full TUI test suite**

Run: `cargo test -p minos-tui --quiet`
Expected: all pass

- [ ] **Step 6: Commit**

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
