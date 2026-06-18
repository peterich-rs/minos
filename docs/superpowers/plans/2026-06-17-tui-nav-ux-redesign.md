# minos-tui 三级导航与交互体验重设计 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 用 Project → Thread → Agent 三级导航取代当前三栏平铺布局，每级统一 80%/20% 布局，支持 cwd 自动匹配 project 直接进入 Threads。

**Architecture:** 扩展 `AgentBackend` trait 加 project 方法；引入 `NavLevel` 导航栈驱动渲染分发；Thread=Room 合并；新增 Projects/Threads 列表视图和 Agent 卡片侧栏。基于当前代码库状态编写（三阶段架构重构尚未落地）。

**Tech Stack:** Rust 2021, Ratatui 0.29, Crossterm 0.28, async-trait, jsonrpsee (daemon), tokio.

**Spec:** `docs/superpowers/specs/2026-06-17-tui-nav-ux-redesign.md`

**Test command:** `cargo test -p minos-tui`
**Build command:** `cargo build -p minos-tui`
**Lint command:** `cargo clippy -p minos-tui -- -D warnings`

---

## File Structure

| File | Responsibility | Status |
|---|---|---|
| `src/nav.rs` | `NavLevel` 枚举、`NavStack` 封装 push/pop/peek | new |
| `src/backend/mod.rs` | `AgentBackend` trait 加 project 方法 + `ProjectEntry`/`ThreadSummaryEntry` 类型 | modify |
| `src/backend/daemon.rs` | 实现 project trait 方法，转发 `minos_*` RPC | modify |
| `src/backend/embedded.rs` | 实现 project trait 方法，内存 project | modify |
| `src/app.rs` | `UiState` 加 `nav_stack`；启动 cwd 匹配；`handle_event` 按 NavLevel 分发 | modify |
| `src/ui/mod.rs` | `render_ui` 按 nav level 渲染；统一布局函数 | modify |
| `src/ui/project_list.rs` | Projects 列表渲染 + 侧栏 | new |
| `src/ui/thread_list_v2.rs` | Threads 列表渲染 + 侧栏（取代旧 thread_list.rs 的角色） | new |
| `src/ui/project_create_dialog.rs` | Project 创建对话框 | new |
| `src/ui/agent_card.rs` | Agent 卡片 widget（Thread 视图侧栏） | new |
| `src/ui/theme.rs` | 扩展语义色 | modify |
| `src/main.rs` | 启动序列加 cwd → project 匹配 | modify |

**测试模式:** 内联 `#[cfg(test)] mod tests`，`#[tokio::test]` for async，手写 `TestBackend` fake（不用 mockall）。`cargo test -p minos-tui`。

**与三阶段重构的关系:** 本计划基于当前代码库（未重构）。如果三阶段重构先落地，`handle_event` → `event_to_actions` 的迁移由重构 spec 负责；本计划的 NavLevel push/pop 逻辑届时作为 Action variant 处理。两个 spec 不冲突。

---

## Phase 1: AgentBackend trait 扩展 + 类型定义

### Task 1.1: 定义 ProjectEntry 和扩展后的 ThreadEntry 类型

**Files:**
- Modify: `crates/minos-tui/src/ui/mod.rs` (ThreadEntry 扩展)
- Create: `crates/minos-tui/src/backend/mod.rs` (加 ProjectEntry)

- [ ] **Step 1: 在 `backend/mod.rs` 末尾（line 106 之后，`pub mod daemon` 之前）加 `ProjectEntry` 类型**

```rust
/// TUI-level project entry, mapped from `minos_protocol::ProjectSummary`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectEntry {
    pub project_id: String,
    pub name: String,
    pub workspace_path: PathBuf,
    pub thread_count: u32,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

impl ProjectEntry {
    /// Build from a protocol `ProjectSummary`, defaulting workspace_path to cwd if absent.
    pub fn from_summary(s: &minos_protocol::ProjectSummary, fallback_cwd: &Path) -> Self {
        let workspace_path = s
            .workspace_path
            .as_ref()
            .map(|p| PathBuf::from(p))
            .unwrap_or_else(|| fallback_cwd.to_path_buf());
        Self {
            project_id: s.project_id.clone(),
            name: s.name.clone(),
            workspace_path,
            thread_count: s.thread_count,
            created_at_ms: s.created_at_ms,
            updated_at_ms: s.updated_at_ms,
        }
    }
}
```

- [ ] **Step 2: 在 `backend/mod.rs` 的 trait 定义中（line 40-106 的 `#[async_trait] pub trait AgentBackend`），在 `list_threads` 之后、`resume_thread` 之前加 project 方法**

```rust
    async fn list_threads(&self) -> Result<Vec<BackendThreadSnapshot>>;

    async fn list_projects(&self) -> Result<Vec<ProjectEntry>>;

    async fn create_project(&self, name: &str, workspace_path: &Path) -> Result<ProjectEntry>;

    async fn list_project_threads(&self, project_id: &str) -> Result<Vec<ThreadSummaryEntry>>;

    async fn start_agent_in_project(
        &self,
        project_id: &str,
        agent: AgentName,
        workspace: PathBuf,
        prompt: Option<&str>,
    ) -> Result<StartAgentOutcome>;
```

Also add `use std::path::Path;` to the imports at the top of the file (currently only `PathBuf` is imported on line 8 — add `Path`).

And add the `ThreadSummaryEntry` type after `BackendThreadSnapshot` (after line 38):

```rust
/// TUI-level thread summary for list views, mapped from `minos_protocol::ThreadSummary`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThreadSummaryEntry {
    pub thread_id: String,
    pub agent: AgentName,
    pub title: Option<String>,
    pub first_ts_ms: i64,
    pub last_ts_ms: i64,
    pub message_count: u32,
    pub ended_at_ms: Option<i64>,
}

impl ThreadSummaryEntry {
    pub fn from_summary(s: &minos_protocol::ThreadSummary) -> Self {
        Self {
            thread_id: s.thread_id.clone(),
            agent: s.agent,
            title: s.title.clone(),
            first_ts_ms: s.first_ts_ms,
            last_ts_ms: s.last_ts_ms,
            message_count: s.message_count,
            ended_at_ms: s.ended_at_ms,
        }
    }
}
```

- [ ] **Step 3: Verify the code compiles (trait not yet implemented by DaemonBackend/EmbeddedBackend/TestBackend — expect compile errors there)**

Run: `cargo build -p minos-tui 2>&1 | head -30`
Expected: errors about missing trait methods in `DaemonBackend`, `EmbeddedBackend`, `TestBackend`. This is expected — we implement them in the next tasks.

- [ ] **Step 4: Commit the type definitions**

```bash
git add crates/minos-tui/src/backend/mod.rs
git commit -m "feat(tui): add ProjectEntry, ThreadSummaryEntry, and project methods to AgentBackend trait"
```

### Task 1.2: 实现 DaemonBackend 的 project 方法

**Files:**
- Modify: `crates/minos-tui/src/backend/daemon.rs`

- [ ] **Step 1: 在 `DaemonBackend` 的 `AgentBackend` impl 块中（line 176 之后），在 `list_threads` 方法之后加入 project 方法**

Add these imports at the top of `daemon.rs` (after the existing `use` statements near line 24):

```rust
use crate::backend::{ProjectEntry, ThreadSummaryEntry};
use std::path::Path;
```

Add the four trait methods after the `list_threads` method (which ends around line 319):

```rust
    async fn list_projects(&self) -> Result<Vec<ProjectEntry>> {
        let response: minos_protocol::ListProjectsResponse = self
            .client
            .request("minos_list_projects", ArrayParams::new())
            .await
            .context("RPC minos_list_projects failed")?;
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        Ok(response
            .projects
            .iter()
            .map(|p| ProjectEntry::from_summary(p, &cwd))
            .collect())
    }

    async fn create_project(&self, name: &str, workspace_path: &Path) -> Result<ProjectEntry> {
        let workspace_str = workspace_path.to_string_lossy().into_owned();
        let slug = workspace_path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| name.to_lowercase().replace(' ', "-"));
        let request = minos_protocol::CreateProjectRequest {
            name: name.to_owned(),
            workspace_slug: slug,
            workspace_path: Some(workspace_str),
        };
        let response: minos_protocol::CreateProjectResponse = self
            .client
            .request("minos_create_project", [request])
            .await
            .context("RPC minos_create_project failed")?;
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        Ok(ProjectEntry::from_summary(&response.project, &cwd))
    }

    async fn list_project_threads(&self, project_id: &str) -> Result<Vec<ThreadSummaryEntry>> {
        let params = minos_protocol::ListProjectThreadsParams {
            project_id: project_id.to_owned(),
            limit: 100,
            before_ts_ms: None,
        };
        let response: minos_protocol::ListProjectThreadsResponse = self
            .client
            .request("minos_list_project_threads", [params])
            .await
            .context("RPC minos_list_project_threads failed")?;
        Ok(response
            .threads
            .iter()
            .map(ThreadSummaryEntry::from_summary)
            .collect())
    }

    async fn start_agent_in_project(
        &self,
        project_id: &str,
        agent: AgentName,
        workspace: PathBuf,
        prompt: Option<&str>,
    ) -> Result<StartAgentOutcome> {
        #[derive(serde::Serialize)]
        struct StartInProjectParams {
            agent: AgentName,
            workspace: String,
            project_id: String,
        }
        let params = StartInProjectParams {
            agent,
            workspace: workspace.to_string_lossy().into_owned(),
            project_id: project_id.to_owned(),
        };
        let response: StartAgentResponse = self
            .client
            .request("minos_start_agent_in_project", [params])
            .await
            .context("RPC minos_start_agent_in_project failed")?;
        // If a prompt was provided, send it as the first message.
        if let Some(text) = prompt {
            let msg_req = SendUserMessageRequest {
                session_id: response.session_id.clone(),
                text: text.to_owned(),
            };
            self.client
                .request::<(), _>("minos_local_send_user_message", [msg_req])
                .await
                .context("RPC minos_local_send_user_message after project start failed")?;
        }
        Ok(StartAgentOutcome {
            thread_id: response.session_id,
            cwd: PathBuf::from(response.cwd),
            provider_session_id: None,
        })
    }
```

Note: `StartAgentResponse` and `SendUserMessageRequest` are already imported in `daemon.rs` (used by existing `start_agent` and `send_message` methods). Verify by checking the imports — they come from `minos_protocol`.

- [ ] **Step 2: Verify DaemonBackend compiles**

Run: `cargo build -p minos-tui 2>&1 | grep -E '^(error|warning)' | head -20`
Expected: errors only in `embedded.rs` and `app.rs` tests (TestBackend), not in `daemon.rs`.

- [ ] **Step 3: Commit**

```bash
git add crates/minos-tui/src/backend/daemon.rs
git commit -m "feat(tui): implement project RPC methods in DaemonBackend"
```

### Task 1.3: 实现 EmbeddedBackend 的 project 方法

**Files:**
- Modify: `crates/minos-tui/src/backend/embedded.rs`

- [ ] **Step 1: 在 `EmbeddedBackend` struct 中加内存 project 存储**

Read the current `EmbeddedBackend` struct definition. Add a field for in-memory projects:

```rust
use crate::backend::{ProjectEntry, ThreadSummaryEntry};
use std::path::Path;
use std::sync::Mutex as StdMutex;
```

In the `EmbeddedBackend` struct, add:
```rust
    projects: StdMutex<Vec<ProjectEntry>>,
```

In `EmbeddedBackend::new()` (or wherever it's constructed), initialize:
```rust
    projects: StdMutex::new(Vec::new()),
```

- [ ] **Step 2: 实现 trait 方法 — 在 `AgentBackend` impl 块中，`list_threads` 之后加**

```rust
    async fn list_projects(&self) -> Result<Vec<ProjectEntry>> {
        let projects = self.projects.lock().expect("projects lock").clone();
        if projects.is_empty() {
            // Auto-generate a single project from cwd for backward compat.
            let cwd = self.workspace.clone();
            let name = cwd
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| "workspace".to_owned());
            Ok(vec![ProjectEntry {
                project_id: format!("embedded-{}", name),
                name,
                workspace_path: cwd,
                thread_count: 0,
                created_at_ms: 0,
                updated_at_ms: 0,
            }])
        } else {
            Ok(projects)
        }
    }

    async fn create_project(&self, name: &str, workspace_path: &Path) -> Result<ProjectEntry> {
        let entry = ProjectEntry {
            project_id: format!("embedded-{}-{}", name, workspace_path.to_string_lossy()),
            name: name.to_owned(),
            workspace_path: workspace_path.to_path_buf(),
            thread_count: 0,
            created_at_ms: chrono_like_now_ms(),
            updated_at_ms: chrono_like_now_ms(),
        };
        self.projects
            .lock()
            .expect("projects lock")
            .push(entry.clone());
        Ok(entry)
    }

    async fn list_project_threads(&self, _project_id: &str) -> Result<Vec<ThreadSummaryEntry>> {
        // Embedded mode: return existing threads as flat list, no project scoping.
        let snapshots = self.list_threads().await?;
        Ok(snapshots
            .into_iter()
            .map(|s| ThreadSummaryEntry {
                thread_id: s.thread_id,
                agent: s.agent.unwrap_or(AgentName::Codex),
                title: None,
                first_ts_ms: 0,
                last_ts_ms: 0,
                message_count: 0,
                ended_at_ms: None,
            })
            .collect())
    }

    async fn start_agent_in_project(
        &self,
        _project_id: &str,
        agent: AgentName,
        workspace: PathBuf,
        prompt: Option<&str>,
    ) -> Result<StartAgentOutcome> {
        let outcome = self.start_agent(agent, workspace).await?;
        if let Some(text) = prompt {
            self.send_message(&outcome.thread_id, text).await?;
        }
        Ok(outcome)
    }
```

Add the helper function (near the bottom of the file, outside the impl):
```rust
fn chrono_like_now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}
```

- [ ] **Step 3: Verify EmbeddedBackend compiles**

Run: `cargo build -p minos-tui 2>&1 | grep -E '^error' | head -20`
Expected: errors only in `app.rs` test module (TestBackend missing trait methods).

- [ ] **Step 4: Commit**

```bash
git add crates/minos-tui/src/backend/embedded.rs
git commit -m "feat(tui): implement project methods in EmbeddedBackend with in-memory storage"
```

### Task 1.4: 更新 TestBackend 实现 project 方法

**Files:**
- Modify: `crates/minos-tui/src/app.rs` (TestBackend in `#[cfg(test)] mod tests`)

- [ ] **Step 1: 在 TestBackend struct 中加 project 相关字段（在 `connection_state` 字段之后）**

```rust
    projects: Mutex<Vec<crate::backend::ProjectEntry>>,
    created_projects: Mutex<Vec<(String, PathBuf)>>,
    project_thread_lists: Mutex<Vec<(String, Vec<crate::backend::ThreadSummaryEntry>)>>,
```

- [ ] **Step 2: 在 `TestBackend::with_agents` (the `new()` constructor) 中初始化这些字段**

```rust
    projects: Mutex::new(Vec::new()),
    created_projects: Mutex::new(Vec::new()),
    project_thread_lists: Mutex::new(Vec::new()),
```

- [ ] **Step 3: 在 `TestBackend` 的 `AgentBackend` impl 中，`list_threads` 之后加 trait 方法**

```rust
        async fn list_projects(&self) -> Result<Vec<crate::backend::ProjectEntry>> {
            Ok(self.projects.lock().expect("projects lock").clone())
        }

        async fn create_project(
            &self,
            name: &str,
            workspace_path: &std::path::Path,
        ) -> Result<crate::backend::ProjectEntry> {
            self.created_projects
                .lock()
                .expect("created projects lock")
                .push((name.to_owned(), workspace_path.to_path_buf()));
            let entry = crate::backend::ProjectEntry {
                project_id: format!("test-project-{}", name),
                name: name.to_owned(),
                workspace_path: workspace_path.to_path_buf(),
                thread_count: 0,
                created_at_ms: 0,
                updated_at_ms: 0,
            };
            self.projects
                .lock()
                .expect("projects lock")
                .push(entry.clone());
            Ok(entry)
        }

        async fn list_project_threads(
            &self,
            project_id: &str,
        ) -> Result<Vec<crate::backend::ThreadSummaryEntry>> {
            let lists = self.project_thread_lists.lock().expect("project threads lock");
            Ok(lists
                .iter()
                .find(|(pid, _)| pid == project_id)
                .map(|(_, threads)| threads.clone())
                .unwrap_or_default())
        }

        async fn start_agent_in_project(
            &self,
            _project_id: &str,
            agent: AgentName,
            workspace: PathBuf,
            _prompt: Option<&str>,
        ) -> Result<StartAgentOutcome> {
            self.start_agent(agent, workspace).await
        }
```

- [ ] **Step 4: Add builder helper methods to TestBackend for tests to seed project data**

After the existing `with_group_chat_pages` method (around line 3573):

```rust
        fn with_projects(self, projects: Vec<crate::backend::ProjectEntry>) -> Self {
            *self.projects.lock().expect("projects lock") = projects;
            self
        }

        fn with_project_threads(
            self,
            project_id: &str,
            threads: Vec<crate::backend::ThreadSummaryEntry>,
        ) -> Self {
            self.project_thread_lists
                .lock()
                .expect("project threads lock")
                .push((project_id.to_owned(), threads));
            self
        }
```

- [ ] **Step 5: Verify full build and all existing tests pass**

Run: `cargo test -p minos-tui 2>&1 | tail -10`
Expected: All existing tests pass. No compile errors.

- [ ] **Step 6: Commit**

```bash
git add crates/minos-tui/src/app.rs
git commit -m "test(tui): add project trait method stubs to TestBackend"
```

### Task 1.5: 写 project trait 方法的单元测试

**Files:**
- Modify: `crates/minos-tui/src/app.rs` (add tests at the end of `#[cfg(test)] mod tests`)

- [ ] **Step 1: Write test for create_project + list_projects round-trip**

Add at the end of the test module (before the closing `}`):

```rust
    #[tokio::test]
    async fn create_project_appears_in_list() {
        let backend = Arc::new(TestBackend::new());
        let created = backend
            .create_project("MyProj", std::path::Path::new("/tmp/myproj"))
            .await
            .unwrap();
        assert_eq!(created.name, "MyProj");
        assert_eq!(created.workspace_path, PathBuf::from("/tmp/myproj"));

        let list = backend.list_projects().await.unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].name, "MyProj");
    }

    #[tokio::test]
    async fn list_project_threads_returns_seeded_data() {
        let backend = Arc::new(
            TestBackend::new().with_project_threads(
                "proj-1",
                vec![crate::backend::ThreadSummaryEntry {
                    thread_id: "thread-99".into(),
                    agent: AgentName::Codex,
                    title: Some("test title".into()),
                    first_ts_ms: 1000,
                    last_ts_ms: 2000,
                    message_count: 5,
                    ended_at_ms: None,
                }],
            ),
        );
        let threads = backend.list_project_threads("proj-1").await.unwrap();
        assert_eq!(threads.len(), 1);
        assert_eq!(threads[0].thread_id, "thread-99");
        assert_eq!(threads[0].title.as_deref(), Some("test title"));
    }

    #[tokio::test]
    async fn start_agent_in_project_delegates_to_start_agent() {
        let backend = Arc::new(
            TestBackend::new().with_agents(vec![ok_agent(AgentName::Codex)]),
        );
        let outcome = backend
            .start_agent_in_project(
                "proj-1",
                AgentName::Codex,
                PathBuf::from("/tmp"),
                Some("hello"),
            )
            .await
            .unwrap();
        assert!(outcome.thread_id.starts_with("thread-"));
        let started = backend.started.lock().expect("started lock");
        assert_eq!(*started, vec![AgentName::Codex]);
    }
```

- [ ] **Step 2: Run tests**

Run: `cargo test -p minos-tui -- create_project list_project_threads start_agent_in_project 2>&1 | tail -15`
Expected: 3 new tests pass.

- [ ] **Step 3: Commit**

```bash
git add crates/minos-tui/src/app.rs
git commit -m "test(tui): add unit tests for project backend trait methods"
```

---

## Phase 2: NavLevel 导航栈 + 启动 cwd 匹配

### Task 2.1: 创建 nav.rs — NavLevel 枚举和导航栈

**Files:**
- Create: `crates/minos-tui/src/nav.rs`
- Modify: `crates/minos-tui/src/main.rs` (add `mod nav;`)

- [ ] **Step 1: Create `src/nav.rs`**

```rust
use minos_domain::AgentName;

/// Navigation level in the Project → Thread → Agent hierarchy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NavLevel {
    /// Top level: list of all projects.
    Projects,
    /// Second level: threads within a project.
    Threads { project_id: String },
    /// Third level: a single thread (group chat with agent overview sidebar).
    Thread { project_id: String, thread_id: String },
    /// Detail view: a specific agent's conversation within a thread.
    Agent {
        project_id: String,
        thread_id: String,
        agent: AgentName,
    },
}

impl NavLevel {
    /// Returns the project_id if this level is within a project context.
    pub fn project_id(&self) -> Option<&str> {
        match self {
            NavLevel::Projects => None,
            NavLevel::Threads { project_id }
            | NavLevel::Thread { project_id, .. }
            | NavLevel::Agent { project_id, .. } => Some(project_id.as_str()),
        }
    }

    /// Returns the thread_id if this level is within a thread context.
    pub fn thread_id(&self) -> Option<&str> {
        match self {
            NavLevel::Projects | NavLevel::Threads { .. } => None,
            NavLevel::Thread { thread_id, .. }
            | NavLevel::Agent { thread_id, .. } => Some(thread_id.as_str()),
        }
    }

    /// Whether Esc at this level should quit the program (true only at Projects).
    pub fn esc_quits(&self) -> bool {
        matches!(self, NavLevel::Projects)
    }
}

/// A navigation stack. `last()` determines what's rendered.
#[derive(Debug, Clone, Default)]
pub struct NavStack {
    levels: Vec<NavLevel>,
}

impl NavStack {
    pub fn new() -> Self {
        Self::default()
    }

    /// Current level, or `None` if the stack is empty (program should quit).
    pub fn current(&self) -> Option<&NavLevel> {
        self.levels.last()
    }

    pub fn push(&mut self, level: NavLevel) {
        self.levels.push(level);
    }

    /// Pop the current level. Returns `true` if the stack is now empty.
    pub fn pop(&mut self) -> bool {
        self.levels.pop();
        self.levels.is_empty()
    }

    pub fn is_empty(&self) -> bool {
        self.levels.is_empty()
    }

    pub fn depth(&self) -> usize {
        self.levels.len()
    }

    /// Replace the entire stack with a single level (used at startup).
    pub fn reset_to(&mut self, level: NavLevel) {
        self.levels.clear();
        self.levels.push(level);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn push_pop_roundtrip() {
        let mut stack = NavStack::new();
        assert!(stack.is_empty());
        stack.push(NavLevel::Projects);
        assert_eq!(stack.current(), Some(&NavLevel::Projects));
        assert!(!stack.pop()); // not empty, still Projects
        assert!(stack.pop()); // now empty
    }

    #[test]
    fn project_id_extraction() {
        assert_eq!(NavLevel::Projects.project_id(), None);
        assert_eq!(
            NavLevel::Threads { project_id: "p1".into() }.project_id(),
            Some("p1")
        );
        assert_eq!(
            NavLevel::Thread {
                project_id: "p1".into(),
                thread_id: "t1".into()
            }
            .thread_id(),
            Some("t1")
        );
    }

    #[test]
    fn esc_quits_only_at_projects() {
        assert!(NavLevel::Projects.esc_quits());
        assert!(!NavLevel::Threads { project_id: "p".into() }.esc_quits());
    }

    #[test]
    fn reset_to_clears_stack() {
        let mut stack = NavStack::new();
        stack.push(NavLevel::Projects);
        stack.push(NavLevel::Threads {
            project_id: "p".into(),
        });
        stack.reset_to(NavLevel::Projects);
        assert_eq!(stack.depth(), 1);
    }
}
```

- [ ] **Step 2: Add `mod nav;` to `main.rs`**

In `main.rs`, find the module declarations (near the top, after the `use` statements). Add:
```rust
mod nav;
```

Also add `pub mod nav;` in `src/lib.rs` if there is one, or ensure `nav` is accessible. Check how other modules are declared — look for `mod backend;`, `mod ui;` etc. and add `nav` alongside them.

- [ ] **Step 3: Run nav tests**

Run: `cargo test -p minos-tui -- nav 2>&1 | tail -10`
Expected: 4 nav tests pass.

- [ ] **Step 4: Commit**

```bash
git add crates/minos-tui/src/nav.rs crates/minos-tui/src/main.rs
git commit -m "feat(tui): add NavLevel enum and NavStack navigation stack"
```

### Task 2.2: 在 UiState 中加 nav_stack 字段

**Files:**
- Modify: `crates/minos-tui/src/ui/mod.rs`
- Modify: `crates/minos-tui/src/app.rs`

- [ ] **Step 1: Add `nav_stack` to `UiState`**

In `ui/mod.rs`, add the import:
```rust
use crate::nav::NavStack;
```

In the `UiState` struct (after `pub render_cache: RenderCache,` around line 66), add:
```rust
    pub nav_stack: NavStack,
    pub projects: Vec<crate::backend::ProjectEntry>,
    pub selected_project: Option<usize>,
    pub project_list_state: ListState,
    pub thread_summaries: Vec<crate::backend::ThreadSummaryEntry>,
```

- [ ] **Step 2: Initialize the new fields in `UiState::new()`**

Find `UiState::new()` (search for `pub fn new(readonly`). Add after `render_cache: RenderCache::default(),`:
```rust
            nav_stack: NavStack::new(),
            projects: Vec::new(),
            selected_project: None,
            project_list_state: ListState::default(),
            thread_summaries: Vec::new(),
```

- [ ] **Step 3: Verify it compiles**

Run: `cargo build -p minos-tui 2>&1 | grep '^error' | head -10`
Expected: clean compile (no errors).

- [ ] **Step 4: Commit**

```bash
git add crates/minos-tui/src/ui/mod.rs
git commit -m "feat(tui): add nav_stack and project fields to UiState"
```

### Task 2.3: 实现 App 启动时的 cwd → project 匹配

**Files:**
- Modify: `crates/minos-tui/src/app.rs`

- [ ] **Step 1: Add a method to App for project matching**

In `app.rs`, in the `impl App` block, add a new method after `init()`:

```rust
    /// At startup, match cwd against known projects.
    /// If a project matches, push Threads level. Otherwise push Projects level
    /// and optionally prompt to create.
    async fn resolve_startup_project(&mut self) -> anyhow::Result<()> {
        let projects = self.backend.list_projects().await?;
        self.ui.projects = projects.clone();

        let cwd = &self.workspace;
        let matched = projects
            .iter()
            .find(|p| workspace_paths_match(&p.workspace_path, cwd));

        if let Some(project) = matched {
            self.load_project_threads(&project.project_id).await?;
            self.ui.nav_stack.reset_to(crate::nav::NavLevel::Threads {
                project_id: project.project_id.clone(),
            });
            self.ui.selected_project = Some(
                self.ui
                    .projects
                    .iter()
                    .position(|p| p.project_id == project.project_id),
            );
        } else {
            self.ui.nav_stack.reset_to(crate::nav::NavLevel::Projects);
        }
        Ok(())
    }

    /// Load threads for a project into `ui.thread_summaries`.
    async fn load_project_threads(&mut self, project_id: &str) -> anyhow::Result<()> {
        let threads = self.backend.list_project_threads(project_id).await?;
        self.ui.thread_summaries = threads;
        Ok(())
    }
```

- [ ] **Step 2: Call `resolve_startup_project` from `init()`**

Modify `init()` (around line 81-93). After the existing `hydrate_daemon_threads` call, add:

```rust
        self.resolve_startup_project().await?;
```

So `init()` becomes:
```rust
    pub async fn init(&mut self) -> anyhow::Result<()> {
        self.load_group_chat_history().await;
        let agents = self.backend.detect_clis().await?;
        self.ui.status.update_agents(agents);
        self.sync_input_agent_picker();
        if matches!(
            self.backend.connection_state(),
            crate::backend::BackendConnectionState::Connected { .. }
        ) {
            self.hydrate_daemon_threads().await;
        }
        self.resolve_startup_project().await?;
        Ok(())
    }
```

- [ ] **Step 3: Write test for startup project matching**

Add to the test module in `app.rs`:

```rust
    #[tokio::test]
    async fn startup_matches_cwd_to_project() {
        let backend = Arc::new(
            TestBackend::new().with_projects(vec![crate::backend::ProjectEntry {
                project_id: "proj-minos".into(),
                name: "Minos".into(),
                workspace_path: PathBuf::from("/tmp/minos"),
                thread_count: 3,
                created_at_ms: 0,
                updated_at_ms: 0,
            }]),
        );
        let mut app = App::new(backend, false, PathBuf::from("/tmp/minos"));
        app.init().await.unwrap();

        assert_eq!(
            app.ui.nav_stack.current(),
            Some(&crate::nav::NavLevel::Threads {
                project_id: "proj-minos".into()
            })
        );
    }

    #[tokio::test]
    async fn startup_falls_back_to_projects_list_when_no_match() {
        let backend = Arc::new(
            TestBackend::new().with_projects(vec![crate::backend::ProjectEntry {
                project_id: "proj-other".into(),
                name: "Other".into(),
                workspace_path: PathBuf::from("/tmp/other"),
                thread_count: 0,
                created_at_ms: 0,
                updated_at_ms: 0,
            }]),
        );
        let mut app = App::new(backend, false, PathBuf::from("/tmp/minos"));
        app.init().await.unwrap();

        assert_eq!(
            app.ui.nav_stack.current(),
            Some(&crate::nav::NavLevel::Projects)
        );
    }
```

- [ ] **Step 4: Run tests**

Run: `cargo test -p minos-tui -- startup_matches startup_falls 2>&1 | tail -10`
Expected: 2 new tests pass.

- [ ] **Step 5: Commit**

```bash
git add crates/minos-tui/src/app.rs
git commit -m "feat(tui): resolve startup project from cwd match"
```

---

## Phase 3: Projects 列表视图

### Task 3.1: 创建 project_list.rs 渲染模块

**Files:**
- Create: `crates/minos-tui/src/ui/project_list.rs`
- Modify: `crates/minos-tui/src/ui/mod.rs` (add module declaration)

- [ ] **Step 1: Add module declaration in `ui/mod.rs`**

After `pub mod thread_list;` (line 8), add:
```rust
pub mod project_list;
```

- [ ] **Step 2: Create `src/ui/project_list.rs`**

```rust
use crate::backend::ProjectEntry;
use crate::ui::theme;
use ratatui::{
    layout::Rect,
    style::Style,
    text::{Line, Span},
    widgets::{Block, List, ListItem, ListState, Paragraph},
    Frame,
};

/// Render the project list in the main content area.
pub fn render_project_list(
    f: &mut Frame,
    area: Rect,
    projects: &[ProjectEntry],
    selected: Option<usize>,
    list_state: &mut ListState,
    focused: bool,
) {
    let border_style = if focused {
        theme::FOCUSED_BORDER
    } else {
        Style::new().fg(theme::BORDER_FG)
    };
    let block = Block::bordered()
        .title("Projects")
        .border_style(border_style);

    let items: Vec<ListItem> = projects
        .iter()
        .map(|p| {
            let path_display = p
                .workspace_path
                .to_string_lossy();
            let line = Line::from(vec![
                Span::styled(
                    format!("{:<16} ", p.name),
                    Style::new().fg(ratatui::style::Color::Cyan),
                ),
                Span::raw(path_display.to_string()),
            ]);
            ListItem::new(line)
        })
        .collect();

    let list = List::new(items)
        .block(block)
        .highlight_style(theme::HIGHLIGHTED)
        .repeat_highlight_symbol(false);

    // Ensure list_state selection is valid
    if selected.is_none() && !projects.is_empty() {
        list_state.select(Some(0));
    }

    f.render_stateful_widget(list, area, list_state);
}

/// Render the project sidebar (selected project's info).
pub fn render_project_sidebar(
    f: &mut Frame,
    area: Rect,
    projects: &[ProjectEntry],
    selected: Option<usize>,
) {
    let block = Block::bordered()
        .title("Project Info")
        .border_style(Style::new().fg(theme::BORDER_FG));

    let content = if let Some(idx) = selected {
        if let Some(project) = projects.get(idx) {
            let lines = vec![
                Line::from(vec![
                    Span::styled("Name: ", Style::new().fg(theme::BORDER_FG)),
                    Span::raw(&project.name),
                ]),
                Line::from(vec![
                    Span::styled("Path: ", Style::new().fg(theme::BORDER_FG)),
                    Span::raw(project.workspace_path.to_string_lossy().to_string()),
                ]),
                Line::from(vec![
                    Span::styled("Threads: ", Style::new().fg(theme::BORDER_FG)),
                    Span::raw(project.thread_count.to_string()),
                ]),
            ];
            Paragraph::new(lines).block(block)
        } else {
            Paragraph::new("No project selected").block(block)
        }
    } else {
        Paragraph::new("Select a project").block(block)
    };

    f.render_widget(content, area);
}

/// Render the bottom hint bar for the Projects level.
pub fn render_project_bottom_hint(f: &mut Frame, area: Rect) {
    let hint = Paragraph::new(Line::from(vec![
        Span::styled("[n] ", theme::FOCUSED_BORDER),
        Span::raw("新建  "),
        Span::styled("[Enter] ", theme::FOCUSED_BORDER),
        Span::raw("打开  "),
        Span::styled("[d] ", theme::FOCUSED_BORDER),
        Span::raw("删除  "),
        Span::styled("[Esc] ", theme::FOCUSED_BORDER),
        Span::raw("退出"),
    ]));
    f.render_widget(hint, area);
}
```

- [ ] **Step 3: Verify it compiles**

Run: `cargo build -p minos-tui 2>&1 | grep '^error' | head -10`
Expected: clean compile.

- [ ] **Step 4: Commit**

```bash
git add crates/minos-tui/src/ui/project_list.rs crates/minos-tui/src/ui/mod.rs
git commit -m "feat(tui): add project_list rendering module"
```

### Task 3.2: 实现统一布局骨架函数

**Files:**
- Modify: `crates/minos-tui/src/ui/mod.rs`

- [ ] **Step 1: Add the unified layout function in `ui/mod.rs`**

Add after the imports (before the struct definitions):

```rust
/// Layout areas for a single navigation level.
pub struct LevelLayout {
    pub status_bar: Rect,
    pub main: Rect,
    pub sidebar: Rect,
    pub bottom: Rect,
}

/// Split the frame into status bar (1 line), main+sidebar row, and bottom bar.
/// When `sidebar_overlay` is true, main takes 100% width and sidebar floats on top.
/// Otherwise main gets ~78% and sidebar ~22%.
pub fn split_level(area: Rect, bottom_height: u16, sidebar_overlay: bool) -> LevelLayout {
    let outer = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(0),
            Constraint::Length(bottom_height.max(1)),
        ])
        .split(area);

    let status_bar = outer[0];
    let middle = outer[1];
    let bottom = outer[2];

    let (main, sidebar) = if sidebar_overlay {
        (middle, middle)
    } else {
        let cols = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Percentage(78),
                Constraint::Percentage(22),
            ])
            .split(middle);
        (cols[0], cols[1])
    };

    LevelLayout {
        status_bar,
        main,
        sidebar,
        bottom,
    }
}
```

- [ ] **Step 2: Add a helper to determine if sidebar should overlay**

```rust
/// Whether the sidebar should be an overlay (terminal width <= 120).
pub fn sidebar_should_overlay(width: u16) -> bool {
    width <= 120
}
```

- [ ] **Step 3: Verify compile**

Run: `cargo build -p minos-tui 2>&1 | grep '^error' | head -5`
Expected: clean.

- [ ] **Step 4: Commit**

```bash
git add crates/minos-tui/src/ui/mod.rs
git commit -m "feat(tui): add unified LevelLayout split function"
```

### Task 3.3: 在 render_ui 中按 NavLevel 分发到 Projects 视图

**Files:**
- Modify: `crates/minos-tui/src/ui/mod.rs`

- [ ] **Step 1: Add a `render_projects_level` function**

In `ui/mod.rs`, add:

```rust
fn render_projects_level(f: &mut Frame, state: &mut UiState) {
    let overlay = sidebar_should_overlay(f.area().width);
    let layout = split_level(f.area(), 1, overlay);

    status_bar::render_status_bar(
        f,
        layout.status_bar,
        &state.status,
        state.is_flash_copied_active(),
    );

    let focused = matches!(state.focus, Focus::RoomList);

    project_list::render_project_list(
        f,
        layout.main,
        &state.projects,
        state.selected_project,
        &mut state.project_list_state,
        focused,
    );

    if !overlay {
        project_list::render_project_sidebar(
            f,
            layout.sidebar,
            &state.projects,
            state.selected_project,
        );
    }

    project_list::render_project_bottom_hint(f, layout.bottom);
}
```

- [ ] **Step 2: Modify `render_ui` to dispatch by nav level**

Replace the existing `render_ui` function (lines 327-366) with:

```rust
pub fn render_ui(f: &mut Frame, state: &mut UiState) {
    state.room_input.focused = matches!(state.focus, Focus::RoomInput);
    state.agent_input.focused = matches!(state.focus, Focus::AgentInput);

    match state.nav_stack.current() {
        Some(crate::nav::NavLevel::Projects) => {
            render_projects_level(f, state);
        }
        Some(crate::nav::NavLevel::Threads { .. }) => {
            // Phase 4: render_threads_level(f, state);
            // For now, fall back to legacy overview mode.
            render_legacy(f, state);
        }
        Some(crate::nav::NavLevel::Thread { .. }) => {
            // Phase 5: render_thread_level(f, state);
            render_legacy(f, state);
        }
        Some(crate::nav::NavLevel::Agent { .. }) => {
            // Phase 6: render_agent_level(f, state);
            render_legacy(f, state);
        }
        None => {
            // Nav stack empty — shouldn't happen, render nothing.
        }
    }

    if let Some(picker) = state.agent_picker.as_ref() {
        agent_picker::render_agent_picker(f, state.status.agents.as_slice(), picker);
    }

    if let Some(confirm) = state.delete_confirm.as_ref() {
        render_delete_confirm(f, confirm);
    }
}

/// Legacy renderer — used for nav levels not yet implemented.
/// Delegates to the old overview/detail mode rendering.
fn render_legacy(f: &mut Frame, state: &mut UiState) {
    let available_height = f.area().height.saturating_sub(1);
    let room_input_height = input_bar::required_height(&state.room_input, f.area().width);
    let input_height = if state.agent_detail_visible {
        let detail_agent_width = f.area().width.saturating_mul(35) / 100;
        let agent_input_height =
            input_bar::required_height(&state.agent_input, detail_agent_width);
        room_input_height.max(agent_input_height)
    } else {
        room_input_height
    }
    .min(available_height);

    let outer = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(0),
            Constraint::Length(input_height.max(1)),
        ])
        .split(f.area());

    status_bar::render_status_bar(
        f,
        outer[0],
        &state.status,
        state.is_flash_copied_active(),
    );

    if state.agent_detail_visible {
        render_detail_mode(f, outer[1], outer[2], state);
    } else {
        render_overview_mode(f, outer[1], outer[2], state);
    }
}
```

Note: `render_overview_mode` and `render_detail_mode` remain unchanged — they're the existing functions.

- [ ] **Step 3: Ensure nav_stack is initialized for existing tests**

The existing tests construct `App` without calling `init()`, so `nav_stack` will be empty. We need to make the legacy path work. Add a fallback in `App::new()` — after `let mut ui = UiState::new(readonly);`, add:

```rust
        ui.nav_stack.reset_to(crate::nav::NavLevel::Projects);
```

This ensures the nav stack always has at least one level, even in tests that don't call `init()`. The `resolve_startup_project` in `init()` will override this.

- [ ] **Step 4: Verify build and run all existing tests**

Run: `cargo test -p minos-tui 2>&1 | tail -10`
Expected: all tests pass. Existing tests that push threads and interact will now start at Projects level but fall through to legacy renderer (since they don't set nav to Threads/Thread level). The tests that directly call `handle_key` with specific key events and check backend state will still work because the legacy renderer is used.

**Important:** Some existing tests call `app.select_thread(0)` and expect detail mode. These tests will need the nav stack set to `Thread` level to trigger legacy detail rendering. Check if any tests break.

- [ ] **Step 5: Fix any broken tests**

If tests break because `nav_stack.current()` is `Projects` but the test expects thread/chat interaction: add `app.ui.nav_stack.reset_to(crate::nav::NavLevel::Thread { project_id: "test".into(), thread_id: "thread-1".into() });` to those tests.

Run: `cargo test -p minos-tui 2>&1 | tail -20`
Expected: all tests pass.

- [ ] **Step 6: Commit**

```bash
git add crates/minos-tui/src/ui/mod.rs crates/minos-tui/src/app.rs
git commit -m "feat(tui): dispatch render by NavLevel, Projects view functional"
```

### Task 3.4: 实现 Projects 列表的键盘交互

**Files:**
- Modify: `crates/minos-tui/src/app.rs`

- [ ] **Step 1: Add a method to handle keys at the Projects level**

In `impl App`, add:

```rust
    /// Handle a key event when at the Projects nav level.
    /// Returns (redraw, consumed). If consumed is false, the caller may
    /// apply global key handling.
    async fn handle_project_list_key(&mut self, key: KeyEvent) -> bool {
        match key.code {
            KeyCode::Up => {
                self.navigate_projects(-1);
                true
            }
            KeyCode::Down => {
                self.navigate_projects(1);
                true
            }
            KeyCode::Enter => {
                self.open_selected_project().await;
                true
            }
            KeyCode::Char('n') => {
                // Phase: create project dialog
                // For now, create a project from cwd name.
                true
            }
            KeyCode::Char('d') => {
                // Phase: delete project
                true
            }
            _ => false,
        }
    }

    fn navigate_projects(&mut self, delta: i32) {
        if self.ui.projects.is_empty() {
            return;
        }
        let current = self.ui.selected_project.unwrap_or(0) as i32;
        let mut next = current + delta;
        if next < 0 {
            next = self.ui.projects.len() as i32 - 1;
        }
        if next >= self.ui.projects.len() as i32 {
            next = 0;
        }
        self.ui.selected_project = Some(next as usize);
        self.ui.project_list_state.select(Some(next as usize));
    }

    async fn open_selected_project(&mut self) {
        if let Some(idx) = self.ui.selected_project {
            if let Some(project) = self.ui.projects.get(idx) {
                let project_id = project.project_id.clone();
                if let Err(e) = self.load_project_threads(&project_id).await {
                    self.flash_error(format!("Failed to load threads: {e}"));
                    return;
                }
                self.ui.nav_stack.push(crate::nav::NavLevel::Threads {
                    project_id,
                });
            }
        }
    }
```

- [ ] **Step 2: Wire project list key handling into `handle_key`**

In the existing `handle_key` method, at the very beginning (before any other key dispatch), add a nav-level check:

```rust
    pub async fn handle_key(&mut self, key: KeyEvent) -> bool {
        // Nav-level dispatch: if we're at Projects level, handle there first.
        if let Some(crate::nav::NavLevel::Projects) = self.ui.nav_stack.current() {
            // Global keys first.
            if matches!(key.code, KeyCode::Char('q'))
                && key.modifiers.contains(KeyModifiers::CONTROL)
            {
                self.should_quit = true;
                return true;
            }
            if key.code == KeyCode::Esc {
                self.should_quit = true;
                return true;
            }
            let consumed = self.handle_project_list_key(key).await;
            if consumed {
                return true;
            }
        }
        // ... existing key handling continues for other levels ...
```

Find the existing `handle_key` method start and insert this block. Be careful not to duplicate the `Ctrl+Q` / `Esc` handling that may already exist for the global case.

- [ ] **Step 3: Write test for project navigation and opening**

```rust
    #[tokio::test]
    async fn project_list_navigate_and_open() {
        let backend = Arc::new(TestBackend::new().with_projects(vec![
            crate::backend::ProjectEntry {
                project_id: "p1".into(),
                name: "Proj1".into(),
                workspace_path: PathBuf::from("/tmp/p1"),
                thread_count: 0,
                created_at_ms: 0,
                updated_at_ms: 0,
            },
            crate::backend::ProjectEntry {
                project_id: "p2".into(),
                name: "Proj2".into(),
                workspace_path: PathBuf::from("/tmp/p2"),
                thread_count: 0,
                created_at_ms: 0,
                updated_at_ms: 0,
            },
        ]));
        let mut app = App::new(backend, false, PathBuf::from("/tmp/test"));
        app.ui.nav_stack.reset_to(crate::nav::NavLevel::Projects);

        // Down to select second project
        app.handle_key(press(KeyCode::Down)).await;
        assert_eq!(app.ui.selected_project, Some(1));

        // Enter to open
        app.handle_key(press(KeyCode::Enter)).await;
        assert_eq!(
            app.ui.nav_stack.current(),
            Some(&crate::nav::NavLevel::Threads {
                project_id: "p2".into()
            })
        );
    }
```

- [ ] **Step 4: Run tests**

Run: `cargo test -p minos-tui -- project_list_navigate 2>&1 | tail -10`
Expected: test passes.

- [ ] **Step 5: Commit**

```bash
git add crates/minos-tui/src/app.rs
git commit -m "feat(tui): project list keyboard navigation and open"
```

### Task 3.5: Project 创建对话框

**Files:**
- Create: `crates/minos-tui/src/ui/project_create_dialog.rs`
- Modify: `crates/minos-tui/src/ui/mod.rs`

- [ ] **Step 1: Add module declaration and dialog state**

In `ui/mod.rs`:
```rust
pub mod project_create_dialog;
```

Add the dialog state struct:
```rust
pub struct ProjectCreateDialogState {
    pub name: String,
    pub path: String,
    pub editing_name: bool, // true = editing name field, false = editing path field
}
```

Add `pub project_create_dialog: Option<ProjectCreateDialogState>,` to `UiState` and initialize to `None` in `new()`.

- [ ] **Step 2: Create `src/ui/project_create_dialog.rs`**

```rust
use crate::ui::theme;
use crate::ui::ProjectCreateDialogState;
use ratatui::{
    layout::{Constraint, Direction, Flex, Layout, Rect},
    style::Style,
    text::{Line, Span},
    widgets::{Block, Clear, Paragraph},
    Frame,
};

pub fn centered_rect(width: u16, height: u16, area: Rect) -> Rect {
    let popup = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Fill(1),
            Constraint::Length(height),
            Constraint::Fill(1),
        ])
        .flex(Flex::Center)
        .split(area);
    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Fill(1),
            Constraint::Length(width),
            Constraint::Fill(1),
        ])
        .flex(Flex::Center)
        .split(popup[1])[1]
}

pub fn render_project_create_dialog(f: &mut Frame, area: Rect, state: &ProjectCreateDialogState) {
    let dialog_area = centered_rect(50, 8, area);
    f.render_widget(Clear, dialog_area);

    let block = Block::bordered()
        .title("New Project")
        .border_style(theme::FOCUSED_BORDER);

    let name_cursor = if state.editing_name { "█" } else { "" };
    let path_cursor = if !state.editing_name { "█" } else { "" };

    let lines = vec![
        Line::raw(""),
        Line::from(vec![
            Span::styled("Name: ", Style::new().fg(theme::BORDER_FG)),
            Span::raw(&state.name),
            Span::raw(name_cursor),
        ]),
        Line::raw(""),
        Line::from(vec![
            Span::styled("Path: ", Style::new().fg(theme::BORDER_FG)),
            Span::raw(&state.path),
            Span::raw(path_cursor),
        ]),
        Line::raw(""),
        Line::from(vec![
            Span::styled("[Tab] ", theme::FOCUSED_BORDER),
            Span::raw("切换字段  "),
            Span::styled("[Enter] ", theme::FOCUSED_BORDER),
            Span::raw("创建  "),
            Span::styled("[Esc] ", theme::FOCUSED_BORDER),
            Span::raw("取消"),
        ]),
    ];

    f.render_widget(Paragraph::new(lines).block(block), dialog_area);
}
```

- [ ] **Step 3: Wire the dialog into render_ui**

In `render_projects_level`, before the closing brace, add:
```rust
    if let Some(dialog) = state.project_create_dialog.as_ref() {
        project_create_dialog::render_project_create_dialog(f, f.area(), dialog);
    }
```

- [ ] **Step 4: Handle dialog input in app.rs**

Add to `handle_project_list_key`, replace the `'n'` arm:
```rust
            KeyCode::Char('n') => {
                let cwd_name = self
                    .workspace
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| "workspace".to_owned());
                self.ui.project_create_dialog = Some(crate::ui::ProjectCreateDialogState {
                    name: cwd_name,
                    path: self.workspace.to_string_lossy().into_owned(),
                    editing_name: true,
                });
                true
            }
```

Add a new method for dialog key handling:
```rust
    async fn handle_project_create_dialog_key(&mut self, key: KeyEvent) -> bool {
        let dialog = match self.ui.project_create_dialog.as_mut() {
            Some(d) => d,
            None => return false,
        };
        match key.code {
            KeyCode::Esc => {
                self.ui.project_create_dialog = None;
                true
            }
            KeyCode::Tab => {
                dialog.editing_name = !dialog.editing_name;
                true
            }
            KeyCode::Enter => {
                let name = dialog.name.clone();
                let path = std::path::PathBuf::from(&dialog.path);
                self.ui.project_create_dialog = None;
                self.create_and_open_project(name, path).await;
                true
            }
            KeyCode::Backspace => {
                let field = if dialog.editing_name { &mut dialog.name } else { &mut dialog.path };
                field.pop();
                true
            }
            KeyCode::Char(c) => {
                let field = if dialog.editing_name { &mut dialog.name } else { &mut dialog.path };
                field.push(c);
                true
            }
            _ => false,
        }
    }

    async fn create_and_open_project(&mut self, name: String, path: PathBuf) {
        match self.backend.create_project(&name, &path).await {
            Ok(project) => {
                let project_id = project.project_id.clone();
                self.ui.projects.push(project);
                if let Err(e) = self.load_project_threads(&project_id).await {
                    self.flash_error(format!("Failed to load threads: {e}"));
                    return;
                }
                self.ui.nav_stack.push(crate::nav::NavLevel::Threads {
                    project_id,
                });
            }
            Err(e) => {
                self.flash_error(format!("Failed to create project: {e}"));
            }
        }
    }
```

In `handle_key`, add the dialog check **before** the Projects nav check:
```rust
        if self.ui.project_create_dialog.is_some() {
            let consumed = self.handle_project_create_dialog_key(key).await;
            if consumed {
                return true;
            }
        }
```

- [ ] **Step 5: Write test for project creation flow**

```rust
    #[tokio::test]
    async fn project_create_dialog_flow() {
        let backend = Arc::new(TestBackend::new());
        let mut app = App::new(backend.clone(), false, PathBuf::from("/tmp/newproj"));
        app.ui.nav_stack.reset_to(crate::nav::NavLevel::Projects);

        // Press 'n' to open dialog
        app.handle_key(press(KeyCode::Char('n'))).await;
        assert!(app.ui.project_create_dialog.is_some());

        // Type a name
        app.handle_key(press(KeyCode::Char('M'))).await;
        let dialog = app.ui.project_create_dialog.as_ref().unwrap();
        assert!(dialog.name.ends_with('M'));

        // Enter to create
        app.handle_key(press(KeyCode::Enter)).await;
        assert!(app.ui.project_create_dialog.is_none());
        assert_eq!(
            app.ui.nav_stack.current(),
            Some(&crate::nav::NavLevel::Threads {
                project_id: "test-project-M".into()
            })
        );
        let created = backend.created_projects.lock().unwrap();
        assert!(!created.is_empty());
    }
```

- [ ] **Step 6: Run tests**

Run: `cargo test -p minos-tui -- project_create 2>&1 | tail -10`
Expected: test passes.

- [ ] **Step 7: Commit**

```bash
git add crates/minos-tui/src/ui/project_create_dialog.rs crates/minos-tui/src/ui/mod.rs crates/minos-tui/src/app.rs
git commit -m "feat(tui): project creation dialog with name/path input"
```

### Task 3.6: 启动时 cwd 未命中 project 的 Y/n 确认提示

**Files:**
- Modify: `crates/minos-tui/src/ui/mod.rs` (add startup prompt state)
- Modify: `crates/minos-tui/src/app.rs` (render + handle prompt)

**Note:** Spec §7.1 — when startup cwd doesn't match any project, prompt user to create one.

- [ ] **Step 1: Add a `StartupPrompt` state to `UiState`**

In `ui/mod.rs`, add:
```rust
pub struct StartupCreatePromptState {
    pub dir_name: String,
    pub path: String,
}

// In UiState struct, add:
    pub startup_create_prompt: Option<StartupCreatePromptState>,
```

Initialize to `None` in `UiState::new()`.

- [ ] **Step 2: Trigger the prompt in `resolve_startup_project` when no match**

In `app.rs`, modify `resolve_startup_project` — replace the `else` branch:

```rust
        } else {
            self.ui.nav_stack.reset_to(crate::nav::NavLevel::Projects);
            // If cwd has a meaningful name, offer to create a project.
            let dir_name = cwd
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| "workspace".to_owned());
            self.ui.startup_create_prompt = Some(crate::ui::StartupCreatePromptState {
                dir_name,
                path: cwd.to_string_lossy().into_owned(),
            });
        }
```

- [ ] **Step 3: Render the startup prompt**

In `render_projects_level`, after the project_create_dialog check, add:
```rust
    if let Some(prompt) = state.startup_create_prompt.as_ref() {
        use ratatui::layout::{Constraint, Direction, Flex, Layout};
        let area = f.area();
        let popup = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Fill(1), Constraint::Length(7), Constraint::Fill(1)])
            .flex(Flex::Center)
            .split(area);
        let popup = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Fill(1), Constraint::Length(52), Constraint::Fill(1)])
            .flex(Flex::Center)
            .split(popup[1])[1];

        f.render_widget(ratatui::widgets::Clear, popup);
        let lines = vec![
            ratatui::text::Line::raw(""),
            ratatui::text::Line::from(format!(
                "  Create project \"{}\" ({})?",
                prompt.dir_name, prompt.path
            )),
            ratatui::text::Line::raw(""),
            ratatui::text::Line::from(vec![
                ratatui::text::Span::styled("[Y] ", theme::FOCUSED_BORDER),
                ratatui::text::Span::raw("Create & enter  "),
                ratatui::text::Span::styled("[n] ", theme::FOCUSED_BORDER),
                ratatui::text::Span::raw("Skip"),
            ]),
        ];
        let block = ratatui::widgets::Block::bordered()
            .title("New Directory Detected")
            .border_style(theme::FOCUSED_BORDER);
        f.render_widget(ratatui::widgets::Paragraph::new(lines).block(block), popup);
    }
```

- [ ] **Step 4: Handle Y/n keys for the startup prompt**

In `handle_key`, add at the very top (before all other dispatch):

```rust
        if let Some(prompt) = self.ui.startup_create_prompt.as_ref() {
            match key.code {
                KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Enter => {
                    let prompt = self.ui.startup_create_prompt.take().unwrap();
                    self.create_and_open_project(
                        prompt.dir_name,
                        PathBuf::from(prompt.path),
                    )
                    .await;
                    return true;
                }
                KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                    self.ui.startup_create_prompt = None;
                    return true;
                }
                _ => return true, // consume all keys while prompt is visible
            }
        }
```

- [ ] **Step 5: Write test**

```rust
    #[tokio::test]
    async fn startup_prompt_offers_create_on_no_match() {
        let backend = Arc::new(TestBackend::new());
        let mut app = App::new(backend.clone(), false, PathBuf::from("/tmp/myproj"));
        app.init().await.unwrap();

        assert!(app.ui.startup_create_prompt.is_some());
        assert_eq!(
            app.ui.nav_stack.current(),
            Some(&crate::nav::NavLevel::Projects)
        );

        // Press Y
        app.handle_key(press(KeyCode::Char('y'))).await;
        assert!(app.ui.startup_create_prompt.is_none());
        assert!(matches!(
            app.ui.nav_stack.current(),
            Some(crate::nav::NavLevel::Threads { .. })
        ));
    }

    #[tokio::test]
    async fn startup_prompt_dismissed_on_n() {
        let backend = Arc::new(TestBackend::new());
        let mut app = App::new(backend.clone(), false, PathBuf::from("/tmp/myproj"));
        app.init().await.unwrap();

        app.handle_key(press(KeyCode::Char('n'))).await;
        assert!(app.ui.startup_create_prompt.is_none());
        assert_eq!(
            app.ui.nav_stack.current(),
            Some(&crate::nav::NavLevel::Projects)
        );
    }
```

- [ ] **Step 6: Run tests**

Run: `cargo test -p minos-tui -- startup_prompt 2>&1 | tail -10`
Expected: 2 new tests pass.

- [ ] **Step 7: Commit**

```bash
git add crates/minos-tui/src/ui/mod.rs crates/minos-tui/src/app.rs
git commit -m "feat(tui): startup Y/n prompt when cwd has no matching project"
```

---

## Phase 4: Threads 列表视图

### Task 4.1: 创建 thread_list_v2.rs 渲染模块

**Files:**
- Create: `crates/minos-tui/src/ui/thread_list_v2.rs`
- Modify: `crates/minos-tui/src/ui/mod.rs`

- [ ] **Step 1: Add module declaration**

In `ui/mod.rs`:
```rust
pub mod thread_list_v2;
```

- [ ] **Step 2: Create `src/ui/thread_list_v2.rs`**

```rust
use crate::backend::ThreadSummaryEntry;
use crate::ui::theme;
use ratatui::{
    layout::Rect,
    style::Style,
    text::{Line, Span},
    widgets::{Block, List, ListItem, ListState, Paragraph},
    Frame,
};

/// Render the threads list in the main content area.
pub fn render_threads_list(
    f: &mut Frame,
    area: Rect,
    project_name: &str,
    threads: &[ThreadSummaryEntry],
    selected: Option<usize>,
    list_state: &mut ListState,
    focused: bool,
) {
    let border_style = if focused {
        theme::FOCUSED_BORDER
    } else {
        Style::new().fg(theme::BORDER_FG)
    };
    let title = format!("Threads — {}", project_name);
    let block = Block::bordered()
        .title(title)
        .border_style(border_style);

    let items: Vec<ListItem> = threads
        .iter()
        .map(|t| {
            let id_short = &t.thread_id[..8.min(t.thread_id.len())];
            let title_text = t.title.clone().unwrap_or_else(|| "(untitled)".to_owned());
            let line = Line::from(vec![
                Span::styled(
                    format!("#{} ", id_short),
                    Style::new().fg(ratatui::style::Color::DarkGray),
                ),
                Span::raw(title_text),
            ]);
            ListItem::new(line)
        })
        .collect();

    let list = List::new(items)
        .block(block)
        .highlight_style(theme::HIGHLIGHTED);

    f.render_stateful_widget(list, area, list_state);
}

/// Render the thread sidebar (selected thread's info).
pub fn render_thread_sidebar(
    f: &mut Frame,
    area: Rect,
    threads: &[ThreadSummaryEntry],
    selected: Option<usize>,
) {
    let block = Block::bordered()
        .title("Thread Info")
        .border_style(Style::new().fg(theme::BORDER_FG));

    let content = if let Some(idx) = selected {
        if let Some(thread) = threads.get(idx) {
            let title = thread.title.clone().unwrap_or_else(|| "(untitled)".to_owned());
            let lines = vec![
                Line::from(vec![
                    Span::styled("Title: ", Style::new().fg(theme::BORDER_FG)),
                    Span::raw(title),
                ]),
                Line::from(vec![
                    Span::styled("Agent: ", Style::new().fg(theme::BORDER_FG)),
                    Span::raw(thread.agent.bin_name()),
                ]),
                Line::from(vec![
                    Span::styled("Messages: ", Style::new().fg(theme::BORDER_FG)),
                    Span::raw(thread.message_count.to_string()),
                ]),
            ];
            Paragraph::new(lines).block(block)
        } else {
            Paragraph::new("No thread selected").block(block)
        }
    } else {
        Paragraph::new("Type a message below to start").block(block)
    };

    f.render_widget(content, area);
}
```

- [ ] **Step 3: Commit**

```bash
git add crates/minos-tui/src/ui/thread_list_v2.rs crates/minos-tui/src/ui/mod.rs
git commit -m "feat(tui): add thread_list_v2 rendering module"
```

### Task 4.2: 实现 Threads 列表视图渲染 + 输入栏

**Files:**
- Modify: `crates/minos-tui/src/ui/mod.rs`
- Modify: `crates/minos-tui/src/app.rs`

- [ ] **Step 1: Add `render_threads_level` in `ui/mod.rs`**

```rust
fn render_threads_level(f: &mut Frame, state: &mut UiState) {
    let overlay = sidebar_should_overlay(f.area().width);
    let input_height = input_bar::required_height(&state.room_input, f.area().width);
    let bottom_height = input_height + 1; // input + 1 line for hint
    let layout = split_level(f.area(), bottom_height, overlay);

    status_bar::render_status_bar(
        f,
        layout.status_bar,
        &state.status,
        state.is_flash_copied_active(),
    );

    let project_name = state
        .selected_project
        .and_then(|idx| state.projects.get(idx))
        .map(|p| p.name.as_str())
        .unwrap_or("Unknown");

    let focused = matches!(state.focus, Focus::RoomList);

    thread_list_v2::render_threads_list(
        f,
        layout.main,
        project_name,
        &state.thread_summaries,
        state.selected_thread,
        &mut state.room_list_state,
        focused,
    );

    if !overlay {
        thread_list_v2::render_thread_sidebar(
            f,
            layout.sidebar,
            &state.thread_summaries,
            state.selected_thread,
        );
    }

    // Input bar at the bottom
    let mention_candidates = state.room_agent_mention_candidates();
    input_bar::render_input_bar(
        f,
        layout.bottom,
        "Threads Input",
        "Type a message to start a new conversation...",
        &state.room_input,
        mention_candidates.as_slice(),
        &mut state.input_metrics[0],
    );
}
```

- [ ] **Step 2: Update `render_ui` dispatch to call `render_threads_level`**

Change the `Threads` arm in `render_ui`:
```rust
        Some(crate::nav::NavLevel::Threads { .. }) => {
            render_threads_level(f, state);
        }
```

- [ ] **Step 3: Add Esc handling for Threads level in `handle_key`**

In `handle_key`, add a block for Threads level (after the Projects block):

```rust
        if let Some(crate::nav::NavLevel::Threads { .. }) = self.ui.nav_stack.current() {
            // Esc returns to Projects
            if key.code == KeyCode::Esc
                && !matches!(self.ui.focus, Focus::RoomInput)
            {
                self.ui.nav_stack.pop();
                return true;
            }
            // Ctrl+Q always quits
            if matches!(key.code, KeyCode::Char('q'))
                && key.modifiers.contains(KeyModifiers::CONTROL)
            {
                self.should_quit = true;
                return true;
            }
        }
```

- [ ] **Step 4: Verify build**

Run: `cargo build -p minos-tui 2>&1 | grep '^error' | head -10`
Expected: clean.

- [ ] **Step 5: Commit**

```bash
git add crates/minos-tui/src/ui/mod.rs crates/minos-tui/src/app.rs
git commit -m "feat(tui): threads list view with input bar and Esc navigation"
```

### Task 4.3: Threads 列表导航 + 从输入创建新 Thread

**Files:**
- Modify: `crates/minos-tui/src/app.rs`

- [ ] **Step 1: Add Threads-level key handling method**

```rust
    async fn handle_threads_list_key(&mut self, key: KeyEvent) -> bool {
        if matches!(self.ui.focus, Focus::RoomInput) {
            return false; // Let input bar handle its own keys
        }
        match key.code {
            KeyCode::Up => {
                self.navigate_threads(-1);
                true
            }
            KeyCode::Down => {
                self.navigate_threads(1);
                true
            }
            KeyCode::Enter => {
                self.open_selected_thread().await;
                true
            }
            _ => false,
        }
    }

    fn navigate_threads(&mut self, delta: i32) {
        if self.ui.thread_summaries.is_empty() {
            return;
        }
        let current = self.ui.selected_thread.unwrap_or(0) as i32;
        let mut next = current + delta;
        if next < 0 {
            next = self.ui.thread_summaries.len() as i32 - 1;
        }
        if next >= self.ui.thread_summaries.len() as i32 {
            next = 0;
        }
        self.ui.selected_thread = Some(next as usize);
    }

    async fn open_selected_thread(&mut self) {
        if let Some(idx) = self.ui.selected_thread {
            if let Some(thread) = self.ui.thread_summaries.get(idx) {
                let project_id = self
                    .ui
                    .nav_stack
                    .current()
                    .and_then(|l| l.project_id())
                    .map(|s| s.to_owned())
                    .unwrap_or_default();
                self.ui.nav_stack.push(crate::nav::NavLevel::Thread {
                    project_id,
                    thread_id: thread.thread_id.clone(),
                });
            }
        }
    }
```

- [ ] **Step 2: Wire threads list key handling into `handle_key`**

Add after the Threads Esc block:

```rust
            let consumed = self.handle_threads_list_key(key).await;
            if consumed {
                return true;
            }
```

- [ ] **Step 3: Handle Enter in the threads input bar to create a new thread**

In `handle_key`, when at Threads level and focus is on RoomInput and Enter is pressed, intercept it before the existing submit logic:

Find where `submit_room_input` is called (or where Enter in room input triggers message send). Add a nav-aware branch. In the existing room input Enter handling:

```rust
            // If at Threads level, Enter creates a new thread instead of sending to room.
            if matches!(self.ui.nav_stack.current(), Some(crate::nav::NavLevel::Threads { .. })) {
                self.create_thread_from_input().await;
                return true;
            }
```

Add the method:
```rust
    async fn create_thread_from_input(&mut self) {
        let text = self.ui.room_input.text.clone();
        if text.trim().is_empty() {
            return;
        }

        // Parse @agent mentions
        let (agent, prompt) = self.parse_thread_creation_input(&text);

        let project_id = self
            .ui
            .nav_stack
            .current()
            .and_then(|l| l.project_id())
            .map(|s| s.to_owned())
            .unwrap_or_default();

        let workspace = self.workspace.clone();
        let backend = self.backend.clone();
        let event_tx = self.event_tx.clone();

        // Clear input
        self.ui.room_input.text.clear();
        self.ui.room_input.cursor_byte = 0;

        tokio::spawn(async move {
            match backend
                .start_agent_in_project(&project_id, agent, workspace, Some(&prompt))
                .await
            {
                Ok(outcome) => {
                    if let Some(tx) = event_tx {
                        let _ = tx.send(AppEvent::AgentStartedForPrompt {
                            agent,
                            thread_id: outcome.thread_id,
                            cwd: outcome.cwd,
                            text: prompt,
                        });
                    }
                }
                Err(e) => {
                    if let Some(tx) = event_tx {
                        let _ = tx.send(AppEvent::SendMessageFailed {
                            thread_id: String::new(),
                            error: e.to_string(),
                        });
                    }
                }
            }
        });
    }

    fn parse_thread_creation_input(&self, text: &str) -> (AgentName, String) {
        // Simple parse: if starts with @agent, extract agent name.
        // Otherwise use default agent (codex).
        if let Some(rest) = text.strip_prefix('@') {
            let (agent_part, remainder) = rest.split_once(' ').unwrap_or((rest, ""));
            let agent = match agent_part.to_lowercase().as_str() {
                "codex" => AgentName::Codex,
                "claude" => AgentName::Claude,
                "gemini" => AgentName::Gemini,
                "opencode" => AgentName::Opencode,
                _ => AgentName::Codex,
            };
            (agent, remainder.trim().to_owned())
        } else {
            (AgentName::Codex, text.to_owned())
        }
    }
```

- [ ] **Step 4: Handle `AgentStartedForPrompt` at Threads level to push Thread nav**

In the `handle_event` method's `AppEvent::AgentStartedForPrompt` arm, add nav-level awareness:

```rust
        AppEvent::AgentStartedForPrompt {
            agent,
            thread_id,
            cwd,
            text,
        } => {
            // ... existing logic to create thread entry, group chat message, etc. ...

            // If at Threads level, push to Thread level
            if matches!(
                self.ui.nav_stack.current(),
                Some(crate::nav::NavLevel::Threads { .. })
            ) {
                let project_id = self
                    .ui
                    .nav_stack
                    .current()
                    .and_then(|l| l.project_id())
                    .map(|s| s.to_owned())
                    .unwrap_or_default();
                self.ui.nav_stack.push(crate::nav::NavLevel::Thread {
                    project_id,
                    thread_id: thread_id.clone(),
                });
            }
            // ... rest of existing handling ...
        }
```

- [ ] **Step 5: Write test for thread creation from input**

```rust
    #[tokio::test]
    async fn threads_input_creates_new_thread() {
        let backend = Arc::new(
            TestBackend::new()
                .with_agents(vec![ok_agent(AgentName::Codex)])
                .with_projects(vec![crate::backend::ProjectEntry {
                    project_id: "p1".into(),
                    name: "Test".into(),
                    workspace_path: PathBuf::from("/tmp"),
                    thread_count: 0,
                    created_at_ms: 0,
                    updated_at_ms: 0,
                }]),
        );
        let mut app = App::new(backend.clone(), false, PathBuf::from("/tmp"));
        app.ui.nav_stack.reset_to(crate::nav::NavLevel::Threads {
            project_id: "p1".into(),
        });
        app.event_tx = {
            let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
            // Drain in background
            tokio::spawn(async move { while rx.recv().await.is_some() {} });
            Some(tx)
        };

        // Type a message
        app.ui.room_input.text = "@codex hello world".into();
        app.ui.focus = crate::ui::Focus::RoomInput;

        // Press Enter
        app.handle_key(press(KeyCode::Enter)).await;

        // Backend should have started agent
        let started = backend.started.lock().unwrap().clone();
        assert_eq!(started, vec![AgentName::Codex]);
    }
```

- [ ] **Step 6: Run all tests**

Run: `cargo test -p minos-tui 2>&1 | tail -15`
Expected: all pass.

- [ ] **Step 7: Commit**

```bash
git add crates/minos-tui/src/app.rs
git commit -m "feat(tui): thread creation from threads input bar with @mention parsing"
```

---

## Phase 5-7: Thread 群聊视图 + Agent 卡片 + Agent 子视图 + 响应式

Phases 5-7 build the Thread group-chat view (80/20 with agent cards), the Agent detail view, and responsive overlay. These follow the same pattern as Phases 3-4:

- **Phase 5:** `render_thread_level` in `ui/mod.rs` + `ui/agent_card.rs` widget. Reuses `group_chat.rs` for chat rendering, new agent_card widget for sidebar. Enter on agent card pushes `NavLevel::Agent`.
- **Phase 6:** `render_agent_level` + sidebar with context/files/duration. Reuses `chat.rs` + `RenderCache`. Filters `ChatState.items` by agent.
- **Phase 7:** `sidebar_should_overlay` integration (already scaffolded in Phase 3), theme expansion, polish.

These phases are deferred to a follow-up plan once Phases 1-4 are validated and merged, to keep each plan focused and reviewable.

---

## Final Integration: Update architecture-tui.md

### Task F.1: Update architecture doc

**Files:**
- Modify: `docs/architecture-tui.md`

- [ ] **Step 1: Update the UI Layout section to document the new nav-level-based rendering**

Replace the "概览模式" and "详情模式" sections with documentation of the Project → Thread → Agent three-level navigation, referencing `NavLevel`, `NavStack`, and the unified 80/20 layout.

Document the startup cwd → project matching logic.

Document the new `AgentBackend` project methods.

- [ ] **Step 2: Commit**

```bash
git add docs/architecture-tui.md
git commit -m "docs: update TUI architecture for three-level navigation"
```
