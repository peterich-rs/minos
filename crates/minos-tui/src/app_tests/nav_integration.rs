use super::*;
use crate::backend::{ConversationEntry, ProjectEntry};
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
    set_test_projects_nav(&mut app);
    app.ui.projects.items = projects;
    app.ui.projects.selected = if app.ui.projects.items.is_empty() {
        None
    } else {
        Some(0)
    };
    app.ui.projects.list_state.select(app.ui.projects.selected);
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

fn sample_conversation(project_id: &str, conversation_id: &str, title: &str) -> ConversationEntry {
    ConversationEntry {
        conversation_id: conversation_id.into(),
        project_id: project_id.into(),
        title: title.into(),
        last_message_preview: None,
        created_at_ms: 0,
        updated_at_ms: 0,
        message_count: 0,
        agent_session_count: 0,
        participating_agents: Vec::new(),
    }
}

#[tokio::test]
async fn projects_navigate_down_then_open() {
    let (_, mut app, mut rx) = app_with_projects(vec![
        sample_project("p1", "P1", "/tmp/p1"),
        sample_project("p2", "P2", "/tmp/p2"),
    ]);
    app.handle_key(press(KeyCode::Down)).await;
    assert_eq!(app.ui.projects.selected, Some(1));
    app.handle_key(press(KeyCode::Enter)).await;
    pump_events(&mut app, &mut rx).await;
    assert_eq!(
        app.ui.nav_level(),
        &NavLevel::Conversations {
            project_id: "p2".into()
        }
    );
}

#[tokio::test]
async fn projects_up_wraps_to_last() {
    let (_, mut app, _rx) = app_with_projects(vec![
        sample_project("p1", "P1", "/tmp/p1"),
        sample_project("p2", "P2", "/tmp/p2"),
    ]);
    app.handle_key(press(KeyCode::Up)).await;
    assert_eq!(app.ui.projects.selected, Some(1));
}

#[tokio::test]
async fn esc_at_projects_does_not_quit() {
    let (_, mut app, _rx) = app_with_projects(vec![]);
    app.handle_key(press(KeyCode::Esc)).await;
    assert!(!app.should_quit());
}

#[tokio::test]
async fn ctrl_q_at_projects_quits() {
    let (_, mut app, _rx) = app_with_projects(vec![]);
    app.handle_key(press_with_modifiers(
        KeyCode::Char('q'),
        KeyModifiers::CONTROL,
    ))
    .await;
    assert!(app.should_quit());
}

#[tokio::test]
async fn ctrl_p_from_agent_detail_jumps_to_projects() {
    let backend = Arc::new(TestBackend::new());
    let mut app = App::new(backend, false, PathBuf::from("/tmp"));
    set_test_agent_detail_nav(&mut app, "p1", "c1");

    assert!(
        app.handle_key(press_with_modifiers(
            KeyCode::Char('p'),
            KeyModifiers::CONTROL
        ))
        .await
    );

    assert_eq!(app.ui.nav.stack, vec![NavLevel::Projects]);
}

#[tokio::test]
async fn ctrl_t_from_agent_detail_jumps_to_current_project_conversations() {
    let backend = Arc::new(TestBackend::new());
    let mut app = App::new(backend, false, PathBuf::from("/tmp"));
    set_test_agent_detail_nav(&mut app, "p1", "c1");
    app.ui.conversations.selected = Some(2);

    assert!(
        app.handle_key(press_with_modifiers(
            KeyCode::Char('t'),
            KeyModifiers::CONTROL
        ))
        .await
    );

    assert_eq!(
        app.ui.nav.stack,
        vec![
            NavLevel::Projects,
            NavLevel::Conversations {
                project_id: "p1".into()
            }
        ]
    );
    assert_eq!(app.ui.conversations.selected, Some(2));
}

#[tokio::test]
async fn ctrl_t_at_projects_is_noop() {
    let (_, mut app, _rx) = app_with_projects(vec![]);

    assert!(
        !app.handle_key(press_with_modifiers(
            KeyCode::Char('t'),
            KeyModifiers::CONTROL
        ))
        .await
    );

    assert_eq!(app.ui.nav.stack, vec![NavLevel::Projects]);
}

#[tokio::test]
async fn ctrl_nav_shortcuts_do_not_interrupt_project_create_dialog() {
    let backend = Arc::new(TestBackend::new());
    let mut app = App::new(backend, false, PathBuf::from("/tmp/newproj"));
    set_test_projects_nav(&mut app);
    app.handle_key(press(KeyCode::Char('n'))).await;
    app.handle_key(press_with_modifiers(
        KeyCode::Char('p'),
        KeyModifiers::CONTROL,
    ))
    .await;

    assert!(app.ui.overlays.project_create.is_some());
    assert_eq!(app.ui.nav.stack, vec![NavLevel::Projects]);
}

#[tokio::test]
async fn n_key_at_conversations_no_longer_opens_modal_picker() {
    let backend = Arc::new(TestBackend::new());
    let mut app = App::new(backend, false, PathBuf::from("/tmp"));
    set_test_conversations_nav(&mut app, "p1");

    assert!(!app.handle_key(press(KeyCode::Char('n'))).await);
    assert!(app.ui.overlays.project_create.is_none());
    assert_eq!(
        app.ui.nav.stack,
        vec![
            NavLevel::Projects,
            NavLevel::Conversations {
                project_id: "p1".into()
            }
        ]
    );
}

#[tokio::test]
async fn esc_at_sessions_returns_to_projects() {
    let backend =
        Arc::new(TestBackend::new().with_projects(vec![sample_project("p1", "P1", "/tmp/p1")]));
    let mut app = App::new(backend, false, PathBuf::from("/tmp/p1"));
    set_test_conversations_nav(&mut app, "p1");
    app.handle_key(press(KeyCode::Esc)).await;
    assert_eq!(app.ui.nav_level(), &NavLevel::Projects);
    assert!(!app.should_quit());
}

#[tokio::test]
async fn open_project_dialog_with_n_key() {
    let backend = Arc::new(TestBackend::new());
    let mut app = App::new(backend.clone(), false, PathBuf::from("/tmp/newproj"));
    set_test_projects_nav(&mut app);
    app.handle_key(press(KeyCode::Char('n'))).await;
    let dialog = app
        .ui
        .overlays
        .project_create
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
        .overlays
        .project_create
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
    set_test_projects_nav(&mut app);
    app.handle_key(press(KeyCode::Char('n'))).await;
    app.handle_key(press(KeyCode::Char('M'))).await;
    assert!(app
        .ui
        .overlays
        .project_create
        .as_ref()
        .map(|d| d.name.ends_with('M'))
        .unwrap_or(false));
    app.handle_key(press(KeyCode::Enter)).await;
    pump_events(&mut app, &mut rx).await;
    let created = backend
        .created_projects
        .lock()
        .expect("created projects lock");
    assert!(!created.is_empty());
}

#[tokio::test]
async fn start_new_session_via_input_transitions_to_conversation_level() {
    use crate::focus::PaneId;
    use minos_domain::{AgentDescriptor, AgentName, AgentStatus};

    let project = sample_project("p1", "P1", "/tmp/minos");
    let backend = Arc::new(TestBackend::new().with_projects(vec![project.clone()]));
    let mut app = App::new(backend.clone(), false, PathBuf::from("/tmp/fire"));
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    app.set_event_sender(tx);
    set_test_conversations_nav(&mut app, "p1");
    app.ui.projects.items = vec![project];
    app.ui.status.update_agents(vec![AgentDescriptor::new(
        AgentName::Codex,
        None,
        None,
        AgentStatus::Ok,
    )]);
    app.ui.focus.focus(PaneId::Input);
    app.ui.inputs.conversation.content = "hello world".to_owned();

    app.handle_key(press(KeyCode::Enter)).await;
    pump_events(&mut app, &mut rx).await;

    let conversation_id = app
        .ui
        .nav_level()
        .conversation_id()
        .map(str::to_owned)
        .unwrap_or_default();
    assert_eq!(
        app.ui.nav_level(),
        &NavLevel::Conversation {
            project_id: "p1".into(),
            conversation_id,
        }
    );
    assert!(
        app.ui
            .conversation
            .agent_sessions
            .items
            .iter()
            .any(|s| s.agent == AgentName::Codex),
        "new session must appear in conversation.agent_sessions.items"
    );
    assert!(
        app.ui.inputs.conversation.content.is_empty(),
        "input must be cleared"
    );
    assert_eq!(
        backend
            .started_workspaces
            .lock()
            .expect("started workspaces lock")
            .as_slice(),
        &[PathBuf::from("/tmp/minos")],
        "agent must start in selected project workspace, not the TUI launch workspace"
    );
}

#[tokio::test]
async fn session_input_accepts_agent_completion_before_submit() {
    use crate::focus::PaneId;

    let project = sample_project("p1", "P1", "/tmp/p1");
    let backend = Arc::new(TestBackend::new().with_projects(vec![project]));
    let mut app = App::new(backend.clone(), false, PathBuf::from("/tmp/p1"));
    set_test_conversations_nav(&mut app, "p1");
    app.ui.status.update_agents(vec![
        ok_agent(AgentName::Codex),
        ok_agent(AgentName::Claude),
    ]);
    app.ui.focus.focus(PaneId::Input);

    assert!(app.handle_key(press(KeyCode::Char('@'))).await);
    assert!(app.handle_key(press(KeyCode::Char('c'))).await);
    assert!(app.ui.inputs.conversation.has_agent_picker());
    assert!(app.handle_key(press(KeyCode::Down)).await);
    assert!(app.handle_key(press(KeyCode::Enter)).await);

    assert_eq!(app.ui.inputs.conversation.content, "@claude ");
    assert_eq!(app.ui.inputs.conversation.cursor_pos, "@claude ".len());
    assert!(!app.ui.inputs.conversation.has_agent_picker());
    assert_eq!(
        app.ui.nav_level(),
        &NavLevel::Conversations {
            project_id: "p1".into()
        }
    );
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
async fn open_existing_session_bridges_into_thread_list() {
    use crate::backend::SessionSummaryEntry;

    let project = sample_project("p1", "P1", "/tmp/p1");
    let conversation = sample_conversation("p1", "c1", "conversation");
    let session = SessionSummaryEntry {
        session_id: "existing-session-1".into(),
        agent: minos_domain::AgentName::Codex,
        title: Some("an existing session".into()),
        first_ts_ms: 0,
        last_ts_ms: 0,
        message_count: 0,
        ended_at_ms: None,
        parent_session_id: None,
        state: minos_agent_runtime::SessionState::Idle,
        needs_continue: false,
    };
    let backend = Arc::new(
        TestBackend::new()
            .with_projects(vec![project.clone()])
            .with_conversations(vec![conversation.clone()])
            .with_conversation_sessions("c1", vec![session.clone()]),
    );
    let mut app = App::new(backend, false, PathBuf::from("/tmp/p1"));
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    app.set_event_sender(tx);
    set_test_conversations_nav(&mut app, "p1");
    app.ui.projects.items = vec![project.clone()];
    app.ui.projects.selected = Some(0);
    app.ui.conversations.items = vec![conversation];
    app.ui.conversations.selected = Some(0);
    app.ui.conversations.list_state.select(Some(0));

    app.handle_key(press(KeyCode::Enter)).await;
    pump_events(&mut app, &mut rx).await;

    assert_eq!(
        app.ui.nav_level(),
        &NavLevel::Conversation {
            project_id: "p1".into(),
            conversation_id: "c1".into(),
        }
    );
    assert_eq!(app.ui.conversation.agent_sessions.items, vec![session]);
    app.handle_key(press(KeyCode::Enter)).await;
    assert!(
        app.ui
            .session_panel.list.items
            .iter()
            .any(|t| t.session_id == "existing-session-1"),
        "ensure_conversation_agent_session_visible must bridge the session into ui.session_panel.list.items"
    );
    let bridged = app
        .ui
        .session_panel
        .list
        .items
        .iter()
        .find(|t| t.session_id == "existing-session-1")
        .expect("bridged thread exists");
    assert_eq!(bridged.workspace, project.workspace_path);
    assert!(
        app.ui
            .session_panel
            .chat_states
            .contains_key("existing-session-1"),
        "bridged conversation session must create chat state before hydration"
    );
}
