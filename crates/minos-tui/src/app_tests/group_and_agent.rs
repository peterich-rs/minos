use super::*;

#[tokio::test]
async fn routed_prompt_records_user_message_in_group_chat() {
    let temp = tempfile::tempdir().expect("tempdir");
    let group_store = GroupChatStore::at_path(temp.path().join("group.jsonl"));
    let backend = Arc::new(TestBackend::with_agents(vec![ok_agent(AgentName::Gemini)]));
    let mut app =
        App::with_group_chat_store(backend.clone(), false, PathBuf::from("/tmp"), group_store);
    app.ui.nav_level = crate::nav::NavLevel::Session {
        project_id: "test".into(),
        thread_id: "test".into(),
    };
    app.ui
        .status
        .update_agents(vec![ok_agent(AgentName::Gemini)]);
    app.ui.focus.focus(PaneId::RoomInput);
    app.ui.room_input.content = "@gemini write tests".into();
    app.ui.room_input.cursor_pos = app.ui.room_input.content.len();
    app.sync_input_agent_picker();

    assert!(app.handle_key(press(KeyCode::Enter)).await);

    assert_eq!(
        backend
            .sent_messages
            .lock()
            .expect("sent messages lock")
            .as_slice(),
        &[("thread-1".to_owned(), "write tests".to_owned())]
    );
    assert_eq!(app.ui.group_chat.messages.len(), 1);
    let message = &app.ui.group_chat.messages[0];
    assert_eq!(message.seq, 1);
    assert_eq!(message.kind, LocalGroupChatMessageKind::User);
    assert_eq!(message.text, "@gemini write tests");
    assert_eq!(message.agent, Some(AgentName::Gemini));

    let persisted = app
        .state
        .group_chat_store
        .load_recent(10)
        .await
        .expect("group chat DB should be readable");
    assert_eq!(persisted.len(), 1);
    assert_eq!(persisted[0].kind, LocalGroupChatMessageKind::User);
    assert_eq!(persisted[0].text, "@gemini write tests");
}

#[tokio::test]
async fn routed_prompt_echoes_in_group_chat_before_backend_send_finishes() {
    let temp = tempfile::tempdir().expect("tempdir");
    let group_store = GroupChatStore::at_path(temp.path().join("group.jsonl"));
    let backend =
        Arc::new(TestBackend::with_agents(vec![ok_agent(AgentName::Gemini)]).with_blocked_sends());
    let mut app =
        App::with_group_chat_store(backend.clone(), false, PathBuf::from("/tmp"), group_store);
    let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
    app.set_event_sender(tx);
    app.ui.nav_level = crate::nav::NavLevel::Session {
        project_id: "test".into(),
        thread_id: "thread-gemini-1234".into(),
    };
    app.ui
        .status
        .update_agents(vec![ok_agent(AgentName::Gemini)]);
    app.ui.focus.focus(PaneId::RoomInput);
    app.ui.threads.push(ThreadEntry {
        thread_id: "thread-gemini-1234".into(),
        agent: AgentName::Gemini,
        workspace: PathBuf::from("/tmp"),
        state: ThreadState::Idle,
    });
    app.ui.chat_states.insert(
        "thread-gemini-1234".into(),
        ChatState::new("thread-gemini-1234".into(), AgentName::Gemini),
    );
    app.select_thread(0);
    app.ui.room_input.content = "@gemini#thread-g write tests".into();
    app.ui.room_input.cursor_pos = app.ui.room_input.content.len();
    app.sync_input_agent_picker();

    let handled = tokio::time::timeout(
        std::time::Duration::from_millis(100),
        app.handle_key(press(KeyCode::Enter)),
    )
    .await
    .expect("room submit should not wait for backend send");

    assert!(handled);
    assert_eq!(app.ui.group_chat.messages.len(), 1);
    assert_eq!(
        app.ui.group_chat.messages[0].kind,
        LocalGroupChatMessageKind::User
    );
    assert_eq!(
        app.ui.group_chat.messages[0].text,
        "@gemini#thread-g write tests"
    );
    assert_eq!(
        app.ui.group_chat.messages[0].thread_id.as_deref(),
        Some("thread-gemini-1234")
    );
    assert_eq!(app.ui.room_input.content, "");
}

#[tokio::test]
async fn routed_prompt_echoes_in_group_chat_before_agent_start_finishes() {
    let temp = tempfile::tempdir().expect("tempdir");
    let group_store = GroupChatStore::at_path(temp.path().join("group.jsonl"));
    let backend =
        Arc::new(TestBackend::with_agents(vec![ok_agent(AgentName::Gemini)]).with_blocked_starts());
    let mut app = App::with_group_chat_store(backend, false, PathBuf::from("/tmp"), group_store);
    let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
    app.set_event_sender(tx);
    app.ui.nav_level = crate::nav::NavLevel::Session {
        project_id: "test".into(),
        thread_id: "test".into(),
    };
    app.ui
        .status
        .update_agents(vec![ok_agent(AgentName::Gemini)]);
    app.ui.focus.focus(PaneId::RoomInput);
    app.ui.room_input.content = "@gemini write tests".into();
    app.ui.room_input.cursor_pos = app.ui.room_input.content.len();
    app.sync_input_agent_picker();

    let handled = tokio::time::timeout(
        std::time::Duration::from_millis(100),
        app.handle_key(press(KeyCode::Enter)),
    )
    .await
    .expect("room submit should not wait for agent startup");

    assert!(handled);
    assert_eq!(app.ui.group_chat.messages.len(), 1);
    assert_eq!(
        app.ui.group_chat.messages[0].kind,
        LocalGroupChatMessageKind::User
    );
    assert_eq!(app.ui.group_chat.messages[0].text, "@gemini write tests");
    assert_eq!(app.ui.group_chat.messages[0].agent, Some(AgentName::Gemini));
    assert_eq!(app.ui.group_chat.messages[0].thread_id, None);
    assert_eq!(app.ui.room_input.content, "");
}

#[tokio::test]
async fn agent_started_prompt_event_creates_chat_state_before_sending() {
    let backend = Arc::new(TestBackend::new());
    let mut app = App::new(backend.clone(), false, PathBuf::from("/tmp"));

    assert!(
        app.handle_event(AppEvent::AgentStartedForPrompt {
            agent: AgentName::Gemini,
            thread_id: "thread-gemini-1234".into(),
            cwd: PathBuf::from("/tmp"),
            text: "write tests".into(),
        })
        .await
    );

    assert_eq!(app.ui.threads.len(), 1);
    assert!(app.ui.chat_states.contains_key("thread-gemini-1234"));
    assert_eq!(
        backend
            .sent_messages
            .lock()
            .expect("sent messages lock")
            .as_slice(),
        &[("thread-gemini-1234".to_owned(), "write tests".to_owned())]
    );
}

#[tokio::test]
async fn loading_group_history_restores_current_workspace_agent_list_entries() {
    let temp = tempfile::tempdir().expect("tempdir");
    let group_store = GroupChatStore::at_path(temp.path().join("group.jsonl"));
    group_store
        .append(LocalGroupChatMessage {
            seq: 0,
            message_id: String::new(),
            created_at_ms: 10,
            kind: LocalGroupChatMessageKind::User,
            text: "@codex inspect the repo".into(),
            agent: Some(AgentName::Codex),
            thread_id: Some("thread-codex-1234".into()),
            thread_short_id: Some("thread-c".into()),
            workspace: Some("/tmp/minos-a".into()),
        })
        .await
        .expect("append codex message");
    group_store
        .append(LocalGroupChatMessage {
            seq: 0,
            message_id: String::new(),
            created_at_ms: 20,
            kind: LocalGroupChatMessageKind::AgentResult,
            text: "done".into(),
            agent: Some(AgentName::Gemini),
            thread_id: Some("thread-gemini-5678".into()),
            thread_short_id: Some("thread-g".into()),
            workspace: Some("/tmp/minos-b".into()),
        })
        .await
        .expect("append gemini message");
    let backend = Arc::new(TestBackend::new());
    let mut app =
        App::with_group_chat_store(backend, false, PathBuf::from("/tmp/minos-a"), group_store);

    app.load_group_chat_history().await;

    assert_eq!(app.ui.group_chat.messages.len(), 1);
    assert_eq!(app.ui.threads.len(), 1);
    assert_eq!(app.ui.selected_thread, Some(0));
    assert_eq!(app.ui.threads[0].thread_id, "thread-codex-1234");
    assert_eq!(app.ui.threads[0].agent, AgentName::Codex);
    assert_eq!(app.ui.threads[0].workspace, PathBuf::from("/tmp/minos-a"));
    assert!(matches!(
        app.ui.threads[0].state,
        ThreadState::Suspended {
            reason: minos_agent_runtime::PauseReason::DaemonRestart
        }
    ));
    assert!(app.ui.chat_states.contains_key("thread-codex-1234"));
    assert!(!app.ui.chat_states.contains_key("thread-gemini-5678"));
}

#[tokio::test]
async fn daemon_group_history_loads_from_backend_and_restores_agent_entries() {
    let backend = Arc::new(
        TestBackend::new()
            .with_connection_state(BackendConnectionState::Connected {
                endpoint: "ws://127.0.0.1:1".into(),
            })
            .with_group_chat_pages(vec![vec![LocalGroupChatMessage {
                seq: 7,
                message_id: "m-daemon-1".into(),
                created_at_ms: 20,
                kind: LocalGroupChatMessageKind::User,
                text: "@opencode inspect this".into(),
                agent: Some(AgentName::Opencode),
                thread_id: Some("thread-opencode-1234".into()),
                thread_short_id: Some("thread-o".into()),
                workspace: Some("/tmp/daemon-ws".into()),
            }]]),
    );
    let mut app = App::new(backend, false, PathBuf::from("/tmp/daemon-ws"));

    app.load_group_chat_history().await;

    assert_eq!(app.ui.group_chat.messages.len(), 1);
    assert_eq!(app.ui.group_chat.messages[0].seq, 7);
    assert_eq!(app.ui.threads.len(), 1);
    assert_eq!(app.ui.threads[0].thread_id, "thread-opencode-1234");
    assert_eq!(app.ui.threads[0].agent, AgentName::Opencode);
    assert_eq!(app.ui.threads[0].workspace, PathBuf::from("/tmp/daemon-ws"));
}

#[tokio::test]
async fn init_filters_daemon_threads_to_current_workspace() {
    let backend = Arc::new(
        TestBackend::new()
            .with_connection_state(BackendConnectionState::Connected {
                endpoint: "ws://127.0.0.1:1".into(),
            })
            .with_listed_threads(vec![
                BackendThreadSnapshot {
                    thread_id: "thread-fire-1234".into(),
                    agent: Some(AgentName::Opencode),
                    workspace: PathBuf::from("/tmp/fire"),
                    state: ThreadState::Idle,
                },
                BackendThreadSnapshot {
                    thread_id: "thread-minos-1234".into(),
                    agent: Some(AgentName::Codex),
                    workspace: PathBuf::from("/tmp/Minos"),
                    state: ThreadState::Idle,
                },
            ]),
    );
    let mut app = App::new(backend.clone(), false, PathBuf::from("/tmp/fire"));

    app.init().await.expect("init should succeed");

    assert_eq!(app.ui.threads.len(), 1);
    assert_eq!(app.ui.threads[0].thread_id, "thread-fire-1234");
    assert_eq!(app.ui.threads[0].workspace, PathBuf::from("/tmp/fire"));
    assert!(app.ui.chat_states.contains_key("thread-fire-1234"));
    assert!(!app.ui.chat_states.contains_key("thread-minos-1234"));
    assert_eq!(
        backend
            .history_calls
            .lock()
            .expect("history calls lock")
            .as_slice(),
        &[("thread-fire-1234".to_owned(), None, 1000)]
    );
}

#[tokio::test]
async fn manager_thread_added_ignores_other_workspace() {
    let backend = Arc::new(TestBackend::new());
    let mut app = App::new(backend, false, PathBuf::from("/tmp/fire"));

    assert!(
        !app.handle_event(AppEvent::ManagerEvent(ManagerEvent::ThreadAdded {
            thread_id: "thread-minos-1234".into(),
            workspace: PathBuf::from("/tmp/Minos"),
            agent: AgentName::Codex,
        }))
        .await
    );

    assert!(app.ui.threads.is_empty());
    assert!(app.ui.chat_states.is_empty());
}

#[tokio::test]
async fn agent_input_group_echo_includes_existing_thread_short_id() {
    let temp = tempfile::tempdir().expect("tempdir");
    let group_store = GroupChatStore::at_path(temp.path().join("group.jsonl"));
    let backend = Arc::new(TestBackend::new());
    let mut app = App::with_group_chat_store(
        backend.clone(),
        false,
        PathBuf::from("/tmp/ws"),
        group_store,
    );
    app.ui.nav_level = crate::nav::NavLevel::Session {
        project_id: "test".into(),
        thread_id: "thread-codex-1234".into(),
    };
    app.ui.threads.push(ThreadEntry {
        thread_id: "thread-codex-1234".into(),
        agent: AgentName::Codex,
        workspace: PathBuf::from("/tmp/ws"),
        state: ThreadState::Idle,
    });
    app.ui.chat_states.insert(
        "thread-codex-1234".into(),
        ChatState::new("thread-codex-1234".into(), AgentName::Codex),
    );
    app.select_thread(0);
    app.ui.agent_detail_visible = true;
    app.ui.focus.switch_layout(true);
    app.ui.focus.focus(PaneId::AgentInput);
    app.ui.agent_input.content = "continue".into();
    app.ui.agent_input.cursor_pos = app.ui.agent_input.content.len();

    assert!(app.handle_key(press(KeyCode::Enter)).await);

    assert_eq!(
        backend
            .sent_messages
            .lock()
            .expect("sent messages lock")
            .as_slice(),
        &[("thread-codex-1234".to_owned(), "continue".to_owned())]
    );
    assert_eq!(app.ui.group_chat.messages.len(), 1);
    let message = &app.ui.group_chat.messages[0];
    assert_eq!(message.text, "@codex#thread-c continue");
    assert_eq!(message.thread_id.as_deref(), Some("thread-codex-1234"));
    assert_eq!(message.thread_short_id.as_deref(), Some("thread-c"));
}

#[tokio::test]
async fn agent_input_answers_pending_question_without_group_echo_or_prompt() {
    let backend = Arc::new(TestBackend::new());
    let mut app = App::new(backend.clone(), false, PathBuf::from("/tmp"));
    app.ui.nav_level = crate::nav::NavLevel::Session {
        project_id: "test".into(),
        thread_id: "thread-codex-1234".into(),
    };
    app.ui.threads.push(ThreadEntry {
        thread_id: "thread-codex-1234".into(),
        agent: AgentName::Codex,
        workspace: PathBuf::from("/tmp"),
        state: ThreadState::Running {
            turn_started_at_ms: 0,
        },
    });
    let mut chat = ChatState::new("thread-codex-1234".into(), AgentName::Codex);
    chat.apply_ui_events(vec![UiEventMessage::Raw {
        kind: "approval/request".into(),
        payload_json: serde_json::json!({
            "request_id": "req-1",
            "method": "item/tool/requestUserInput",
            "params": {
                "questions": [{
                    "header": "Need input",
                    "id": "q1",
                    "question": "Pick one"
                }]
            }
        })
        .to_string(),
    }]);
    app.ui.chat_states.insert("thread-codex-1234".into(), chat);
    app.select_thread(0);
    app.ui.agent_detail_visible = true;
    app.ui.focus.switch_layout(true);
    app.ui.focus.focus(PaneId::AgentInput);
    app.ui.agent_input.content = "blue".into();
    app.ui.agent_input.cursor_pos = app.ui.agent_input.content.len();

    assert!(app.handle_key(press(KeyCode::Enter)).await);

    assert!(app.ui.group_chat.messages.is_empty());
    assert!(backend
        .sent_messages
        .lock()
        .expect("sent messages lock")
        .is_empty());
    let decisions = backend
        .approval_decisions
        .lock()
        .expect("approval decisions lock");
    assert_eq!(decisions.len(), 1);
    assert_eq!(decisions[0].0, "req-1");
    assert_eq!(decisions[0].1, "thread-codex-1234");
    assert_eq!(
        decisions[0].2,
        serde_json::json!({ "answers": { "q1": { "answers": ["blue"] } } })
    );
    assert!(app
        .ui
        .chat_states
        .get("thread-codex-1234")
        .expect("chat state")
        .pending_requests
        .is_empty());
}

#[tokio::test]
async fn agent_input_answers_opencode_permission() {
    let backend = Arc::new(TestBackend::new());
    let mut app = App::new(backend.clone(), false, PathBuf::from("/tmp"));
    app.ui.nav_level = crate::nav::NavLevel::Session {
        project_id: "test".into(),
        thread_id: "thread-opencode-1234".into(),
    };
    app.ui.threads.push(ThreadEntry {
        thread_id: "thread-opencode-1234".into(),
        agent: AgentName::Opencode,
        workspace: PathBuf::from("/tmp"),
        state: ThreadState::Running {
            turn_started_at_ms: 0,
        },
    });
    let mut chat = ChatState::new("thread-opencode-1234".into(), AgentName::Opencode);
    chat.apply_ui_events(vec![UiEventMessage::Raw {
        kind: "opencode/permission.updated".into(),
        payload_json: serde_json::json!({
            "permissionID": "perm-1",
            "title": "Run shell",
            "options": [
                {"optionId": "proceed_once", "kind": "allow_once"},
                {"optionId": "cancel", "kind": "reject_once"}
            ]
        })
        .to_string(),
    }]);
    app.ui
        .chat_states
        .insert("thread-opencode-1234".into(), chat);
    app.select_thread(0);
    app.ui.agent_detail_visible = true;
    app.ui.focus.switch_layout(true);
    app.ui.focus.focus(PaneId::AgentInput);
    app.ui.agent_input.content = "yes".into();
    app.ui.agent_input.cursor_pos = app.ui.agent_input.content.len();

    assert!(app.handle_key(press(KeyCode::Enter)).await);

    assert!(app.ui.group_chat.messages.is_empty());
    let responses = backend
        .opencode_permission_responses
        .lock()
        .expect("permission responses lock");
    assert_eq!(
        responses.as_slice(),
        &[(
            "thread-opencode-1234".to_owned(),
            "perm-1".to_owned(),
            "proceed_once".to_owned()
        )]
    );
}

#[tokio::test]
async fn agent_input_answers_opencode_question_with_selected_option() {
    let backend = Arc::new(TestBackend::new());
    let mut app = App::new(backend.clone(), false, PathBuf::from("/tmp"));
    app.ui.nav_level = crate::nav::NavLevel::Session {
        project_id: "test".into(),
        thread_id: "thread-opencode-1234".into(),
    };
    app.ui.threads.push(ThreadEntry {
        thread_id: "thread-opencode-1234".into(),
        agent: AgentName::Opencode,
        workspace: PathBuf::from("/tmp"),
        state: ThreadState::Running {
            turn_started_at_ms: 0,
        },
    });
    let mut chat = ChatState::new("thread-opencode-1234".into(), AgentName::Opencode);
    chat.apply_ui_events(vec![UiEventMessage::Raw {
        kind: "opencode/question.asked".into(),
        payload_json: serde_json::json!({
            "type": "question.asked",
            "properties": {
                "id": "que-1",
                "questions": [{
                    "header": "Core",
                    "question": "Pick a direction",
                    "options": [
                        {"label": "Fast", "description": "Ship quickly"},
                        {"label": "Robust", "description": "Prefer durability"}
                    ]
                }]
            }
        })
        .to_string(),
    }]);
    app.ui
        .chat_states
        .insert("thread-opencode-1234".into(), chat);
    app.select_thread(0);
    app.ui.agent_detail_visible = true;
    app.ui.focus.switch_layout(true);
    app.ui.focus.focus(PaneId::AgentInput);
    app.ui.agent_input.content = "2".into();
    app.ui.agent_input.cursor_pos = app.ui.agent_input.content.len();

    assert!(app.handle_key(press(KeyCode::Enter)).await);

    assert!(app.ui.group_chat.messages.is_empty());
    let responses = backend
        .opencode_question_responses
        .lock()
        .expect("question responses lock");
    assert_eq!(
        responses.as_slice(),
        &[(
            "thread-opencode-1234".to_owned(),
            "que-1".to_owned(),
            vec![vec!["Robust".to_owned()]]
        )]
    );
    assert!(app
        .ui
        .chat_states
        .get("thread-opencode-1234")
        .expect("chat state")
        .pending_requests
        .is_empty());
}
