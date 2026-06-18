use super::*;
use crate::backend::ProjectEntry;
use crate::nav::NavLevel;
use std::path::PathBuf;
use std::time::Duration;
use tokio::sync::mpsc::UnboundedReceiver;

fn app_with_projects(
    projects: Vec<ProjectEntry>,
) -> (Arc<TestBackend>, App, UnboundedReceiver<AppEvent>) {
    let backend = Arc::new(TestBackend::new().with_projects(projects.clone()));
    let mut app = App::new(backend.clone(), false, PathBuf::from("/tmp/test"));
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    app.set_event_sender(tx);
    app.ui.nav_level = NavLevel::Projects;
    app.ui.projects = projects;
    app.ui.selected_project = if app.ui.projects.is_empty() {
        None
    } else {
        Some(0)
    };
    app.ui.project_list_state.select(app.ui.selected_project);
    (backend, app, rx)
}

async fn pump_events(app: &mut App, rx: &mut UnboundedReceiver<AppEvent>) {
    for _ in 0..8 {
        match tokio::time::timeout(Duration::from_millis(100), rx.recv()).await {
            Ok(Some(event)) => {
                app.handle_event(event).await;
            }
            _ => break,
        }
    }
}

fn sample_project(id: &str, name: &str, path: &str) -> ProjectEntry {
    ProjectEntry {
        project_id: id.into(),
        name: name.into(),
        workspace_path: PathBuf::from(path),
        thread_count: 0,
        created_at_ms: 0,
        updated_at_ms: 0,
    }
}

#[tokio::test]
async fn projects_navigate_down_then_open() {
    let (_, mut app, mut rx) = app_with_projects(vec![
        sample_project("p1", "P1", "/tmp/p1"),
        sample_project("p2", "P2", "/tmp/p2"),
    ]);
    app.handle_key(press(KeyCode::Down)).await;
    assert_eq!(app.ui.selected_project, Some(1));
    app.handle_key(press(KeyCode::Enter)).await;
    pump_events(&mut app, &mut rx).await;
    assert_eq!(
        app.ui.nav_level,
        NavLevel::Sessions { project_id: "p2".into() }
    );
}

#[tokio::test]
async fn projects_up_wraps_to_last() {
    let (_, mut app, _rx) = app_with_projects(vec![
        sample_project("p1", "P1", "/tmp/p1"),
        sample_project("p2", "P2", "/tmp/p2"),
    ]);
    app.handle_key(press(KeyCode::Up)).await;
    assert_eq!(app.ui.selected_project, Some(1));
}

#[tokio::test]
async fn esc_at_projects_quits() {
    let (_, mut app, _rx) = app_with_projects(vec![]);
    app.handle_key(press(KeyCode::Esc)).await;
    assert!(app.should_quit());
}

#[tokio::test]
async fn ctrl_q_at_projects_quits() {
    let (_, mut app, _rx) = app_with_projects(vec![]);
    app.handle_key(press_with_modifiers(KeyCode::Char('q'), KeyModifiers::CONTROL))
        .await;
    assert!(app.should_quit());
}

#[tokio::test]
async fn esc_at_sessions_returns_to_projects() {
    let backend = Arc::new(
        TestBackend::new().with_projects(vec![sample_project("p1", "P1", "/tmp/p1")]),
    );
    let mut app = App::new(backend, false, PathBuf::from("/tmp/p1"));
    app.ui.nav_level = NavLevel::Sessions { project_id: "p1".into() };
    app.handle_key(press(KeyCode::Esc)).await;
    assert_eq!(app.ui.nav_level, NavLevel::Projects);
    assert!(!app.should_quit());
}

#[tokio::test]
async fn open_project_dialog_with_n_key() {
    let backend = Arc::new(TestBackend::new());
    let mut app = App::new(backend.clone(), false, PathBuf::from("/tmp/newproj"));
    app.ui.nav_level = NavLevel::Projects;
    app.handle_key(press(KeyCode::Char('n'))).await;
    let dialog = app
        .ui
        .project_create_dialog
        .as_ref()
        .expect("project dialog opens");
    assert_eq!(dialog.name, "newproj");
    assert_eq!(dialog.path, "/tmp/newproj");
}

#[tokio::test]
async fn init_opens_project_dialog_for_unmatched_workspace() {
    let backend = Arc::new(TestBackend::new());
    let mut app = App::new(backend, false, PathBuf::from("/tmp/fire"));

    app.init().await.expect("app initializes");

    let dialog = app
        .ui
        .project_create_dialog
        .as_ref()
        .expect("startup opens project create dialog");
    assert_eq!(dialog.name, "fire");
    assert_eq!(dialog.path, "/tmp/fire");
}

#[tokio::test]
async fn create_project_dialog_types_and_confirms() {
    let backend = Arc::new(TestBackend::new());
    let mut app = App::new(backend.clone(), false, PathBuf::from("/tmp/newproj"));
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    app.set_event_sender(tx);
    app.ui.nav_level = NavLevel::Projects;
    app.handle_key(press(KeyCode::Char('n'))).await;
    app.handle_key(press(KeyCode::Char('M'))).await;
    assert!(
        app.ui
            .project_create_dialog
            .as_ref()
            .map(|d| d.name.ends_with('M'))
            .unwrap_or(false)
    );
    app.handle_key(press(KeyCode::Enter)).await;
    pump_events(&mut app, &mut rx).await;
    let created = backend
        .created_projects
        .lock()
        .expect("created projects lock");
    assert!(!created.is_empty());
}

#[tokio::test]
async fn start_new_session_via_input_transitions_to_session_level() {
    use crate::focus::PaneId;
    use minos_domain::{AgentDescriptor, AgentName, AgentStatus};

    let project = sample_project("p1", "P1", "/tmp/p1");
    let backend = Arc::new(TestBackend::new().with_projects(vec![project]));
    let mut app = App::new(backend.clone(), false, PathBuf::from("/tmp/p1"));
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    app.set_event_sender(tx);
    app.ui.nav_level = NavLevel::Sessions { project_id: "p1".into() };
    app.ui.status.update_agents(vec![AgentDescriptor {
        name: AgentName::Codex,
        path: None,
        version: None,
        status: AgentStatus::Ok,
    }]);
    app.ui.focus.focus(PaneId::RoomInput);
    app.ui.room_input.content = "hello world".to_owned();

    app.handle_key(press(KeyCode::Enter)).await;
    pump_events(&mut app, &mut rx).await;

    assert_eq!(
        app.ui.nav_level,
        NavLevel::Session {
            project_id: "p1".into(),
            thread_id: app
                .ui
                .nav_level
                .thread_id()
                .map(str::to_owned)
                .unwrap_or_default(),
        }
    );
    let thread_id = app.ui.nav_level.thread_id().unwrap().to_owned();
    assert!(
        app.ui
            .project_sessions
            .iter()
            .any(|s| s.thread_id == thread_id),
        "new session must appear in project_sessions"
    );
    assert!(app.ui.room_input.content.is_empty(), "input must be cleared");
}

#[tokio::test]
async fn session_input_accepts_agent_completion_before_submit() {
    use crate::focus::PaneId;

    let project = sample_project("p1", "P1", "/tmp/p1");
    let backend = Arc::new(TestBackend::new().with_projects(vec![project]));
    let mut app = App::new(backend.clone(), false, PathBuf::from("/tmp/p1"));
    app.ui.nav_level = NavLevel::Sessions { project_id: "p1".into() };
    app.ui
        .status
        .update_agents(vec![ok_agent(AgentName::Codex), ok_agent(AgentName::Claude)]);
    app.ui.focus.focus(PaneId::RoomInput);

    assert!(app.handle_key(press(KeyCode::Char('@'))).await);
    assert!(app.handle_key(press(KeyCode::Char('c'))).await);
    assert!(app.ui.room_input.has_agent_picker());
    assert!(app.handle_key(press(KeyCode::Down)).await);
    assert!(app.handle_key(press(KeyCode::Enter)).await);

    assert_eq!(app.ui.room_input.content, "@claude ");
    assert_eq!(app.ui.room_input.cursor_pos, "@claude ".len());
    assert!(!app.ui.room_input.has_agent_picker());
    assert_eq!(app.ui.nav_level, NavLevel::Sessions { project_id: "p1".into() });
    assert!(
        backend
            .started
            .lock()
            .expect("started list lock")
            .is_empty(),
        "accepting a mention must not create a session"
    );
}

#[tokio::test]
async fn open_existing_session_bridges_into_legacy_thread_list() {
    use crate::backend::ThreadSummaryEntry;

    let project = sample_project("p1", "P1", "/tmp/p1");
    let backend = Arc::new(TestBackend::new().with_projects(vec![project.clone()]));
    let mut app = App::new(backend, false, PathBuf::from("/tmp/p1"));
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    app.set_event_sender(tx);
    app.ui.nav_level = NavLevel::Sessions { project_id: "p1".into() };
    app.ui.projects = vec![project.clone()];
    app.ui.selected_project = Some(0);
    app.ui.project_sessions = vec![ThreadSummaryEntry {
        thread_id: "existing-session-1".into(),
        agent: minos_domain::AgentName::Codex,
        title: Some("an existing session".into()),
        first_ts_ms: 0,
        last_ts_ms: 0,
        message_count: 0,
        ended_at_ms: None,
    }];
    app.ui.selected_thread = Some(0);
    app.ui.room_list_state.select(Some(0));

    app.handle_key(press(KeyCode::Enter)).await;
    pump_events(&mut app, &mut rx).await;

    assert_eq!(
        app.ui.nav_level,
        NavLevel::Session {
            project_id: "p1".into(),
            thread_id: "existing-session-1".into(),
        }
    );
    assert!(
        app.ui
            .threads
            .iter()
            .any(|t| t.thread_id == "existing-session-1"),
        "ensure_project_session_visible must bridge the session into ui.threads"
    );
    let bridged = app
        .ui
        .threads
        .iter()
        .find(|t| t.thread_id == "existing-session-1")
        .expect("bridged thread exists");
    assert_eq!(bridged.workspace, project.workspace_path);
    assert!(
        app.ui.chat_states.contains_key("existing-session-1"),
        "bridged project session must create chat state before hydration"
    );
}
