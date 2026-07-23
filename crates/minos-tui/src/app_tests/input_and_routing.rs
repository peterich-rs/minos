use super::*;

#[tokio::test]
async fn ctrl_c_interrupts_running_thread() {
    let backend = Arc::new(TestBackend::new());
    let mut app = App::new(backend.clone(), false, PathBuf::from("/tmp"));
    set_test_conversation_nav(&mut app, "test", "conversation-1");
    app.ui.session_panel.list.items.push(SessionEntry {
        session_id: "thread-1".into(),
        agent: AgentName::Codex,
        workspace: PathBuf::from("/tmp"),
        state: SessionState::Running {
            turn_started_at_ms: 0,
        },
        parent_session_id: None,
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
    app.ui.session_panel.list.items.push(SessionEntry {
        session_id: "thread-1".into(),
        agent: AgentName::Gemini,
        workspace: PathBuf::from("/tmp"),
        state: SessionState::Idle,
        parent_session_id: None,
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
    let _clipboard_guard = super::TEST_CLIPBOARD_LOCK.lock().await;
    let backend = Arc::new(TestBackend::new());
    let mut app = App::new(backend.clone(), false, PathBuf::from("/tmp"));
    set_test_conversation_nav(&mut app, "test", "conversation-1");
    set_test_agent_detail_nav(&mut app, "test", "conversation-1");
    app.ui.focus.switch_layout(true);
    app.ui.focus.focus(PaneId::Input);

    {
        let mut clip = super::TEST_CLIPBOARD.lock().expect("test clipboard lock");
        clip.clear();
        clip.push("hello from clipboard".to_owned());
    }
    let key = press_with_modifiers(KeyCode::Char('v'), KeyModifiers::CONTROL);
    let redraw = app.handle_key(key).await;

    assert!(redraw);
    assert_eq!(app.ui.inputs.agent.content, "hello from clipboard");
    super::TEST_CLIPBOARD
        .lock()
        .expect("test clipboard lock")
        .clear();
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

    assert_eq!(app.ui.inputs.conversation.content, "@claude ");
    assert_eq!(app.ui.inputs.conversation.cursor_pos, "@claude ".len());
    assert!(!app.ui.inputs.conversation.has_agent_picker());
}

#[test]
fn session_short_id_in_conversation_resolves_conversation_session_only() {
    let backend = Arc::new(TestBackend::new());
    let mut app = App::new(backend, false, PathBuf::from("/tmp"));
    set_test_conversation_nav(&mut app, "test", "conversation-1");
    app.ui.session_panel.list.items.push(SessionEntry {
        session_id: "global-opencode-1234".into(),
        agent: AgentName::Opencode,
        workspace: PathBuf::from("/tmp"),
        state: SessionState::Idle,
        parent_session_id: None,
    });
    app.ui.conversation.agent_sessions.items = vec![
        crate::backend::SessionSummaryEntry {
            session_id: "conv-opencode-5678".into(),
            agent: AgentName::Opencode,
            title: None,
            first_ts_ms: 0,
            last_ts_ms: 0,
            message_count: 0,
            ended_at_ms: None,
            parent_session_id: None,
            state: SessionState::Idle,
            needs_continue: false,
        },
        crate::backend::SessionSummaryEntry {
            session_id: "conv-closed-9999".into(),
            agent: AgentName::Opencode,
            title: None,
            first_ts_ms: 0,
            last_ts_ms: 0,
            message_count: 0,
            ended_at_ms: None,
            parent_session_id: None,
            state: SessionState::Closed {
                reason: minos_agent_runtime::CloseReason::UserClose,
            },
            needs_continue: false,
        },
    ];

    assert_eq!(
        app.session_id_for_agent_short_id(AgentName::Opencode, "conv-ope"),
        Some("conv-opencode-5678".into())
    );
    assert_eq!(
        app.session_id_for_agent_short_id(AgentName::Opencode, "global-o"),
        None
    );
    assert_eq!(
        app.session_id_for_agent_short_id(AgentName::Opencode, "conv-clo"),
        None
    );
}

#[test]
fn session_short_id_outside_conversation_does_not_resolve_global_threads() {
    let backend = Arc::new(TestBackend::new());
    let mut app = App::new(backend, false, PathBuf::from("/tmp"));
    set_test_conversations_nav(&mut app, "test");
    app.ui.session_panel.list.items.push(SessionEntry {
        session_id: "global-codex-1234".into(),
        agent: AgentName::Codex,
        workspace: PathBuf::from("/tmp"),
        state: SessionState::Idle,
        parent_session_id: None,
    });
    app.ui.conversation.agent_sessions.items = vec![crate::backend::SessionSummaryEntry {
        session_id: "conv-codex-5678".into(),
        agent: AgentName::Codex,
        title: None,
        first_ts_ms: 0,
        last_ts_ms: 0,
        message_count: 0,
        ended_at_ms: None,
        parent_session_id: None,
        state: SessionState::Idle,
        needs_continue: false,
    }];

    assert_eq!(
        app.session_id_for_agent_short_id(AgentName::Codex, "global-"),
        None
    );
}

#[tokio::test]
async fn conversations_input_does_not_route_stale_session_short_id() {
    let backend = Arc::new(TestBackend::with_agents(vec![ok_agent(
        AgentName::Opencode,
    )]));
    let mut app = App::new(backend.clone(), false, PathBuf::from("/tmp"));
    set_test_conversations_nav(&mut app, "test");
    app.ui
        .status
        .update_agents(vec![ok_agent(AgentName::Opencode)]);
    app.ui.conversation.agent_sessions.items = vec![crate::backend::SessionSummaryEntry {
        session_id: "conv-opencode-5678".into(),
        agent: AgentName::Opencode,
        title: None,
        first_ts_ms: 0,
        last_ts_ms: 0,
        message_count: 0,
        ended_at_ms: None,
        parent_session_id: None,
        state: SessionState::Idle,
        needs_continue: false,
    }];
    app.ui.focus.focus(PaneId::Input);
    app.ui.inputs.conversation.content = "@opencode#conv-ope hello".into();
    app.ui.inputs.conversation.cursor_pos = app.ui.inputs.conversation.content.len();

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
    assert!(error.contains("No existing opencode session matches #conv-ope"));
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
    app.ui.inputs.conversation.content = "hello brave world".into();
    app.ui.inputs.conversation.cursor_pos = app.ui.inputs.conversation.content.len();

    assert!(
        app.handle_key(press_with_modifiers(
            KeyCode::Char('w'),
            KeyModifiers::CONTROL
        ))
        .await
    );
    assert_eq!(app.ui.inputs.conversation.content, "hello brave ");
    assert_eq!(app.ui.inputs.conversation.cursor_pos, "hello brave ".len());

    assert!(
        app.handle_key(press_with_modifiers(KeyCode::Char('b'), KeyModifiers::ALT))
            .await
    );
    assert_eq!(app.ui.inputs.conversation.cursor_pos, "hello ".len());

    assert!(app.handle_key(press(KeyCode::Right)).await);
    assert_eq!(app.ui.inputs.conversation.cursor_pos, "hello b".len());

    assert!(
        app.handle_key(press_with_modifiers(
            KeyCode::Char('a'),
            KeyModifiers::CONTROL
        ))
        .await
    );
    assert_eq!(app.ui.inputs.conversation.cursor_pos, 0);
    assert_eq!(app.ui.inputs.conversation.content, "hello brave ");
}

#[tokio::test]
async fn conversation_input_paste_inserts_multiline_text_without_submitting() {
    let backend = Arc::new(TestBackend::with_agents(vec![ok_agent(AgentName::Codex)]));
    let mut app = App::with_teamwork_store(
        backend.clone(),
        false,
        PathBuf::from("/tmp"),
        crate::teamwork::TeamworkStore::disabled(),
    );
    set_test_conversation_nav(&mut app, "test", "conversation-1");
    app.ui
        .status
        .update_agents(vec![ok_agent(AgentName::Codex)]);
    app.ui.session_panel.list.items.push(SessionEntry {
        session_id: "thread-codex-1234".into(),
        agent: AgentName::Codex,
        workspace: PathBuf::from("/tmp"),
        state: SessionState::Idle,
        parent_session_id: None,
    });
    app.ui.session_panel.chat_states.insert(
        "thread-codex-1234".into(),
        ChatState::new("thread-codex-1234".into(), AgentName::Codex),
    );
    app.select_thread(0);
    app.ui.focus.focus(PaneId::Input);

    assert!(
        app.handle_event(AppEvent::Paste("first\r\nsecond\nthird".into()))
            .await
    );

    assert_eq!(app.ui.inputs.conversation.content, "first\nsecond\nthird");
    assert_eq!(
        app.ui.inputs.conversation.cursor_pos,
        "first\nsecond\nthird".len()
    );
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
    assert_eq!(app.ui.conversation.messages.len(), 1);
    assert_eq!(app.ui.conversation.messages[0].body, "first\nsecond\nthird");
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
    app.ui.inputs.conversation.content = "@gemini write tests".into();
    app.ui.inputs.conversation.cursor_pos = app.ui.inputs.conversation.content.len();
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
    assert_eq!(app.ui.inputs.conversation.content, "");
    assert_eq!(app.ui.session_panel.list.selected, Some(0));
    assert_eq!(app.ui.session_panel.list.items[0].agent, AgentName::Gemini);
}

#[tokio::test]
async fn conversation_input_on_closed_selected_thread_starts_new_same_agent() {
    let backend = Arc::new(TestBackend::with_agents(vec![ok_agent(
        AgentName::Opencode,
    )]));
    let mut app = App::new(backend.clone(), false, PathBuf::from("/tmp"));
    set_test_conversation_nav(&mut app, "test", "conversation-1");
    app.ui
        .status
        .update_agents(vec![ok_agent(AgentName::Opencode)]);
    app.ui.session_panel.list.items.push(SessionEntry {
        session_id: "thread-opencode-closed".into(),
        agent: AgentName::Opencode,
        workspace: PathBuf::from("/tmp"),
        state: SessionState::Closed {
            reason: minos_agent_runtime::CloseReason::UserClose,
        },
        parent_session_id: None,
    });
    app.select_thread(0);
    app.ui.focus.focus(PaneId::Input);
    app.ui.inputs.conversation.content = "are you there?".into();
    app.ui.inputs.conversation.cursor_pos = app.ui.inputs.conversation.content.len();

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
    assert_eq!(app.ui.inputs.conversation.content, "");
    assert_eq!(app.ui.session_panel.list.selected, Some(1));
    assert_eq!(app.ui.session_panel.list.items[1].agent, AgentName::Opencode);
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
    app.ui.session_panel.list.items.push(SessionEntry {
        session_id: "thread-opencode-closed".into(),
        agent: AgentName::Opencode,
        workspace: PathBuf::from("/tmp"),
        state: SessionState::Closed {
            reason: minos_agent_runtime::CloseReason::UserClose,
        },
        parent_session_id: None,
    });
    app.select_thread(0);
    set_test_agent_detail_nav(&mut app, "test", "conversation-1");
    app.ui.focus.switch_layout(true);
    app.ui.focus.focus(PaneId::Input);
    app.ui.inputs.agent.content = "continue".into();
    app.ui.inputs.agent.cursor_pos = app.ui.inputs.agent.content.len();

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
    assert_eq!(app.ui.inputs.agent.content, "");
    assert_eq!(app.ui.session_panel.list.selected, Some(1));
    assert_eq!(app.ui.session_panel.list.items[1].agent, AgentName::Opencode);
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
    app.ui.session_panel.list.items.push(SessionEntry {
        session_id: "thread-opencode-closed".into(),
        agent: AgentName::Opencode,
        workspace: PathBuf::from("/tmp"),
        state: SessionState::Closed {
            reason: minos_agent_runtime::CloseReason::UserClose,
        },
        parent_session_id: None,
    });
    app.ui.focus.focus(PaneId::Input);
    app.ui.inputs.conversation.content = "@opencode#thread-o hello".into();
    app.ui.inputs.conversation.cursor_pos = app.ui.inputs.conversation.content.len();
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
    assert!(error.contains("No existing opencode session matches #thread-o"));
}
