use super::*;

#[tokio::test]
async fn daemon_tick_replays_history_without_tui_result_writeback() {
    let backend = Arc::new(
        TestBackend::new()
            .with_connection_state(BackendConnectionState::Connected {
                endpoint: "ws://127.0.0.1:43123".into(),
            })
            .with_listed_threads(vec![BackendSessionSnapshot {
                session_id: "thread-opencode-1234".into(),
                agent: Some(AgentName::Opencode),
                workspace: PathBuf::from("/tmp/ws"),
                state: SessionState::Idle,
                parent_session_id: None,
            }])
            .with_history_pages(
                "thread-opencode-1234",
                vec![ReadSessionRawHistoryResponse {
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
    let mut app = App::with_teamwork_store(
        backend,
        false,
        PathBuf::from("/tmp/ws"),
        crate::teamwork::TeamworkStore::disabled(),
    );
    set_test_conversation_nav(&mut app, "test", "conversation-1");
    app.ui.conversation.agent_sessions.items = vec![crate::backend::SessionSummaryEntry {
        session_id: "thread-opencode-1234".into(),
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

    assert!(app.handle_event(AppEvent::Tick).await);

    // History is hydrated into chat state for AgentDetail, but conversation
    // agent-result rows are written by daemon completion, not TUI.
    assert!(app.ui.conversation.messages.is_empty());
    let chat = app
        .ui
        .session_panel
        .chat_states
        .get("thread-opencode-1234")
        .expect("opencode chat hydrated");
    assert_eq!(
        chat.last_completed_assistant_text().map(|(_, text)| text),
        Some("在的，有什么可以帮你的？".into())
    );
}

#[tokio::test]
async fn conversation_append_event_refreshes_current_conversation_messages() {
    let backend = Arc::new(TestBackend::new());
    let mut app = App::with_teamwork_store(
        backend.clone(),
        false,
        PathBuf::from("/tmp/ws"),
        crate::teamwork::TeamworkStore::disabled(),
    );
    set_test_conversation_nav(&mut app, "test", "conversation-1");
    backend
        .append_conversation_message(
            "conversation-1",
            Some("msg-1"),
            None,
            "user",
            None,
            "visible update",
        )
        .await
        .unwrap();

    assert!(app.ui.conversation.messages.is_empty());
    assert!(
        app.handle_event(AppEvent::ConversationMessageAppended {
            conversation_id: "conversation-1".into(),
            message_seq: 1,
        })
        .await
    );

    assert_eq!(app.ui.conversation.messages.len(), 1);
    assert_eq!(app.ui.conversation.messages[0].body, "visible update");
}

#[tokio::test]
async fn post_conversation_update_uses_source_identity_and_delivers_target_mention() {
    let sessions = vec![
        crate::backend::SessionSummaryEntry {
            session_id: "thread-codex-1234".into(),
            agent: AgentName::Codex,
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
            session_id: "thread-opencode-1234".into(),
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
    ];
    let backend =
        Arc::new(TestBackend::new().with_conversation_sessions("conversation-1", sessions.clone()));
    let mut app = App::with_teamwork_store(
        backend.clone(),
        false,
        PathBuf::from("/tmp/ws"),
        crate::teamwork::TeamworkStore::disabled(),
    );
    set_test_conversation_nav(&mut app, "test", "conversation-1");
    app.ui.conversation.agent_sessions.items = sessions;

    let response = app
        .handle_mcp_tool_call(
            minos_chat_store::mcp_socket::SocketRequest::PostConversationUpdate {
                conversation_id: "conversation-1".into(),
                source_agent: Some("codex".into()),
                source_session_id: Some("thread-codex-1234".into()),
                message: "@opencode#thread-o review posted".into(),
            },
        )
        .await
        .unwrap();

    assert!(matches!(
        response,
        minos_chat_store::mcp_socket::SocketResponse::Ok { .. }
    ));
    assert_eq!(
        backend
            .sent_messages
            .lock()
            .expect("sent messages lock")
            .as_slice(),
        &[(
            "thread-opencode-1234".to_string(),
            "review posted".to_string()
        )]
    );
    let messages = backend
        .conversation_messages
        .lock()
        .expect("conversation messages lock")
        .get("conversation-1")
        .cloned()
        .unwrap_or_default();
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].sender_role, "agent");
    assert_eq!(messages[0].agent, Some(AgentName::Codex));
    assert_eq!(messages[0].session_id.as_deref(), Some("thread-codex-1234"));
    assert_eq!(messages[0].body, "@opencode#thread-o review posted");
}

#[tokio::test]
async fn post_conversation_update_rejects_source_session_outside_conversation() {
    let backend =
        Arc::new(TestBackend::new().with_conversation_sessions("conversation-1", Vec::new()));
    let mut app = App::with_teamwork_store(
        backend.clone(),
        false,
        PathBuf::from("/tmp/ws"),
        crate::teamwork::TeamworkStore::disabled(),
    );
    set_test_conversation_nav(&mut app, "test", "conversation-1");

    let error = app
        .handle_mcp_tool_call(
            minos_chat_store::mcp_socket::SocketRequest::PostConversationUpdate {
                conversation_id: "conversation-1".into(),
                source_agent: Some("codex".into()),
                source_session_id: Some("thread-from-other-room".into()),
                message: "review posted".into(),
            },
        )
        .await
        .expect_err("stale source thread should be rejected");

    assert!(error
        .to_string()
        .contains("does not belong to conversation conversation-1"));
    assert!(backend
        .conversation_messages
        .lock()
        .expect("conversation messages lock")
        .get("conversation-1")
        .is_none());
}

#[tokio::test]
async fn delegate_to_agent_send_failure_does_not_record_visible_message() {
    let tmp = tempfile::tempdir().unwrap();
    let db_path = tmp.path().join("teamwork.sqlite");
    minos_chat_store::TeamworkStore::open(&db_path)
        .await
        .unwrap()
        .ensure_conversation("conversation-1", "main", "/tmp/ws")
        .await
        .unwrap();
    let sessions = vec![crate::backend::SessionSummaryEntry {
        session_id: "thread-codex-1234".into(),
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
    let backend = Arc::new(
        TestBackend::new()
            .with_conversation_sessions("conversation-1", sessions.clone())
            .with_fail_sends(),
    );
    let mut app = App::with_teamwork_store(
        backend.clone(),
        false,
        PathBuf::from("/tmp/ws"),
        crate::teamwork::TeamworkStore::for_db_path(db_path),
    );
    set_test_conversation_nav(&mut app, "test", "conversation-1");
    app.ui.conversation.agent_sessions.items = sessions;
    app.ui.status.agents = vec![ok_agent(AgentName::Opencode)];

    let error = app
        .handle_mcp_tool_call(
            minos_chat_store::mcp_socket::SocketRequest::DelegateToAgent {
                conversation_id: "conversation-1".into(),
                source_agent: Some("codex".into()),
                source_session_id: Some("thread-codex-1234".into()),
                target_agent: Some("opencode".into()),
                profile_id: None,
                target_profile: None,
                prompt: "say hi".into(),
            },
        )
        .await
        .expect_err("target send failure should fail tool call");

    assert!(error.to_string().contains("send failed"));
    assert!(backend
        .conversation_messages
        .lock()
        .expect("conversation messages lock")
        .get("conversation-1")
        .is_none());
}

#[tokio::test]
async fn delegated_agent_cannot_delegate_to_third_agent() {
    let tmp = tempfile::tempdir().unwrap();
    let db_path = tmp.path().join("teamwork.sqlite");
    let persistent = minos_chat_store::TeamworkStore::open(&db_path)
        .await
        .unwrap();
    persistent
        .ensure_conversation("conversation-1", "main", "/tmp/ws")
        .await
        .unwrap();
    persistent
        .create_delegation(
            "conversation-1",
            Some(AgentName::Codex),
            Some("thread-codex-1234".into()),
            AgentName::Opencode,
            "check this".into(),
            Some("thread-opencode-1234".into()),
        )
        .await
        .unwrap();
    let sessions = vec![crate::backend::SessionSummaryEntry {
        session_id: "thread-opencode-1234".into(),
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
    let backend =
        Arc::new(TestBackend::new().with_conversation_sessions("conversation-1", sessions.clone()));
    let mut app = App::with_teamwork_store(
        backend.clone(),
        false,
        PathBuf::from("/tmp/ws"),
        crate::teamwork::TeamworkStore::for_db_path(db_path),
    );
    set_test_conversation_nav(&mut app, "test", "conversation-1");
    app.ui.conversation.agent_sessions.items = sessions;
    app.ui.status.agents = vec![ok_agent(AgentName::Gemini)];

    let error = app
        .handle_mcp_tool_call(
            minos_chat_store::mcp_socket::SocketRequest::DelegateToAgent {
                conversation_id: "conversation-1".into(),
                source_agent: Some("opencode".into()),
                source_session_id: Some("thread-opencode-1234".into()),
                target_agent: Some("gemini".into()),
                profile_id: None,
                target_profile: None,
                prompt: "say hi".into(),
            },
        )
        .await
        .expect_err("third-agent delegation should be rejected");

    assert!(error
        .to_string()
        .contains("may only delegate back to codex"));
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
}

#[tokio::test]
async fn delegated_agent_result_writeback_is_daemon_owned_not_tui() {
    // Agent result writeback / delegation completion moved to minos-daemon so
    // the loop closes without TUI. TUI only refreshes on conversation events.
    let backend = Arc::new(TestBackend::new());
    let mut app = App::new(backend.clone(), false, PathBuf::from("/tmp/ws"));
    set_test_conversation_nav(&mut app, "test", "conversation-1");
    app.ui.session_panel.list.items.push(SessionEntry {
        session_id: "thread-codex-1234".into(),
        agent: AgentName::Codex,
        workspace: PathBuf::from("/tmp/ws"),
        state: SessionState::Idle,
        parent_session_id: None,
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
            text: "hi".into(),
        },
        UiEventMessage::MessageCompleted {
            message_id: "assistant-1".into(),
            finished_at_ms: 2,
        },
    ]);
    app.ui
        .session_panel
        .chat_states
        .insert("thread-codex-1234".into(), chat);

    app.record_agent_conversation_result_if_done("thread-codex-1234")
        .await;

    let messages = backend
        .conversation_messages
        .lock()
        .expect("conversation messages lock")
        .get("conversation-1")
        .cloned()
        .unwrap_or_default();
    assert!(
        messages.is_empty(),
        "TUI must not write agent results after daemon ownership cutover"
    );
    assert!(backend
        .sent_messages
        .lock()
        .expect("sent messages lock")
        .is_empty());
}

#[tokio::test]
async fn hidden_project_agent_ingest_does_not_write_conversation_from_tui() {
    let backend = Arc::new(TestBackend::new());
    let mut app = App::new(backend.clone(), false, PathBuf::from("/tmp/p1"));
    app.ui.projects.items = vec![
        crate::backend::ProjectEntry {
            project_id: "p1".into(),
            name: "P1".into(),
            workspace_path: PathBuf::from("/tmp/p1"),
            thread_count: 0,
            created_at_ms: 0,
            updated_at_ms: 0,
        },
        crate::backend::ProjectEntry {
            project_id: "p2".into(),
            name: "P2".into(),
            workspace_path: PathBuf::from("/tmp/p2"),
            thread_count: 0,
            created_at_ms: 0,
            updated_at_ms: 0,
        },
    ];

    app.apply_action(Action::EffectCompleted(
        crate::action::EffectResult::ConversationOpened {
            project_id: "p2".into(),
            conversation_id: "c2".into(),
            messages: Vec::new(),
            sessions: vec![crate::backend::SessionSummaryEntry {
                session_id: "thread-p2".into(),
                agent: AgentName::Codex,
                title: None,
                first_ts_ms: 0,
                last_ts_ms: 0,
                message_count: 0,
                ended_at_ms: None,
                parent_session_id: None,
                state: SessionState::Idle,
                needs_continue: false,
            }],
        },
    ))
    .await;
    app.apply_action(Action::EffectCompleted(
        crate::action::EffectResult::ConversationsLoaded {
            project_id: "p1".into(),
            conversations: Vec::new(),
        },
    ))
    .await;

    assert!(
        app.handle_event(AppEvent::ManagerEvent(ManagerEvent::SessionAdded {
            session_id: "thread-p2".into(),
            workspace: PathBuf::from("/tmp/p2"),
            agent: AgentName::Codex,
            parent_session_id: None,
        }))
        .await
    );
    assert!(
        app.handle_event(AppEvent::Ingest(LocalIngestFrame {
            session_id: "thread-p2".into(),
            seq: 1,
            agent: AgentName::Codex,
            ui_events: vec![
                UiEventMessage::MessageStarted {
                    message_id: "assistant-1".into(),
                    role: MessageRole::Assistant,
                    started_at_ms: 1,
                },
                UiEventMessage::TextDelta {
                    message_id: "assistant-1".into(),
                    text: "done from p2".into(),
                },
                UiEventMessage::MessageCompleted {
                    message_id: "assistant-1".into(),
                    finished_at_ms: 2,
                },
            ],
            ts_ms: 2,
        }))
        .await
    );

    // Daemon owns writeback; TUI must not append conversation messages here.
    assert!(backend
        .conversation_messages
        .lock()
        .expect("conversation messages lock")
        .get("c2")
        .is_none_or(|messages| messages.is_empty()));
    assert!(app.ui.conversation.messages.is_empty());
}

#[tokio::test]
async fn subagent_result_is_not_recorded_to_conversation_timeline() {
    let backend = Arc::new(TestBackend::new());
    let mut app = App::new(backend.clone(), false, PathBuf::from("/tmp/ws"));
    set_test_conversation_nav(&mut app, "test", "conversation-1");
    app.ui.conversation.agent_sessions.items = vec![
        crate::backend::SessionSummaryEntry {
            session_id: "parent-thread".into(),
            agent: AgentName::Codex,
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
            session_id: "sub-thread".into(),
            agent: AgentName::Codex,
            title: None,
            first_ts_ms: 0,
            last_ts_ms: 0,
            message_count: 0,
            ended_at_ms: None,
            parent_session_id: Some("parent-thread".into()),
            state: SessionState::Idle,
            needs_continue: false,
        },
    ];
    app.apply_action(Action::EffectCompleted(
        crate::action::EffectResult::ConversationOpened {
            project_id: "test".into(),
            conversation_id: "conversation-1".into(),
            messages: Vec::new(),
            sessions: app.ui.conversation.agent_sessions.items.clone(),
        },
    ))
    .await;
    app.ui.session_panel.list.items.push(SessionEntry {
        session_id: "sub-thread".into(),
        agent: AgentName::Codex,
        workspace: PathBuf::from("/tmp/ws"),
        state: SessionState::Idle,
        parent_session_id: Some("parent-thread".into()),
    });

    assert!(
        app.handle_event(AppEvent::Ingest(LocalIngestFrame {
            session_id: "sub-thread".into(),
            seq: 1,
            agent: AgentName::Codex,
            ui_events: vec![
                UiEventMessage::MessageStarted {
                    message_id: "sub-answer".into(),
                    role: MessageRole::Assistant,
                    started_at_ms: 1,
                },
                UiEventMessage::TextDelta {
                    message_id: "sub-answer".into(),
                    text: "subagent answer".into(),
                },
                UiEventMessage::MessageCompleted {
                    message_id: "sub-answer".into(),
                    finished_at_ms: 2,
                },
            ],
            ts_ms: 2,
        }))
        .await
    );

    assert!(app.ui.conversation.messages.is_empty());
    assert!(backend
        .conversation_messages
        .lock()
        .expect("conversation messages lock")
        .get("conversation-1")
        .is_none());
}

#[tokio::test]
async fn idle_thread_state_finishes_streaming_assistant_cursor() {
    let backend = Arc::new(TestBackend::new());
    let mut app = App::new(backend, false, PathBuf::from("/tmp"));
    app.ui.session_panel.list.items.push(SessionEntry {
        session_id: "thread-codex-1234".into(),
        agent: AgentName::Codex,
        workspace: PathBuf::from("/tmp/ws"),
        state: SessionState::Running {
            turn_started_at_ms: 0,
        },
        parent_session_id: None,
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
    app.ui
        .session_panel
        .chat_states
        .insert("thread-codex-1234".into(), chat);

    assert!(
        app.handle_event(AppEvent::ManagerEvent(ManagerEvent::SessionStateChanged {
            session_id: "thread-codex-1234".into(),
            old: SessionState::Running {
                turn_started_at_ms: 0,
            },
            new: SessionState::Idle,
            at_ms: 2,
        }))
        .await
    );

    match &app.ui.session_panel.chat_states["thread-codex-1234"].items[0] {
        ChatItem::AssistantText { is_streaming, .. } => assert!(!*is_streaming),
        other => panic!("expected AssistantText, got {other:?}"),
    }
}

#[tokio::test]
async fn esc_at_projects_level_does_not_quit() {
    let backend = Arc::new(TestBackend::new());
    let mut app = App::new(backend, false, PathBuf::from("/tmp"));
    set_test_projects_nav(&mut app);

    let redraw = app.handle_key(press(KeyCode::Esc)).await;

    assert!(!redraw);
    assert!(!app.should_quit());
}

#[tokio::test]
async fn enter_on_agent_list_opens_detail_and_esc_uplevels() {
    let backend = Arc::new(TestBackend::new());
    let mut app = App::new(backend, false, PathBuf::from("/tmp"));
    set_test_conversation_nav(&mut app, "test", "conversation-1");
    app.ui.session_panel.list.items.push(SessionEntry {
        session_id: "thread-1".into(),
        agent: AgentName::Codex,
        workspace: PathBuf::from("/tmp"),
        state: SessionState::Idle,
        parent_session_id: None,
    });
    app.ui.session_panel.chat_states.insert(
        "thread-1".into(),
        ChatState::new("thread-1".into(), AgentName::Codex),
    );
    app.ui.conversation.agent_sessions.items = vec![crate::backend::SessionSummaryEntry {
        session_id: "thread-1".into(),
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
    app.ui.conversation.agent_sessions.select(Some(0));
    app.select_thread(0);
    app.ui.focus.focus(PaneId::Sidebar);

    assert!(app.handle_key(press(KeyCode::Enter)).await);
    assert!(matches!(
        app.ui.nav_level(),
        crate::nav::NavLevel::AgentDetail { session_id, .. } if session_id == "thread-1"
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
async fn live_subagent_spawn_appears_in_sidebar_and_opens_readonly_detail() {
    let backend = Arc::new(TestBackend::new());
    let mut app = App::new(backend, false, PathBuf::from("/tmp/ws"));
    app.ui.projects.items = vec![crate::backend::ProjectEntry {
        project_id: "test".into(),
        name: "Test".into(),
        workspace_path: PathBuf::from("/tmp/ws"),
        thread_count: 0,
        created_at_ms: 0,
        updated_at_ms: 0,
    }];
    set_test_conversation_nav(&mut app, "test", "conversation-1");
    app.ui.conversation.agent_sessions.items = vec![crate::backend::SessionSummaryEntry {
        session_id: "parent-thread".into(),
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
    app.ui.conversation.agent_sessions.select(Some(0));

    assert!(
        app.handle_event(AppEvent::Ingest(LocalIngestFrame {
            session_id: "parent-thread".into(),
            seq: 1,
            agent: AgentName::Codex,
            ui_events: vec![UiEventMessage::SubagentSpawned {
                parent_session_id: "parent-thread".into(),
                sub_session_id: "sub-thread".into(),
                tool_call_id: "sub-call".into(),
                agent: AgentName::Codex,
                model: Some("gpt-5".into()),
                prompt: Some("inspect".into()),
                title: Some("inspect".into()),
            }],
            ts_ms: 42,
        }))
        .await
    );

    assert_eq!(
        app.ui
            .flat_agent_sessions()
            .iter()
            .map(|flat| {
                (
                    app.ui.conversation.agent_sessions.items[flat.source_index]
                        .session_id
                        .as_str(),
                    flat.depth,
                )
            })
            .collect::<Vec<_>>(),
        vec![("parent-thread", 0), ("sub-thread", 1)]
    );

    app.ui.conversation.agent_sessions.select(Some(1));
    assert!(
        app.apply_action(Action::Nav(crate::nav::NavAction::Downlevel))
            .await
    );

    assert!(matches!(
        app.ui.nav_level(),
        crate::nav::NavLevel::AgentDetail { session_id, .. } if session_id == "sub-thread"
    ));
    assert!(app.ui.current_thread_is_subagent());
}

#[tokio::test]
async fn mouse_wheel_scrolls_chat_and_focuses_it() {
    let backend = Arc::new(TestBackend::new());
    let mut app = App::new(backend, false, PathBuf::from("/tmp"));
    app.ui.session_panel.list.items.push(SessionEntry {
        session_id: "thread-1".into(),
        agent: AgentName::Codex,
        workspace: PathBuf::from("/tmp"),
        state: SessionState::Idle,
        parent_session_id: None,
    });
    app.ui.session_panel.chat_states.insert(
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
    app.ui.session_panel.list.items.push(SessionEntry {
        session_id: "thread-1".into(),
        agent: AgentName::Codex,
        workspace: PathBuf::from("/tmp"),
        state: SessionState::Idle,
        parent_session_id: None,
    });
    app.ui.session_panel.list.items.push(SessionEntry {
        session_id: "thread-2".into(),
        agent: AgentName::Claude,
        workspace: PathBuf::from("/tmp"),
        state: SessionState::Idle,
        parent_session_id: None,
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
    assert_eq!(app.ui.session_panel.list.selected, Some(1));
}

#[tokio::test]
async fn clicking_thread_list_blank_area_focuses_thread_list() {
    let backend = Arc::new(TestBackend::new());
    let mut app = App::new(backend, false, PathBuf::from("/tmp"));
    app.ui.panel_areas.main_list = Rect::new(0, 0, 20, 10);
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
    let _clipboard_guard = super::TEST_CLIPBOARD_LOCK.lock().await;
    let backend = Arc::new(TestBackend::new());
    let mut app = App::new(backend, false, PathBuf::from("/tmp"));
    app.ui.session_panel.list.items.push(SessionEntry {
        session_id: "thread-1".into(),
        agent: AgentName::Codex,
        workspace: PathBuf::from("/tmp"),
        state: SessionState::Idle,
        parent_session_id: None,
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
    app.ui
        .session_panel
        .chat_states
        .insert("thread-1".into(), chat);
    app.select_thread(0);
    set_test_agent_detail_nav(&mut app, "test", "conversation-1");
    app.ui.focus.switch_layout(true);
    app.ui.panel_areas.agent_chat = Rect::new(0, 0, 40, 10);
    super::TEST_CLIPBOARD
        .lock()
        .expect("test clipboard lock")
        .clear();

    // Content area is inset by the chat border. User lines paint as
    // `❯ hello` / `  world` (Grok-style, no [You] label row).
    let down = app
        .handle_mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 3,
            row: 1,
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
            column: 10,
            row: 2,
            modifiers: KeyModifiers::NONE,
        })
        .await;

    assert!(up);
    let clip = super::TEST_CLIPBOARD
        .lock()
        .expect("test clipboard lock")
        .clone();
    assert!(
        clip.iter().any(|s| s.contains('h') || s.contains('w')),
        "expected selected user text on clipboard, got {clip:?}"
    );
    assert!(app
        .ui
        .current_chat()
        .expect("current chat")
        .selection
        .is_none());
}

#[tokio::test]
async fn mouse_selection_copies_conversation_text_on_release() {
    let _clipboard_guard = super::TEST_CLIPBOARD_LOCK.lock().await;
    let backend = Arc::new(TestBackend::new());
    let mut app = App::new(backend, false, PathBuf::from("/tmp"));
    set_test_conversation_nav(&mut app, "test", "conversation-1");
    app.ui.focus.switch_layout(true);
    app.ui.panel_areas.conversation_chat = Rect::new(0, 0, 40, 12);
    app.ui
        .conversation
        .set_messages(vec![crate::backend::ConversationMessageEntry {
            message_seq: 1,
            message_id: "m1".into(),
            conversation_id: "conversation-1".into(),
            session_id: None,
            created_at_ms: 1,
            sender_role: "user".into(),
            agent: None,
            body: "hello\nworld".into(),
            reply_to_message_id: None,
            delegation_id: None,
            mentions: Vec::new(),
        }]);
    super::TEST_CLIPBOARD
        .lock()
        .expect("test clipboard lock")
        .clear();

    // Content area is inset by the conversation border.
    // Layout: row0 = "[You]", row1 = "hello", row2 = "world"
    let down = app
        .handle_mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 1,
            row: 2,
            modifiers: KeyModifiers::NONE,
        })
        .await;
    assert!(down);
    assert!(app.ui.conversation.selection.is_some());
    assert!(super::TEST_CLIPBOARD
        .lock()
        .expect("test clipboard lock")
        .is_empty());

    let up = app
        .handle_mouse(MouseEvent {
            kind: MouseEventKind::Up(MouseButton::Left),
            column: 10,
            row: 3,
            modifiers: KeyModifiers::NONE,
        })
        .await;

    assert!(up);
    let clip = super::TEST_CLIPBOARD
        .lock()
        .expect("test clipboard lock")
        .clone();
    assert!(
        clip.iter()
            .any(|s| s.contains("hello") || s.contains("world") || s.contains('h')),
        "expected selected conversation text on clipboard, got {clip:?}"
    );
    assert!(app.ui.conversation.selection.is_none());
}

#[tokio::test]
async fn delete_key_in_thread_list_opens_confirmation() {
    let backend = Arc::new(TestBackend::new());
    let mut app = App::new(backend.clone(), false, PathBuf::from("/tmp"));
    set_test_conversation_nav(&mut app, "test", "conversation-1");
    app.ui.session_panel.list.items.push(SessionEntry {
        session_id: "thread-1".into(),
        agent: AgentName::Codex,
        workspace: PathBuf::from("/tmp"),
        state: SessionState::Idle,
        parent_session_id: None,
    });
    app.ui.session_panel.list.items.push(SessionEntry {
        session_id: "thread-2".into(),
        agent: AgentName::Claude,
        workspace: PathBuf::from("/tmp"),
        state: SessionState::Idle,
        parent_session_id: None,
    });
    app.ui.session_panel.chat_states.insert(
        "thread-1".into(),
        ChatState::new("thread-1".into(), AgentName::Codex),
    );
    app.state.hydrated_threads.insert("thread-1".into());
    app.select_thread(0);
    app.ui.focus.focus(PaneId::Sidebar);

    let redraw = app.handle_key(press(KeyCode::Delete)).await;

    assert!(redraw);
    assert!(app.ui.overlays.delete_confirm.is_some());
    assert!(backend.deleted.lock().expect("delete list lock").is_empty());
    assert_eq!(app.ui.session_panel.list.items.len(), 2);
    assert!(app.ui.session_panel.chat_states.contains_key("thread-1"));

    let redraw = app.handle_key(press(KeyCode::Esc)).await;

    assert!(redraw);
    assert!(app.ui.overlays.delete_confirm.is_none());
    assert_eq!(app.ui.session_panel.list.items.len(), 2);
}

#[tokio::test]
async fn enter_confirms_thread_delete_and_removes_local_state() {
    let backend = Arc::new(TestBackend::new());
    let mut app = App::new(backend.clone(), false, PathBuf::from("/tmp"));
    set_test_conversation_nav(&mut app, "test", "conversation-1");
    app.ui.session_panel.list.items.push(SessionEntry {
        session_id: "thread-1".into(),
        agent: AgentName::Codex,
        workspace: PathBuf::from("/tmp"),
        state: SessionState::Idle,
        parent_session_id: None,
    });
    app.ui.session_panel.list.items.push(SessionEntry {
        session_id: "thread-2".into(),
        agent: AgentName::Claude,
        workspace: PathBuf::from("/tmp"),
        state: SessionState::Idle,
        parent_session_id: None,
    });
    app.ui.session_panel.chat_states.insert(
        "thread-1".into(),
        ChatState::new("thread-1".into(), AgentName::Codex),
    );
    app.state.hydrated_threads.insert("thread-1".into());
    app.select_thread(0);
    app.ui.focus.focus(PaneId::Sidebar);

    assert!(app.handle_key(press(KeyCode::Delete)).await);
    let redraw = app.handle_key(press(KeyCode::Enter)).await;

    assert!(redraw);
    assert!(app.ui.overlays.delete_confirm.is_none());
    assert_eq!(
        backend.deleted.lock().expect("delete list lock").as_slice(),
        &["thread-1".to_owned()]
    );
    assert_eq!(app.ui.session_panel.list.items.len(), 1);
    assert_eq!(app.ui.session_panel.list.items[0].session_id, "thread-2");
    assert_eq!(app.ui.session_panel.list.selected, Some(0));
    assert!(!app.ui.session_panel.chat_states.contains_key("thread-1"));
    assert!(!app.state.hydrated_threads.contains("thread-1"));
}

#[tokio::test]
async fn init_hydrates_connected_daemon_threads_with_agent_and_paginated_history() {
    let backend = Arc::new(
        TestBackend::with_agents(vec![ok_agent(AgentName::Claude)])
            .with_connection_state(BackendConnectionState::Connected {
                endpoint: "ws://127.0.0.1:43123".into(),
            })
            .with_listed_threads(vec![BackendSessionSnapshot {
                session_id: "thread-1".into(),
                agent: Some(AgentName::Claude),
                workspace: PathBuf::from("/tmp/ws"),
                state: SessionState::Suspended {
                    reason: minos_agent_runtime::PauseReason::DaemonRestart,
                },
                parent_session_id: None,
            }])
            .with_history_pages(
                "thread-1",
                vec![
                    ReadSessionRawHistoryResponse {
                        events: Vec::new(),
                        next_seq: Some(2),
                    },
                    ReadSessionRawHistoryResponse {
                        events: Vec::new(),
                        next_seq: None,
                    },
                ],
            ),
    );
    let mut app = App::new(backend.clone(), false, PathBuf::from("/tmp/ws"));

    app.init().await.unwrap();

    assert_eq!(app.ui.session_panel.list.items.len(), 1);
    assert_eq!(app.ui.session_panel.list.items[0].agent, AgentName::Claude);
    assert_eq!(app.ui.session_panel.list.selected, Some(0));
    assert_eq!(
        app.ui
            .session_panel
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
async fn apply_daemon_thread_metadata_updates_state_without_history_rpc() {
    // Regression: main-loop DaemonThreadsListed must stay CPU-cheap — no
    // history RPC even when a new unhydrated thread appears in the list.
    let backend = Arc::new(
        TestBackend::with_agents(vec![ok_agent(AgentName::Codex)])
            .with_connection_state(BackendConnectionState::Connected {
                endpoint: "ws://127.0.0.1:43123".into(),
            })
            .with_history_pages(
                "thread-a",
                vec![ReadSessionRawHistoryResponse {
                    events: Vec::new(),
                    next_seq: None,
                }],
            )
            .with_history_pages(
                "thread-b",
                vec![ReadSessionRawHistoryResponse {
                    events: Vec::new(),
                    next_seq: None,
                }],
            ),
    );
    let mut app = App::new(backend.clone(), false, PathBuf::from("/tmp/ws"));
    app.ui.session_panel.list.items.push(SessionEntry {
        session_id: "thread-a".into(),
        agent: AgentName::Codex,
        workspace: PathBuf::from("/tmp/ws"),
        state: SessionState::Idle,
        parent_session_id: None,
    });
    app.ui.session_panel.chat_states.insert(
        "thread-a".into(),
        ChatState::new("thread-a".into(), AgentName::Codex),
    );
    app.state.hydrated_threads.insert("thread-a".into());
    app.ui.session_panel.list.select(Some(0));

    let redraw = app.apply_daemon_thread_metadata(vec![
        BackendSessionSnapshot {
            session_id: "thread-a".into(),
            agent: Some(AgentName::Codex),
            workspace: PathBuf::from("/tmp/ws"),
            state: SessionState::Running {
                turn_started_at_ms: 1,
            },
            parent_session_id: None,
        },
        BackendSessionSnapshot {
            session_id: "thread-b".into(),
            agent: Some(AgentName::Claude),
            workspace: PathBuf::from("/tmp/ws"),
            state: SessionState::Idle,
            parent_session_id: None,
        },
    ]);

    assert!(redraw, "state update and new thread should mark changed");
    assert_eq!(app.ui.session_panel.list.items.len(), 2);
    let thread_a = app
        .ui
        .session_panel
        .list
        .items
        .iter()
        .find(|t| t.session_id == "thread-a")
        .expect("thread-a");
    assert!(
        matches!(thread_a.state, SessionState::Running { .. }),
        "existing session state must refresh from snapshot"
    );
    assert!(
        app.ui
            .session_panel
            .list
            .items
            .iter()
            .any(|t| t.session_id == "thread-b"),
        "new thread appears in list"
    );
    assert!(
        app.ui.session_panel.chat_states.contains_key("thread-b"),
        "new thread gets an empty chat shell"
    );
    assert!(
        !app.state.hydrated_threads.contains("thread-b"),
        "metadata apply must not hydrate new sessions"
    );
    assert!(
        backend
            .history_calls
            .lock()
            .expect("history calls lock")
            .is_empty(),
        "metadata apply must not call read_session_raw_history"
    );
}

#[tokio::test]
async fn daemon_threads_listed_event_stays_metadata_only() {
    let backend = Arc::new(
        TestBackend::with_agents(vec![ok_agent(AgentName::Codex)])
            .with_connection_state(BackendConnectionState::Connected {
                endpoint: "ws://127.0.0.1:43123".into(),
            })
            .with_history_pages(
                "thread-new",
                vec![ReadSessionRawHistoryResponse {
                    events: Vec::new(),
                    next_seq: None,
                }],
            ),
    );
    let mut app = App::new(backend.clone(), false, PathBuf::from("/tmp/ws"));
    app.ui.session_panel.list.items.push(SessionEntry {
        session_id: "thread-1".into(),
        agent: AgentName::Codex,
        workspace: PathBuf::from("/tmp/ws"),
        state: SessionState::Idle,
        parent_session_id: None,
    });
    app.ui.session_panel.chat_states.insert(
        "thread-1".into(),
        ChatState::new("thread-1".into(), AgentName::Codex),
    );
    app.state.hydrated_threads.insert("thread-1".into());

    let redraw = app
        .handle_event(crate::event::AppEvent::DaemonThreadsListed {
            sessions: vec![
                BackendSessionSnapshot {
                    session_id: "thread-1".into(),
                    agent: Some(AgentName::Codex),
                    workspace: PathBuf::from("/tmp/ws"),
                    state: SessionState::Closed {
                        reason: minos_agent_runtime::CloseReason::UserClose,
                    },
                    parent_session_id: None,
                },
                // Unhydrated new thread must not trigger history RPC on this path.
                BackendSessionSnapshot {
                    session_id: "thread-new".into(),
                    agent: Some(AgentName::Claude),
                    workspace: PathBuf::from("/tmp/ws"),
                    state: SessionState::Idle,
                    parent_session_id: None,
                },
            ],
        })
        .await;

    assert!(redraw);
    assert!(
        matches!(
            app.ui.session_panel.list.items[0].state,
            SessionState::Closed { .. }
        ),
        "DaemonThreadsListed must still apply state metadata"
    );
    assert!(app
        .ui
        .session_panel
        .list
        .items
        .iter()
        .any(|t| t.session_id == "thread-new"));
    assert!(!app.state.hydrated_threads.contains("thread-new"));
    assert!(
        backend
            .history_calls
            .lock()
            .expect("history calls lock")
            .is_empty(),
        "DaemonThreadsListed must never history-RPC (frame-safe poll path)"
    );
}

#[tokio::test]
async fn shutdown_does_not_close_sessions_for_daemon_backend() {
    let backend = Arc::new(TestBackend::new().with_connection_state(
        BackendConnectionState::Connected {
            endpoint: "ws://127.0.0.1:43123".into(),
        },
    ));
    let mut app = App::new(backend.clone(), false, PathBuf::from("/tmp"));
    app.ui.session_panel.list.items.push(SessionEntry {
        session_id: "thread-1".into(),
        agent: AgentName::Codex,
        workspace: PathBuf::from("/tmp"),
        state: SessionState::Idle,
        parent_session_id: None,
    });

    app.shutdown().await;

    assert!(backend.closed.lock().expect("close list lock").is_empty());
}
