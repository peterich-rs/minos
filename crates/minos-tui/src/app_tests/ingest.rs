use super::*;

#[tokio::test]
async fn room_can_invite_second_agent_after_first_routed_prompt() {
    let temp = tempfile::tempdir().expect("tempdir");
    let group_store = GroupChatStore::at_path(temp.path().join("group.jsonl"));
    let backend = Arc::new(TestBackend::with_agents(vec![
        ok_agent(AgentName::Codex),
        ok_agent(AgentName::Gemini),
    ]));
    let mut app =
        App::with_group_chat_store(backend.clone(), false, PathBuf::from("/tmp"), group_store);
    app.ui.nav_level = crate::nav::NavLevel::Session {
        project_id: "test".into(),
        thread_id: "test".into(),
    };
    app.ui.status.update_agents(vec![
        ok_agent(AgentName::Codex),
        ok_agent(AgentName::Gemini),
    ]);
    app.ui.focus.focus(PaneId::RoomInput);

    app.ui.room_input.content = "@codex inspect the repo".into();
    app.ui.room_input.cursor_pos = app.ui.room_input.content.len();
    app.sync_input_agent_picker();
    assert!(app.handle_key(press(KeyCode::Enter)).await);

    app.ui.room_input.content = "@gemini".into();
    app.ui.room_input.cursor_pos = app.ui.room_input.content.len();
    app.sync_input_agent_picker();
    assert!(app.ui.room_input.has_agent_picker());
    assert!(app.handle_key(press(KeyCode::Enter)).await);
    assert_eq!(app.ui.room_input.content, "@gemini ");
    assert_eq!(
        backend
            .started
            .lock()
            .expect("started list lock")
            .as_slice(),
        &[AgentName::Codex]
    );
    assert_eq!(app.ui.group_chat.messages.len(), 1);

    assert!(app.handle_key(press(KeyCode::Enter)).await);

    assert_eq!(
        backend
            .started
            .lock()
            .expect("started list lock")
            .as_slice(),
        &[AgentName::Codex, AgentName::Gemini]
    );
    assert_eq!(
        backend
            .sent_messages
            .lock()
            .expect("sent messages lock")
            .as_slice(),
        &[("thread-1".to_owned(), "inspect the repo".to_owned())]
    );
    assert_eq!(app.ui.group_chat.messages.len(), 2);
    assert_eq!(
        app.ui.group_chat.messages[0].text,
        "@codex inspect the repo"
    );
    assert_eq!(app.ui.group_chat.messages[0].agent, Some(AgentName::Codex));
    assert_eq!(app.ui.group_chat.messages[1].text, "@gemini");
    assert_eq!(app.ui.group_chat.messages[1].agent, Some(AgentName::Gemini));
    assert_eq!(app.ui.room_input.content, "");
    assert_eq!(app.ui.selected_thread, Some(1));
}

#[tokio::test]
async fn picker_can_route_prompt_to_existing_agent_session() {
    let temp = tempfile::tempdir().expect("tempdir");
    let group_store = GroupChatStore::at_path(temp.path().join("group.jsonl"));
    let backend = Arc::new(TestBackend::with_agents(vec![
        ok_agent(AgentName::Codex),
        ok_agent(AgentName::Claude),
    ]));
    let mut app =
        App::with_group_chat_store(backend.clone(), false, PathBuf::from("/tmp"), group_store);
    app.ui.nav_level = crate::nav::NavLevel::Session {
        project_id: "test".into(),
        thread_id: "thread-codex-1234".into(),
    };
    app.ui.status.update_agents(vec![
        ok_agent(AgentName::Codex),
        ok_agent(AgentName::Claude),
    ]);
    app.ui.focus.focus(PaneId::RoomInput);
    app.ui.threads.push(ThreadEntry {
        thread_id: "thread-codex".into(),
        agent: AgentName::Codex,
        workspace: PathBuf::from("/tmp"),
        state: ThreadState::Idle,
    });
    app.select_thread(0);
    app.ui.chat_states.insert(
        "thread-codex-1234".into(),
        ChatState::new("thread-codex-1234".into(), AgentName::Codex),
    );
    app.ui.threads[0].thread_id = "thread-codex-1234".into();
    app.ui.room_input.content = "@codex".into();
    app.ui.room_input.cursor_pos = app.ui.room_input.content.len();
    app.sync_input_agent_picker();

    let tokens: Vec<String> = app
        .ui
        .room_agent_mention_candidates()
        .into_iter()
        .map(|candidate| candidate.token)
        .collect();
    assert!(tokens.contains(&"codex".to_owned()));
    assert!(tokens.contains(&"codex#thread-c".to_owned()));
    assert!(app.handle_key(press(KeyCode::Down)).await);
    assert!(app.handle_key(press(KeyCode::Enter)).await);
    assert_eq!(app.ui.room_input.content, "@codex#thread-c ");

    app.ui.room_input.content.push_str("explain the diff");
    app.ui.room_input.cursor_pos = app.ui.room_input.content.len();
    app.sync_input_agent_picker();
    assert!(app.handle_key(press(KeyCode::Enter)).await);

    assert!(backend
        .started
        .lock()
        .expect("started list lock")
        .is_empty());
    assert_eq!(
        backend
            .sent_messages
            .lock()
            .expect("sent messages lock")
            .as_slice(),
        &[(
            "thread-codex-1234".to_owned(),
            "explain the diff".to_owned()
        )]
    );
    assert_eq!(app.ui.group_chat.messages.len(), 1);
    assert_eq!(
        app.ui.group_chat.messages[0].text,
        "@codex#thread-c explain the diff"
    );
}

#[tokio::test]
async fn ingest_does_not_record_agent_result_before_message_completion() {
    let temp = tempfile::tempdir().expect("tempdir");
    let group_store = GroupChatStore::at_path(temp.path().join("group.sqlite"));
    let backend = Arc::new(TestBackend::new());
    let mut app = App::with_group_chat_store(backend, false, PathBuf::from("/tmp/ws"), group_store);
    app.ui.threads.push(ThreadEntry {
        thread_id: "thread-codex-1234".into(),
        agent: AgentName::Codex,
        workspace: PathBuf::from("/tmp/ws"),
        state: ThreadState::Running {
            turn_started_at_ms: 0,
        },
    });
    app.ui.chat_states.insert(
        "thread-codex-1234".into(),
        ChatState::new("thread-codex-1234".into(), AgentName::Codex),
    );
    app.select_thread(0);

    assert!(
        app.handle_event(AppEvent::Ingest(projected_frame(
            "thread-codex-1234",
            1,
            AgentName::Codex,
            vec![
                UiEventMessage::MessageStarted {
                    message_id: "assistant-1".into(),
                    role: MessageRole::Assistant,
                    started_at_ms: 1,
                },
                UiEventMessage::TextDelta {
                    message_id: "assistant-1".into(),
                    text: "Hel".into(),
                },
            ],
        )))
        .await
    );

    assert!(app.ui.group_chat.messages.is_empty());

    assert!(
        app.handle_event(AppEvent::Ingest(projected_frame(
            "thread-codex-1234",
            2,
            AgentName::Codex,
            vec![
                UiEventMessage::TextDelta {
                    message_id: "assistant-1".into(),
                    text: "lo".into(),
                },
                UiEventMessage::MessageCompleted {
                    message_id: "assistant-1".into(),
                    finished_at_ms: 3,
                },
            ],
        )))
        .await
    );

    assert_eq!(app.ui.group_chat.messages.len(), 1);
    let message = &app.ui.group_chat.messages[0];
    assert_eq!(message.kind, LocalGroupChatMessageKind::AgentResult);
    assert_eq!(message.text, "Hello");
    assert_eq!(message.agent, Some(AgentName::Codex));
    assert_eq!(message.thread_id.as_deref(), Some("thread-codex-1234"));
    assert!(message.message_id.contains("assistant-1"));
}

#[tokio::test]
async fn group_chat_records_completed_assistant_message_not_earlier_turn_draft() {
    let temp = tempfile::tempdir().expect("tempdir");
    let group_store = GroupChatStore::at_path(temp.path().join("group.sqlite"));
    let backend = Arc::new(TestBackend::new());
    let mut app = App::with_group_chat_store(backend, false, PathBuf::from("/tmp/ws"), group_store);
    app.ui.threads.push(ThreadEntry {
        thread_id: "thread-codex-1234".into(),
        agent: AgentName::Codex,
        workspace: PathBuf::from("/tmp/ws"),
        state: ThreadState::Running {
            turn_started_at_ms: 0,
        },
    });
    app.ui.chat_states.insert(
        "thread-codex-1234".into(),
        ChatState::new("thread-codex-1234".into(), AgentName::Codex),
    );

    assert!(
        app.handle_event(AppEvent::Ingest(projected_frame(
            "thread-codex-1234",
            1,
            AgentName::Codex,
            vec![
                UiEventMessage::MessageStarted {
                    message_id: "assistant-draft".into(),
                    role: MessageRole::Assistant,
                    started_at_ms: 1,
                },
                UiEventMessage::TextDelta {
                    message_id: "assistant-draft".into(),
                    text: "draft".into(),
                },
            ],
        )))
        .await
    );
    assert!(app.ui.group_chat.messages.is_empty());

    assert!(
        app.handle_event(AppEvent::Ingest(projected_frame(
            "thread-codex-1234",
            2,
            AgentName::Codex,
            vec![
                UiEventMessage::MessageStarted {
                    message_id: "assistant-final".into(),
                    role: MessageRole::Assistant,
                    started_at_ms: 2,
                },
                UiEventMessage::TextDelta {
                    message_id: "assistant-final".into(),
                    text: "final answer".into(),
                },
                UiEventMessage::MessageCompleted {
                    message_id: "assistant-final".into(),
                    finished_at_ms: 3,
                },
            ],
        )))
        .await
    );

    assert_eq!(app.ui.group_chat.messages.len(), 1);
    let message = &app.ui.group_chat.messages[0];
    assert_eq!(message.text, "final answer");
    assert!(message.message_id.contains("assistant-final"));
}

#[tokio::test]
async fn idle_thread_records_last_assistant_message_in_group_chat_once() {
    let temp = tempfile::tempdir().expect("tempdir");
    let group_store = GroupChatStore::at_path(temp.path().join("group.jsonl"));
    let backend = Arc::new(TestBackend::new());
    let mut app = App::with_group_chat_store(backend, false, PathBuf::from("/tmp/ws"), group_store);
    app.ui.threads.push(ThreadEntry {
        thread_id: "thread-gemini-1234".into(),
        agent: AgentName::Gemini,
        workspace: PathBuf::from("/tmp/ws"),
        state: ThreadState::Running {
            turn_started_at_ms: 0,
        },
    });
    let mut chat = ChatState::new("thread-gemini-1234".into(), AgentName::Gemini);
    chat.apply_ui_events(vec![
        UiEventMessage::MessageStarted {
            message_id: "assistant-1".into(),
            role: MessageRole::Assistant,
            started_at_ms: 1,
        },
        UiEventMessage::TextDelta {
            message_id: "assistant-1".into(),
            text: "The module handles auth.".into(),
        },
        UiEventMessage::MessageCompleted {
            message_id: "assistant-1".into(),
            finished_at_ms: 2,
        },
    ]);
    app.ui.chat_states.insert("thread-gemini-1234".into(), chat);
    app.select_thread(0);

    let event = ManagerEvent::ThreadStateChanged {
        thread_id: "thread-gemini-1234".into(),
        old: ThreadState::Running {
            turn_started_at_ms: 0,
        },
        new: ThreadState::Idle,
        at_ms: 3,
    };
    assert!(
        app.handle_event(AppEvent::ManagerEvent(event.clone()))
            .await
    );
    assert!(app.handle_event(AppEvent::ManagerEvent(event)).await);

    assert_eq!(app.ui.group_chat.messages.len(), 1);
    let message = &app.ui.group_chat.messages[0];
    assert_eq!(message.kind, LocalGroupChatMessageKind::AgentResult);
    assert_eq!(message.text, "The module handles auth.");
    assert_eq!(message.agent, Some(AgentName::Gemini));
    assert_eq!(message.thread_short_id.as_deref(), Some("thread-g"));
}

#[tokio::test]
async fn failed_agent_group_result_append_is_retried_on_tick() {
    let temp = tempfile::tempdir().expect("tempdir");
    let failing_store = GroupChatStore::failing();
    let backend = Arc::new(TestBackend::new());
    let mut app = App::with_group_chat_store(backend, false, PathBuf::from("/tmp"), failing_store);
    app.ui.threads.push(ThreadEntry {
        thread_id: "thread-gemini-1234".into(),
        agent: AgentName::Gemini,
        workspace: PathBuf::from("/tmp/ws"),
        state: ThreadState::Running {
            turn_started_at_ms: 0,
        },
    });
    let mut chat = ChatState::new("thread-gemini-1234".into(), AgentName::Gemini);
    chat.apply_ui_events(vec![
        UiEventMessage::MessageStarted {
            message_id: "assistant-1".into(),
            role: MessageRole::Assistant,
            started_at_ms: 1,
        },
        UiEventMessage::TextDelta {
            message_id: "assistant-1".into(),
            text: "The module handles auth.".into(),
        },
        UiEventMessage::MessageCompleted {
            message_id: "assistant-1".into(),
            finished_at_ms: 2,
        },
    ]);
    app.ui.chat_states.insert("thread-gemini-1234".into(), chat);

    assert!(
        app.handle_event(AppEvent::ManagerEvent(ManagerEvent::ThreadStateChanged {
            thread_id: "thread-gemini-1234".into(),
            old: ThreadState::Running {
                turn_started_at_ms: 0,
            },
            new: ThreadState::Idle,
            at_ms: 3,
        }))
        .await
    );

    assert!(app.ui.group_chat.messages.is_empty());
    assert!(!app
        .state
        .recorded_agent_results
        .contains_key("thread-gemini-1234"));

    app.state.group_chat_store = GroupChatStore::at_path(temp.path().join("group.sqlite"));
    assert!(app.handle_event(AppEvent::Tick).await);

    assert_eq!(app.ui.group_chat.messages.len(), 1);
    let message = &app.ui.group_chat.messages[0];
    assert_eq!(message.kind, LocalGroupChatMessageKind::AgentResult);
    assert_eq!(message.text, "The module handles auth.");
    assert_eq!(
        app.state
            .recorded_agent_results
            .get("thread-gemini-1234")
            .map(String::as_str),
        Some("assistant-1")
    );
}

#[tokio::test]
async fn opencode_session_idle_ingest_records_group_result_without_manager_idle() {
    let temp = tempfile::tempdir().expect("tempdir");
    let group_store = GroupChatStore::at_path(temp.path().join("group.jsonl"));
    let backend = Arc::new(TestBackend::new());
    let mut app = App::with_group_chat_store(backend, false, PathBuf::from("/tmp"), group_store);
    app.ui.threads.push(ThreadEntry {
        thread_id: "thread-opencode-1234".into(),
        agent: AgentName::Opencode,
        workspace: PathBuf::from("/tmp/ws"),
        state: ThreadState::Running {
            turn_started_at_ms: 0,
        },
    });
    app.ui.chat_states.insert(
        "thread-opencode-1234".into(),
        ChatState::new("thread-opencode-1234".into(), AgentName::Opencode),
    );

    assert!(
        app.handle_event(AppEvent::Ingest(projected_frame(
            "thread-opencode-1234",
            1,
            AgentName::Opencode,
            vec![
                UiEventMessage::MessageStarted {
                    message_id: "msg-assistant-1".into(),
                    role: MessageRole::Assistant,
                    started_at_ms: 1,
                },
                UiEventMessage::TextDelta {
                    message_id: "msg-assistant-1".into(),
                    text: "在的！有什么可以帮你的？".into(),
                },
            ],
        )))
        .await
    );
    assert!(app.ui.group_chat.messages.is_empty());

    assert!(
        app.handle_event(AppEvent::Ingest(projected_frame(
            "thread-opencode-1234",
            2,
            AgentName::Opencode,
            vec![UiEventMessage::MessageCompleted {
                message_id: "msg-assistant-1".into(),
                finished_at_ms: 2,
            }],
        )))
        .await
    );

    assert_eq!(app.ui.group_chat.messages.len(), 1);
    let message = &app.ui.group_chat.messages[0];
    assert_eq!(message.kind, LocalGroupChatMessageKind::AgentResult);
    assert_eq!(message.text, "在的！有什么可以帮你的？");
    assert_eq!(message.agent, Some(AgentName::Opencode));
    assert_eq!(message.thread_id.as_deref(), Some("thread-opencode-1234"));
}

#[tokio::test]
async fn opencode_manager_idle_does_not_record_partial_result_before_final_snapshot() {
    let temp = tempfile::tempdir().expect("tempdir");
    let group_store = GroupChatStore::at_path(temp.path().join("group.jsonl"));
    let backend = Arc::new(TestBackend::new());
    let mut app = App::with_group_chat_store(backend, false, PathBuf::from("/tmp"), group_store);
    app.ui.threads.push(ThreadEntry {
        thread_id: "thread-opencode-1234".into(),
        agent: AgentName::Opencode,
        workspace: PathBuf::from("/tmp/ws"),
        state: ThreadState::Running {
            turn_started_at_ms: 0,
        },
    });
    app.ui.chat_states.insert(
        "thread-opencode-1234".into(),
        ChatState::new("thread-opencode-1234".into(), AgentName::Opencode),
    );

    assert!(
        app.handle_event(AppEvent::Ingest(projected_frame(
            "thread-opencode-1234",
            1,
            AgentName::Opencode,
            vec![
                UiEventMessage::MessageStarted {
                    message_id: "msg-assistant-1".into(),
                    role: MessageRole::Assistant,
                    started_at_ms: 1,
                },
                UiEventMessage::TextDelta {
                    message_id: "msg-assistant-1".into(),
                    text: "Gemini 说了以下内容：它详细介绍了".into(),
                },
            ],
        )))
        .await
    );
    assert!(app.ui.group_chat.messages.is_empty());

    assert!(
        app.handle_event(AppEvent::ManagerEvent(ManagerEvent::ThreadStateChanged {
            thread_id: "thread-opencode-1234".into(),
            old: ThreadState::Running {
                turn_started_at_ms: 0,
            },
            new: ThreadState::Idle,
            at_ms: 2,
        }))
        .await
    );
    assert!(app.ui.group_chat.messages.is_empty());

    assert!(
        app.handle_event(AppEvent::Ingest(projected_frame(
            "thread-opencode-1234",
            3,
            AgentName::Opencode,
            vec![UiEventMessage::TextReplace {
                message_id: "msg-assistant-1".into(),
                text: "Gemini 说了以下内容：它详细介绍了自己的能力。".into(),
            }],
        )))
        .await
    );
    assert!(app.ui.group_chat.messages.is_empty());

    assert!(
        app.handle_event(AppEvent::Ingest(projected_frame(
            "thread-opencode-1234",
            4,
            AgentName::Opencode,
            vec![UiEventMessage::MessageCompleted {
                message_id: "msg-assistant-1".into(),
                finished_at_ms: 4,
            }],
        )))
        .await
    );

    assert_eq!(app.ui.group_chat.messages.len(), 1);
    assert_eq!(
        app.ui.group_chat.messages[0].text,
        "Gemini 说了以下内容：它详细介绍了自己的能力。"
    );
}
