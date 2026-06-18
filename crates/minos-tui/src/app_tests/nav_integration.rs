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
    assert!(app.ui.project_create_dialog.is_some());
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
