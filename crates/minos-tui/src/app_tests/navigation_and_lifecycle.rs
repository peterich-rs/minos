use super::*;

#[tokio::test]
async fn daemon_tick_replays_history_and_records_opencode_result_when_live_ingest_is_missing() {
    let temp = tempfile::tempdir().expect("tempdir");
    let group_store = GroupChatStore::at_path(temp.path().join("group.jsonl"));
    let backend = Arc::new(
        TestBackend::new()
            .with_connection_state(BackendConnectionState::Connected {
                endpoint: "ws://127.0.0.1:43123".into(),
            })
            .with_listed_threads(vec![BackendThreadSnapshot {
                thread_id: "thread-opencode-1234".into(),
                agent: Some(AgentName::Opencode),
                workspace: PathBuf::from("/tmp/ws"),
                state: ThreadState::Idle,
                parent_thread_id: None,
            }])
            .with_history_pages(
                "thread-opencode-1234",
                vec![ReadThreadRawHistoryResponse {
                    events: vec![
                        projected_frame(
                            "thread-opencode-1234",
                            1,
                            AgentName::Opencode,
                            vec![UiEventMessage::MessageStarted {
                                message_id: "msg-assistant-1".into(),
                                role: MessageRole::Assistant,
                                started_at_ms: 1,
                            }],
                        ),
                        projected_frame(
                            "thread-opencode-1234",
                            2,
                            AgentName::Opencode,
                            vec![UiEventMessage::TextDelta {
                                message_id: "msg-assistant-1".into(),
                                text: "在的，有什么可以帮你的？".into(),
                            }],
                        ),
                        projected_frame(
                            "thread-opencode-1234",
                            3,
                            AgentName::Opencode,
                            vec![UiEventMessage::MessageCompleted {
                                message_id: "msg-assistant-1".into(),
                                finished_at_ms: 3,
                            }],
                        ),
                    ],
                    next_seq: None,
                }],
            ),
    );
    let mut app = App::with_group_chat_store(backend, false, PathBuf::from("/tmp/ws"), group_store);

    assert!(app.handle_event(AppEvent::Tick).await);

    assert_eq!(app.ui.group_chat.messages.len(), 1);
    let message = &app.ui.group_chat.messages[0];
    assert_eq!(message.kind, LocalGroupChatMessageKind::AgentResult);
    assert_eq!(message.text, "在的，有什么可以帮你的？");
    assert_eq!(message.agent, Some(AgentName::Opencode));
    assert_eq!(message.thread_id.as_deref(), Some("thread-opencode-1234"));
}

#[tokio::test]
async fn idle_thread_state_finishes_streaming_assistant_cursor() {
    let backend = Arc::new(TestBackend::new());
    let mut app = App::new(backend, false, PathBuf::from("/tmp"));
    app.ui.threads.push(ThreadEntry {
        thread_id: "thread-codex-1234".into(),
        agent: AgentName::Codex,
        workspace: PathBuf::from("/tmp/ws"),
        state: ThreadState::Running {
            turn_started_at_ms: 0,
        },
        parent_thread_id: None,
    });
    let mut chat = ChatState::new("thread-codex-1234".into(), AgentName::Codex);
    chat.apply_ui_events(vec![
        UiEventMessage::MessageStarted {
            message_id: "assistant-1".into(),
            role: MessageRole::Assistant,
            started_at_ms: 1,
        },
        UiEventMessage::TextDelta {
            message_id: "assistant-1".into(),
            text: "partial".into(),
        },
    ]);
    app.ui.chat_states.insert("thread-codex-1234".into(), chat);

    assert!(
        app.handle_event(AppEvent::ManagerEvent(ManagerEvent::ThreadStateChanged {
            thread_id: "thread-codex-1234".into(),
            old: ThreadState::Running {
                turn_started_at_ms: 0,
            },
            new: ThreadState::Idle,
            at_ms: 2,
        }))
        .await
    );

    match &app.ui.chat_states["thread-codex-1234"].items[0] {
        ChatItem::AssistantText { is_streaming, .. } => assert!(!*is_streaming),
        other => panic!("expected AssistantText, got {other:?}"),
    }
}

#[tokio::test]
async fn esc_at_projects_level_quits() {
    let backend = Arc::new(TestBackend::new());
    let mut app = App::new(backend, false, PathBuf::from("/tmp"));
    set_test_projects_nav(&mut app);

    let redraw = app.handle_key(press(KeyCode::Esc)).await;

    assert!(!redraw);
    assert!(app.should_quit());
}

#[tokio::test]
async fn enter_on_agent_list_opens_detail_and_esc_uplevels() {
    let backend = Arc::new(TestBackend::new());
    let mut app = App::new(backend, false, PathBuf::from("/tmp"));
    set_test_conversation_nav(&mut app, "test", "conversation-1");
    app.ui.threads.push(ThreadEntry {
        thread_id: "thread-1".into(),
        agent: AgentName::Codex,
        workspace: PathBuf::from("/tmp"),
        state: ThreadState::Idle,
        parent_thread_id: None,
    });
    app.ui.chat_states.insert(
        "thread-1".into(),
        ChatState::new("thread-1".into(), AgentName::Codex),
    );
    app.ui.conversation_agent_sessions = vec![crate::backend::ThreadSummaryEntry {
        thread_id: "thread-1".into(),
        agent: AgentName::Codex,
        title: None,
        first_ts_ms: 0,
        last_ts_ms: 0,
        message_count: 0,
        ended_at_ms: None,
        parent_thread_id: None,
    }];
    app.ui.selected_agent_session = Some(0);
    app.ui.agent_list_state.select(Some(0));
    app.select_thread(0);
    app.ui.focus.focus(PaneId::Sidebar);

    assert!(app.handle_key(press(KeyCode::Enter)).await);
    assert!(matches!(
        app.ui.nav_level(),
        crate::nav::NavLevel::AgentDetail { thread_id, .. } if thread_id == "thread-1"
    ));

    assert!(app.handle_key(press(KeyCode::Esc)).await);
    assert_eq!(
        app.ui.nav_level(),
        &crate::nav::NavLevel::Conversation {
            conversation_id: "conversation-1".into(),
            project_id: "test".into()
        }
    );
}

#[tokio::test]
async fn mouse_wheel_scrolls_chat_and_focuses_it() {
    let backend = Arc::new(TestBackend::new());
    let mut app = App::new(backend, false, PathBuf::from("/tmp"));
    app.ui.threads.push(ThreadEntry {
        thread_id: "thread-1".into(),
        agent: AgentName::Codex,
        workspace: PathBuf::from("/tmp"),
        state: ThreadState::Idle,
        parent_thread_id: None,
    });
    app.ui.chat_states.insert(
        "thread-1".into(),
        ChatState::new("thread-1".into(), AgentName::Codex),
    );
    app.select_thread(0);
    app.ui.focus.focus(PaneId::Input);
    set_test_agent_detail_nav(&mut app, "test", "conversation-1");
    app.ui.focus.switch_layout(true);
    app.ui.panel_areas.agent_chat = Rect::new(20, 0, 60, 20);
    if let Some(chat) = app.ui.current_chat_mut() {
        chat.update_max_scroll(40);
    }

    let redraw = app
        .handle_mouse(MouseEvent {
            row: 1,
            column: 25,
            ..scroll(MouseEventKind::ScrollUp)
        })
        .await;

    assert!(redraw);
    assert_eq!(app.ui.focus.current(), PaneId::MainChat);
    assert!(app
        .ui
        .current_chat_mut()
        .is_some_and(|chat| !chat.auto_scroll));
}

#[tokio::test]
async fn mouse_wheel_over_thread_list_moves_selection() {
    let backend = Arc::new(TestBackend::new());
    let mut app = App::new(backend, false, PathBuf::from("/tmp"));
    app.ui.threads.push(ThreadEntry {
        thread_id: "thread-1".into(),
        agent: AgentName::Codex,
        workspace: PathBuf::from("/tmp"),
        state: ThreadState::Idle,
        parent_thread_id: None,
    });
    app.ui.threads.push(ThreadEntry {
        thread_id: "thread-2".into(),
        agent: AgentName::Claude,
        workspace: PathBuf::from("/tmp"),
        state: ThreadState::Idle,
        parent_thread_id: None,
    });
    app.select_thread(0);
    app.ui.panel_areas.agent_list = Rect::new(0, 0, 20, 10);

    let redraw = app
        .handle_mouse(MouseEvent {
            row: 2,
            column: 1,
            ..scroll(MouseEventKind::ScrollDown)
        })
        .await;

    assert!(redraw);
    assert_eq!(app.ui.focus.current(), PaneId::Sidebar);
    assert_eq!(app.ui.selected_thread, Some(1));
}

#[tokio::test]
async fn clicking_thread_list_blank_area_focuses_thread_list() {
    let backend = Arc::new(TestBackend::new());
    let mut app = App::new(backend, false, PathBuf::from("/tmp"));
    app.ui.panel_areas.room_list = Rect::new(0, 0, 20, 10);
    app.ui.focus.focus(PaneId::MainChat);

    let redraw = app
        .handle_mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 1,
            row: 8,
            modifiers: KeyModifiers::NONE,
        })
        .await;

    assert!(redraw);
    assert_eq!(app.ui.focus.current(), PaneId::MainList);
}

#[tokio::test]
async fn mouse_selection_copies_chat_text_on_release() {
    let backend = Arc::new(TestBackend::new());
    let mut app = App::new(backend, false, PathBuf::from("/tmp"));
    app.ui.threads.push(ThreadEntry {
        thread_id: "thread-1".into(),
        agent: AgentName::Codex,
        workspace: PathBuf::from("/tmp"),
        state: ThreadState::Idle,
        parent_thread_id: None,
    });
    let mut chat = ChatState::new("thread-1".into(), AgentName::Codex);
    chat.apply_ui_events(vec![
        UiEventMessage::MessageStarted {
            message_id: "m1".into(),
            role: MessageRole::User,
            started_at_ms: 0,
        },
        UiEventMessage::TextDelta {
            message_id: "m1".into(),
            text: "hello\nworld".into(),
        },
        UiEventMessage::MessageCompleted {
            message_id: "m1".into(),
            finished_at_ms: 1,
        },
    ]);
    app.ui.chat_states.insert("thread-1".into(), chat);
    app.select_thread(0);
    set_test_agent_detail_nav(&mut app, "test", "conversation-1");
    app.ui.focus.switch_layout(true);
    app.ui.panel_areas.agent_chat = Rect::new(0, 0, 40, 10);
    super::TEST_CLIPBOARD
        .lock()
        .expect("test clipboard lock")
        .clear();

    let down = app
        .handle_mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 2,
            row: 2,
            modifiers: KeyModifiers::NONE,
        })
        .await;
    assert!(down);
    assert!(super::TEST_CLIPBOARD
        .lock()
        .expect("test clipboard lock")
        .is_empty());

    let up = app
        .handle_mouse(MouseEvent {
            kind: MouseEventKind::Up(MouseButton::Left),
            column: 3,
            row: 3,
            modifiers: KeyModifiers::NONE,
        })
        .await;

    assert!(up);
    assert_eq!(
        super::TEST_CLIPBOARD
            .lock()
            .expect("test clipboard lock")
            .as_slice(),
        &["ello\nwor".to_owned()]
    );
}

#[tokio::test]
async fn delete_key_in_thread_list_opens_confirmation() {
    let backend = Arc::new(TestBackend::new());
    let mut app = App::new(backend.clone(), false, PathBuf::from("/tmp"));
    set_test_conversation_nav(&mut app, "test", "conversation-1");
    app.ui.threads.push(ThreadEntry {
        thread_id: "thread-1".into(),
        agent: AgentName::Codex,
        workspace: PathBuf::from("/tmp"),
        state: ThreadState::Idle,
        parent_thread_id: None,
    });
    app.ui.threads.push(ThreadEntry {
        thread_id: "thread-2".into(),
        agent: AgentName::Claude,
        workspace: PathBuf::from("/tmp"),
        state: ThreadState::Idle,
        parent_thread_id: None,
    });
    app.ui.chat_states.insert(
        "thread-1".into(),
        ChatState::new("thread-1".into(), AgentName::Codex),
    );
    app.state.hydrated_threads.insert("thread-1".into());
    app.select_thread(0);
    app.ui.focus.focus(PaneId::Sidebar);

    let redraw = app.handle_key(press(KeyCode::Delete)).await;

    assert!(redraw);
    assert!(app.ui.delete_confirm.is_some());
    assert!(backend.deleted.lock().expect("delete list lock").is_empty());
    assert_eq!(app.ui.threads.len(), 2);
    assert!(app.ui.chat_states.contains_key("thread-1"));

    let redraw = app.handle_key(press(KeyCode::Esc)).await;

    assert!(redraw);
    assert!(app.ui.delete_confirm.is_none());
    assert_eq!(app.ui.threads.len(), 2);
}

#[tokio::test]
async fn enter_confirms_thread_delete_and_removes_local_state() {
    let backend = Arc::new(TestBackend::new());
    let mut app = App::new(backend.clone(), false, PathBuf::from("/tmp"));
    set_test_conversation_nav(&mut app, "test", "conversation-1");
    app.ui.threads.push(ThreadEntry {
        thread_id: "thread-1".into(),
        agent: AgentName::Codex,
        workspace: PathBuf::from("/tmp"),
        state: ThreadState::Idle,
        parent_thread_id: None,
    });
    app.ui.threads.push(ThreadEntry {
        thread_id: "thread-2".into(),
        agent: AgentName::Claude,
        workspace: PathBuf::from("/tmp"),
        state: ThreadState::Idle,
        parent_thread_id: None,
    });
    app.ui.chat_states.insert(
        "thread-1".into(),
        ChatState::new("thread-1".into(), AgentName::Codex),
    );
    app.state.hydrated_threads.insert("thread-1".into());
    app.select_thread(0);
    app.ui.focus.focus(PaneId::Sidebar);

    assert!(app.handle_key(press(KeyCode::Delete)).await);
    let redraw = app.handle_key(press(KeyCode::Enter)).await;

    assert!(redraw);
    assert!(app.ui.delete_confirm.is_none());
    assert_eq!(
        backend.deleted.lock().expect("delete list lock").as_slice(),
        &["thread-1".to_owned()]
    );
    assert_eq!(app.ui.threads.len(), 1);
    assert_eq!(app.ui.threads[0].thread_id, "thread-2");
    assert_eq!(app.ui.selected_thread, Some(0));
    assert!(!app.ui.chat_states.contains_key("thread-1"));
    assert!(!app.state.hydrated_threads.contains("thread-1"));
}

#[tokio::test]
async fn init_hydrates_connected_daemon_threads_with_agent_and_paginated_history() {
    let backend = Arc::new(
        TestBackend::with_agents(vec![ok_agent(AgentName::Claude)])
            .with_connection_state(BackendConnectionState::Connected {
                endpoint: "ws://127.0.0.1:43123".into(),
            })
            .with_listed_threads(vec![BackendThreadSnapshot {
                thread_id: "thread-1".into(),
                agent: Some(AgentName::Claude),
                workspace: PathBuf::from("/tmp/ws"),
                state: ThreadState::Suspended {
                    reason: minos_agent_runtime::PauseReason::DaemonRestart,
                },
                parent_thread_id: None,
            }])
            .with_history_pages(
                "thread-1",
                vec![
                    ReadThreadRawHistoryResponse {
                        events: Vec::new(),
                        next_seq: Some(2),
                    },
                    ReadThreadRawHistoryResponse {
                        events: Vec::new(),
                        next_seq: None,
                    },
                ],
            ),
    );
    let mut app = App::new(backend.clone(), false, PathBuf::from("/tmp/ws"));

    app.init().await.unwrap();

    assert_eq!(app.ui.threads.len(), 1);
    assert_eq!(app.ui.threads[0].agent, AgentName::Claude);
    assert_eq!(app.ui.selected_thread, Some(0));
    assert_eq!(
        app.ui
            .chat_states
            .get("thread-1")
            .expect("chat state")
            .agent,
        AgentName::Claude
    );
    assert_eq!(
        backend
            .history_calls
            .lock()
            .expect("history calls lock")
            .as_slice(),
        &[
            ("thread-1".to_owned(), None, 1000),
            ("thread-1".to_owned(), Some(1), 1000),
        ]
    );
}

#[tokio::test]
async fn shutdown_does_not_close_threads_for_daemon_backend() {
    let backend = Arc::new(TestBackend::new().with_connection_state(
        BackendConnectionState::Connected {
            endpoint: "ws://127.0.0.1:43123".into(),
        },
    ));
    let mut app = App::new(backend.clone(), false, PathBuf::from("/tmp"));
    app.ui.threads.push(ThreadEntry {
        thread_id: "thread-1".into(),
        agent: AgentName::Codex,
        workspace: PathBuf::from("/tmp"),
        state: ThreadState::Idle,
        parent_thread_id: None,
    });

    app.shutdown().await;

    assert!(backend.closed.lock().expect("close list lock").is_empty());
}
