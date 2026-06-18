use super::*;

#[tokio::test]
async fn ctrl_c_interrupts_running_thread() {
    let backend = Arc::new(TestBackend::new());
    let mut app = App::new(backend.clone(), false, PathBuf::from("/tmp"));
    set_test_conversation_nav(&mut app, "test", "conversation-1");
    app.ui.threads.push(ThreadEntry {
        thread_id: "thread-1".into(),
        agent: AgentName::Codex,
        workspace: PathBuf::from("/tmp"),
        state: ThreadState::Running {
            turn_started_at_ms: 0,
        },
    });
    app.select_thread(0);

    let key = KeyEvent {
        code: KeyCode::Char('c'),
        modifiers: KeyModifiers::CONTROL,
        kind: KeyEventKind::Press,
        state: KeyEventState::NONE,
    };
    let redraw = app.handle_key(key).await;

    assert!(redraw);
    assert!(!app.should_quit());
    assert_eq!(
        backend
            .interrupted
            .lock()
            .expect("interrupt list lock")
            .as_slice(),
        &["thread-1".to_owned()]
    );
}

#[tokio::test]
async fn ctrl_c_quits_idle_thread_view() {
    let backend = Arc::new(TestBackend::new());
    let mut app = App::new(backend.clone(), false, PathBuf::from("/tmp"));
    set_test_conversation_nav(&mut app, "test", "conversation-1");
    app.ui.threads.push(ThreadEntry {
        thread_id: "thread-1".into(),
        agent: AgentName::Gemini,
        workspace: PathBuf::from("/tmp"),
        state: ThreadState::Idle,
    });
    app.select_thread(0);

    let key = KeyEvent {
        code: KeyCode::Char('c'),
        modifiers: KeyModifiers::CONTROL,
        kind: KeyEventKind::Press,
        state: KeyEventState::NONE,
    };
    let redraw = app.handle_key(key).await;

    assert!(!redraw);
    assert!(app.should_quit());
    assert!(backend
        .interrupted
        .lock()
        .expect("interrupt list lock")
        .is_empty());
}

#[tokio::test]
async fn ctrl_v_pastes_from_clipboard() {
    let backend = Arc::new(TestBackend::new());
    let mut app = App::new(backend.clone(), false, PathBuf::from("/tmp"));
    set_test_conversation_nav(&mut app, "test", "conversation-1");
    set_test_agent_detail_nav(&mut app, "test", "conversation-1");
    app.ui.focus.switch_layout(true);
    app.ui.focus.focus(PaneId::Input);

    super::TEST_CLIPBOARD
        .lock()
        .expect("test clipboard lock")
        .push("hello from clipboard".to_owned());
    let key = press_with_modifiers(KeyCode::Char('v'), KeyModifiers::CONTROL);
    let redraw = app.handle_key(key).await;

    assert!(redraw);
    assert_eq!(app.ui.agent_input.content, "hello from clipboard");
    super::TEST_CLIPBOARD
        .lock()
        .expect("test clipboard lock")
        .clear();
}

#[tokio::test]
async fn open_agent_picker_defaults_to_current_thread_agent() {
    let backend = Arc::new(TestBackend::with_agents(vec![
        ok_agent(AgentName::Codex),
        ok_agent(AgentName::Claude),
        ok_agent(AgentName::Gemini),
    ]));
    let mut app = App::new(backend, false, PathBuf::from("/tmp"));
    app.ui.status.update_agents(vec![
        ok_agent(AgentName::Codex),
        ok_agent(AgentName::Claude),
        ok_agent(AgentName::Gemini),
    ]);
    app.ui.threads.push(ThreadEntry {
        thread_id: "thread-claude".into(),
        agent: AgentName::Claude,
        workspace: PathBuf::from("/tmp"),
        state: ThreadState::Idle,
    });
    app.select_thread(0);

    assert!(
        app.apply_action(Action::Global(GlobalAction::OpenAgentPicker))
            .await
    );
    assert_eq!(
        app.ui.agent_picker.as_ref().map(|picker| picker.selected),
        Some(1)
    );
}

#[tokio::test]
async fn at_completion_inserts_selected_agent() {
    let backend = Arc::new(TestBackend::with_agents(vec![
        ok_agent(AgentName::Codex),
        ok_agent(AgentName::Claude),
        ok_agent(AgentName::Gemini),
    ]));
    let mut app = App::new(backend, false, PathBuf::from("/tmp"));
    set_test_conversation_nav(&mut app, "test", "conversation-1");
    app.ui.status.update_agents(vec![
        ok_agent(AgentName::Codex),
        ok_agent(AgentName::Claude),
        ok_agent(AgentName::Gemini),
    ]);
    app.ui.focus.focus(PaneId::Input);
    app.sync_input_agent_picker();

    assert!(app.handle_key(press(KeyCode::Char('@'))).await);
    assert!(app.handle_key(press(KeyCode::Char('c'))).await);
    assert!(app.handle_key(press(KeyCode::Down)).await);
    assert!(app.handle_key(press(KeyCode::Enter)).await);

    assert_eq!(app.ui.room_input.content, "@claude ");
    assert_eq!(app.ui.room_input.cursor_pos, "@claude ".len());
    assert!(!app.ui.room_input.has_agent_picker());
}

#[tokio::test]
async fn input_shortcuts_edit_without_inserting_control_text() {
    let backend = Arc::new(TestBackend::with_agents(vec![ok_agent(AgentName::Codex)]));
    let mut app = App::new(backend, false, PathBuf::from("/tmp"));
    set_test_conversation_nav(&mut app, "test", "conversation-1");
    app.ui
        .status
        .update_agents(vec![ok_agent(AgentName::Codex)]);
    app.ui.focus.focus(PaneId::Input);
    app.ui.room_input.content = "hello brave world".into();
    app.ui.room_input.cursor_pos = app.ui.room_input.content.len();

    assert!(
        app.handle_key(press_with_modifiers(
            KeyCode::Char('w'),
            KeyModifiers::CONTROL
        ))
        .await
    );
    assert_eq!(app.ui.room_input.content, "hello brave ");
    assert_eq!(app.ui.room_input.cursor_pos, "hello brave ".len());

    assert!(
        app.handle_key(press_with_modifiers(KeyCode::Char('b'), KeyModifiers::ALT))
            .await
    );
    assert_eq!(app.ui.room_input.cursor_pos, "hello ".len());

    assert!(app.handle_key(press(KeyCode::Right)).await);
    assert_eq!(app.ui.room_input.cursor_pos, "hello b".len());

    assert!(
        app.handle_key(press_with_modifiers(
            KeyCode::Char('a'),
            KeyModifiers::CONTROL
        ))
        .await
    );
    assert_eq!(app.ui.room_input.cursor_pos, 0);
    assert_eq!(app.ui.room_input.content, "hello brave ");
}

#[tokio::test]
async fn room_input_paste_inserts_multiline_text_without_submitting() {
    let temp = tempfile::tempdir().expect("tempdir");
    let group_store = GroupChatStore::at_path(temp.path().join("group.jsonl"));
    let backend = Arc::new(TestBackend::with_agents(vec![ok_agent(AgentName::Codex)]));
    let mut app =
        App::with_group_chat_store(backend.clone(), false, PathBuf::from("/tmp"), group_store);
    set_test_conversation_nav(&mut app, "test", "conversation-1");
    app.ui
        .status
        .update_agents(vec![ok_agent(AgentName::Codex)]);
    app.ui.threads.push(ThreadEntry {
        thread_id: "thread-codex-1234".into(),
        agent: AgentName::Codex,
        workspace: PathBuf::from("/tmp"),
        state: ThreadState::Idle,
    });
    app.ui.chat_states.insert(
        "thread-codex-1234".into(),
        ChatState::new("thread-codex-1234".into(), AgentName::Codex),
    );
    app.select_thread(0);
    app.ui.focus.focus(PaneId::Input);

    assert!(
        app.handle_event(AppEvent::Paste("first\r\nsecond\nthird".into()))
            .await
    );

    assert_eq!(app.ui.room_input.content, "first\nsecond\nthird");
    assert_eq!(app.ui.room_input.cursor_pos, "first\nsecond\nthird".len());
    assert!(backend
        .sent_messages
        .lock()
        .expect("sent messages lock")
        .is_empty());

    assert!(app.handle_key(press(KeyCode::Enter)).await);

    assert_eq!(
        backend
            .sent_messages
            .lock()
            .expect("sent messages lock")
            .as_slice(),
        &[("thread-1".to_owned(), "first\nsecond\nthird".to_owned())]
    );
    assert_eq!(app.ui.conversation_messages.len(), 1);
    assert_eq!(app.ui.conversation_messages[0].body, "first\nsecond\nthird");
}

#[tokio::test]
async fn routed_prompt_starts_target_agent_and_sends_body_only() {
    let backend = Arc::new(TestBackend::with_agents(vec![
        ok_agent(AgentName::Codex),
        ok_agent(AgentName::Claude),
        ok_agent(AgentName::Gemini),
    ]));
    let mut app = App::new(backend.clone(), false, PathBuf::from("/tmp"));
    set_test_conversation_nav(&mut app, "test", "conversation-1");
    app.ui.status.update_agents(vec![
        ok_agent(AgentName::Codex),
        ok_agent(AgentName::Claude),
        ok_agent(AgentName::Gemini),
    ]);
    app.ui.focus.focus(PaneId::Input);
    app.ui.room_input.content = "@gemini write tests".into();
    app.ui.room_input.cursor_pos = app.ui.room_input.content.len();
    app.sync_input_agent_picker();

    assert!(app.handle_key(press(KeyCode::Enter)).await);

    assert_eq!(
        backend
            .started
            .lock()
            .expect("started list lock")
            .as_slice(),
        &[AgentName::Gemini]
    );
    assert_eq!(
        backend
            .sent_messages
            .lock()
            .expect("sent messages lock")
            .as_slice(),
        &[("thread-1".to_owned(), "write tests".to_owned())]
    );
    assert_eq!(app.ui.room_input.content, "");
    assert_eq!(app.ui.selected_thread, Some(0));
    assert_eq!(app.ui.threads[0].agent, AgentName::Gemini);
}

#[tokio::test]
async fn room_input_on_closed_selected_thread_starts_new_same_agent() {
    let backend = Arc::new(TestBackend::with_agents(vec![ok_agent(
        AgentName::Opencode,
    )]));
    let mut app = App::new(backend.clone(), false, PathBuf::from("/tmp"));
    set_test_conversation_nav(&mut app, "test", "conversation-1");
    app.ui
        .status
        .update_agents(vec![ok_agent(AgentName::Opencode)]);
    app.ui.threads.push(ThreadEntry {
        thread_id: "thread-opencode-closed".into(),
        agent: AgentName::Opencode,
        workspace: PathBuf::from("/tmp"),
        state: ThreadState::Closed {
            reason: minos_agent_runtime::CloseReason::UserClose,
        },
    });
    app.select_thread(0);
    app.ui.focus.focus(PaneId::Input);
    app.ui.room_input.content = "are you there?".into();
    app.ui.room_input.cursor_pos = app.ui.room_input.content.len();

    assert!(app.handle_key(press(KeyCode::Enter)).await);

    assert_eq!(
        backend
            .started
            .lock()
            .expect("started list lock")
            .as_slice(),
        &[AgentName::Opencode]
    );
    assert_eq!(
        backend
            .sent_messages
            .lock()
            .expect("sent messages lock")
            .as_slice(),
        &[("thread-1".to_owned(), "are you there?".to_owned())]
    );
    assert_eq!(app.ui.room_input.content, "");
    assert_eq!(app.ui.selected_thread, Some(1));
    assert_eq!(app.ui.threads[1].agent, AgentName::Opencode);
}

#[tokio::test]
async fn agent_input_on_closed_selected_thread_starts_new_same_agent() {
    let backend = Arc::new(TestBackend::with_agents(vec![ok_agent(
        AgentName::Opencode,
    )]));
    let mut app = App::new(backend.clone(), false, PathBuf::from("/tmp"));
    set_test_conversation_nav(&mut app, "test", "conversation-1");
    app.ui
        .status
        .update_agents(vec![ok_agent(AgentName::Opencode)]);
    app.ui.threads.push(ThreadEntry {
        thread_id: "thread-opencode-closed".into(),
        agent: AgentName::Opencode,
        workspace: PathBuf::from("/tmp"),
        state: ThreadState::Closed {
            reason: minos_agent_runtime::CloseReason::UserClose,
        },
    });
    app.select_thread(0);
    set_test_agent_detail_nav(&mut app, "test", "conversation-1");
    app.ui.focus.switch_layout(true);
    app.ui.focus.focus(PaneId::Input);
    app.ui.agent_input.content = "continue".into();
    app.ui.agent_input.cursor_pos = app.ui.agent_input.content.len();

    assert!(app.handle_key(press(KeyCode::Enter)).await);

    assert_eq!(
        backend
            .started
            .lock()
            .expect("started list lock")
            .as_slice(),
        &[AgentName::Opencode]
    );
    assert_eq!(
        backend
            .sent_messages
            .lock()
            .expect("sent messages lock")
            .as_slice(),
        &[("thread-1".to_owned(), "continue".to_owned())]
    );
    assert_eq!(app.ui.agent_input.content, "");
    assert_eq!(app.ui.selected_thread, Some(1));
    assert_eq!(app.ui.threads[1].agent, AgentName::Opencode);
}

#[tokio::test]
async fn routed_prompt_to_closed_thread_reports_error_without_sending() {
    let backend = Arc::new(TestBackend::with_agents(vec![ok_agent(
        AgentName::Opencode,
    )]));
    let mut app = App::new(backend.clone(), false, PathBuf::from("/tmp"));
    set_test_conversation_nav(&mut app, "test", "conversation-1");
    app.ui
        .status
        .update_agents(vec![ok_agent(AgentName::Opencode)]);
    app.ui.threads.push(ThreadEntry {
        thread_id: "thread-opencode-closed".into(),
        agent: AgentName::Opencode,
        workspace: PathBuf::from("/tmp"),
        state: ThreadState::Closed {
            reason: minos_agent_runtime::CloseReason::UserClose,
        },
    });
    app.ui.focus.focus(PaneId::Input);
    app.ui.room_input.content = "@opencode#thread-o hello".into();
    app.ui.room_input.cursor_pos = app.ui.room_input.content.len();
    app.sync_input_agent_picker();

    assert!(app.handle_key(press(KeyCode::Enter)).await);

    assert!(backend
        .started
        .lock()
        .expect("started list lock")
        .is_empty());
    assert!(backend
        .sent_messages
        .lock()
        .expect("sent messages lock")
        .is_empty());
    let error = app
        .ui
        .error_flash
        .as_ref()
        .map(|(message, _)| message.as_str())
        .unwrap_or("");
    assert!(error.contains("session #thread-o is closed"));
}
