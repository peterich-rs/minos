use super::*;

impl App {
    pub(super) async fn handle_key(&mut self, key: KeyEvent) -> bool {
        if matches!(key.kind, KeyEventKind::Release) {
            return false;
        }

        match event_mapping::key_to_mapping(&self.ui, key) {
            event_mapping::KeyMapping::Actions(actions) => self.apply_actions(actions).await,
            event_mapping::KeyMapping::Input(target) => {
                self.handle_input_key_via_action(key, target).await
            }
            event_mapping::KeyMapping::ClipboardPaste => {
                if let Ok(text) = paste_from_clipboard() {
                    self.handle_paste(text).await
                } else {
                    false
                }
            }
            event_mapping::KeyMapping::None => false,
        }
    }

    pub(super) async fn apply_actions(&mut self, actions: Vec<Action>) -> bool {
        let mut needs_redraw = false;
        for action in actions {
            needs_redraw |= self.apply_action(action).await;
        }
        needs_redraw
    }

    pub(super) async fn handle_input_key_via_action(
        &mut self,
        key: KeyEvent,
        target: InputTarget,
    ) -> bool {
        let (input, area) = match target {
            InputTarget::Room => (&self.ui.room_input, self.ui.panel_areas.room_input),
            InputTarget::Agent => (&self.ui.agent_input, self.ui.panel_areas.agent_input),
        };
        let width = area.width.saturating_sub(2).max(1);
        let Some(action) = crate::input::key_to_input_action(key, input, width, target) else {
            return self.handle_unmapped_input_key(key).await;
        };

        let sync_agent_picker = matches!(target, InputTarget::Room)
            && room_input_action_needs_agent_picker_sync(&action);
        let (change, effects) =
            crate::update::update(&mut self.state, &mut self.ui, Action::Input(target, action));
        if sync_agent_picker {
            self.sync_input_agent_picker();
        }
        let effects_redraw = self.execute_effects(effects).await;
        let needs_redraw = change.needs_redraw || effects_redraw;
        if needs_redraw {
            self.request_frame();
        }
        needs_redraw
    }

    pub(super) async fn handle_unmapped_input_key(&mut self, key: KeyEvent) -> bool {
        match key.code {
            KeyCode::Tab => {
                self.apply_action(Action::Global(GlobalAction::CycleFocus))
                    .await
            }
            KeyCode::BackTab => {
                self.apply_action(Action::Global(GlobalAction::CycleFocusPrev))
                    .await
            }
            KeyCode::Esc => {
                self.apply_action(Action::Global(GlobalAction::Escape))
                    .await
            }
            _ => false,
        }
    }

    pub(super) async fn execute_effects(&mut self, effects: Vec<Effect>) -> bool {
        let mut redraw = false;
        for effect in effects {
            redraw |= self.execute_effect(effect).await;
        }
        redraw
    }

    async fn execute_non_recursive_effects(&mut self, effects: Vec<Effect>) -> bool {
        let mut redraw = false;
        for effect in effects {
            match effect {
                Effect::AgentStartedForPrompt {
                    agent,
                    thread_id,
                    cwd,
                    text,
                } => {
                    self.ensure_thread_visible(thread_id.clone(), agent, cwd);
                    if !text.trim().is_empty() {
                        redraw |= self.send_text_to_thread(thread_id, text, None).await;
                    }
                }
                _ => {
                    tracing::warn!(
                        target: "minos_tui::app",
                        "unexpected follow-up effect in synchronous conversation fallback"
                    );
                }
            }
        }
        redraw
    }

    pub(super) async fn execute_effect(&mut self, effect: Effect) -> bool {
        match effect {
            Effect::Quit => {
                self.should_quit = true;
                false
            }
            Effect::InterruptOrQuit => self.handle_ctrl_c().await,
            Effect::CloseCurrentThread => self.close_current_thread().await,
            Effect::StartAgentAt(index) => self.start_agent_at(index).await,
            Effect::HandleIngest(ingest) => self.handle_ingest(ingest).await,
            Effect::HandleManagerEvent(event) => self.handle_manager_event(event).await,
            Effect::HandleTick => self.handle_tick().await,
            Effect::HandleMcpToolCall(event) => {
                let response = self.handle_mcp_tool_call(event.request).await;
                let _ = event.response_tx.send(response);
                true
            }
            Effect::AgentStartedForPrompt {
                agent,
                thread_id,
                cwd,
                text,
            } => {
                self.ensure_thread_visible(thread_id.clone(), agent, cwd);
                if text.trim().is_empty() {
                    true
                } else {
                    self.send_text_to_thread(thread_id, text, None).await
                }
            }
            Effect::DispatchPromptToExistingAgent {
                agent,
                thread_short_id,
                text,
                group_text,
            } => {
                self.dispatch_prompt_to_existing_agent(agent, thread_short_id, text, group_text)
                    .await
            }
            Effect::InviteAgentToRoom { agent, group_text } => {
                self.invite_agent_to_room(agent, group_text).await
            }
            Effect::DispatchPromptToAgent {
                agent,
                text,
                group_text,
            } => self.dispatch_prompt_to_agent(agent, text, group_text).await,
            Effect::SendTextToThread {
                thread_id,
                text,
                group_text,
            } => self.send_text_to_thread(thread_id, text, group_text).await,
            Effect::SubmitPendingAgentRequest {
                thread_id,
                pending,
                text,
            } => {
                self.submit_pending_agent_request(thread_id, pending, text)
                    .await
            }
            Effect::ConfirmDeleteThread => self.confirm_delete_thread().await,
            Effect::CopyToClipboard(text) => {
                if let Err(error) = copy_to_clipboard(&text) {
                    self.ui
                        .set_error(format!("Failed to copy selection: {error}"));
                } else {
                    self.ui.flash_copied();
                }
                true
            }
            Effect::CreateProject {
                name,
                workspace_path,
            } => {
                if let Some(tx) = self.event_tx.clone() {
                    let backend = Arc::clone(&self.backend);
                    tokio::spawn(async move {
                        tracing::info!(
                            target: "minos_tui::app",
                            project_name = %name,
                            workspace = %workspace_path.display(),
                            "create_project requested",
                        );
                        let result = backend.create_project(&name, &workspace_path).await;
                        let _ = tx.send(match result {
                            Ok(project) => {
                                tracing::info!(
                                    target: "minos_tui::app",
                                    project_id = %project.project_id,
                                    project_name = %project.name,
                                    workspace = %project.workspace_path.display(),
                                    "create_project succeeded",
                                );
                                AppEvent::ProjectCreated(project)
                            }
                            Err(e) => {
                                tracing::warn!(
                                    target: "minos_tui::app",
                                    error = ?e,
                                    project_name = %name,
                                    workspace = %workspace_path.display(),
                                    "create_project failed"
                                );
                                AppEvent::ProjectFailed(e.to_string())
                            }
                        });
                    });
                }
                false
            }
            Effect::LoadConversations { project_id } => {
                if let Some(tx) = self.event_tx.clone() {
                    let backend = Arc::clone(&self.backend);
                    tokio::spawn(async move {
                        let result = backend.list_conversations(&project_id).await;
                        let _ = tx.send(match result {
                            Ok(conversations) => AppEvent::ConversationsLoaded {
                                project_id,
                                conversations,
                            },
                            Err(e) => AppEvent::ProjectFailed(e.to_string()),
                        });
                    });
                }
                false
            }
            Effect::CreateConversationAndStartAgent {
                project_id,
                agent,
                workspace,
                message_body,
                prompt,
            } => {
                if let Some(tx) = self.event_tx.clone() {
                    let backend = Arc::clone(&self.backend);
                    tokio::spawn(async move {
                        let title = prompt
                            .lines()
                            .next()
                            .map(str::trim)
                            .filter(|line| !line.is_empty())
                            .unwrap_or("Untitled conversation")
                            .chars()
                            .take(80)
                            .collect::<String>();
                        let conversation =
                            match backend.create_conversation(&project_id, &title).await {
                                Ok(conversation) => conversation,
                                Err(e) => {
                                    let _ = tx.send(AppEvent::ProjectFailed(e.to_string()));
                                    return;
                                }
                            };
                        let conversation_id = conversation.conversation_id.clone();
                        let _ = backend
                            .append_conversation_message(
                                &conversation_id,
                                None,
                                "user",
                                None,
                                &message_body,
                            )
                            .await;
                        let messages = backend
                            .list_conversation_messages(&conversation_id)
                            .await
                            .unwrap_or_default();
                        let sessions = backend
                            .list_conversation_agent_sessions(&conversation_id)
                            .await
                            .unwrap_or_default();
                        let _ = tx.send(AppEvent::ConversationOpened {
                            project_id: project_id.clone(),
                            conversation_id: conversation_id.clone(),
                            messages,
                            sessions,
                        });
                        match backend
                            .start_agent_in_conversation(&conversation_id, agent, workspace)
                            .await
                        {
                            Ok(outcome) => {
                                let _ = tx.send(AppEvent::ConversationAgentStarted {
                                    conversation_id,
                                    agent,
                                    thread_id: outcome.thread_id,
                                    cwd: outcome.cwd,
                                    text: prompt,
                                });
                            }
                            Err(e) => {
                                let _ = tx.send(AppEvent::ProjectFailed(e.to_string()));
                            }
                        }
                    });
                    return false;
                }
                let title = conversation_title_from_prompt(&prompt);
                let conversation = match self.backend.create_conversation(&project_id, &title).await
                {
                    Ok(conversation) => conversation,
                    Err(error) => {
                        self.ui.set_error(error.to_string());
                        return true;
                    }
                };
                let conversation_id = conversation.conversation_id.clone();
                let _ = self
                    .backend
                    .append_conversation_message(
                        &conversation_id,
                        None,
                        "user",
                        None,
                        &message_body,
                    )
                    .await;
                let messages = self
                    .backend
                    .list_conversation_messages(&conversation_id)
                    .await
                    .unwrap_or_default();
                let sessions = self
                    .backend
                    .list_conversation_agent_sessions(&conversation_id)
                    .await
                    .unwrap_or_default();
                let (change, effects) = crate::update::update(
                    &mut self.state,
                    &mut self.ui,
                    Action::EffectCompleted(crate::action::EffectResult::ConversationOpened {
                        project_id: project_id.clone(),
                        conversation_id: conversation_id.clone(),
                        messages,
                        sessions,
                    }),
                );
                debug_assert!(effects.is_empty());
                let mut redraw = change.needs_redraw;
                match self
                    .backend
                    .start_agent_in_conversation(&conversation_id, agent, workspace)
                    .await
                {
                    Ok(outcome) => {
                        let (change, effects) = crate::update::update(
                            &mut self.state,
                            &mut self.ui,
                            Action::EffectCompleted(
                                crate::action::EffectResult::ConversationAgentStarted {
                                    conversation_id,
                                    agent,
                                    thread_id: outcome.thread_id,
                                    cwd: outcome.cwd,
                                    text: prompt,
                                },
                            ),
                        );
                        redraw |= change.needs_redraw;
                        redraw |= self.execute_non_recursive_effects(effects).await;
                    }
                    Err(error) => {
                        self.ui.set_error(error.to_string());
                        redraw = true;
                    }
                }
                redraw
            }
            Effect::StartAgentInConversation {
                project_id,
                conversation_id,
                agent,
                workspace,
                message_body,
                prompt,
            } => {
                if let Some(tx) = self.event_tx.clone() {
                    let backend = Arc::clone(&self.backend);
                    tokio::spawn(async move {
                        let _ = backend
                            .append_conversation_message(
                                &conversation_id,
                                None,
                                "user",
                                None,
                                &message_body,
                            )
                            .await;
                        let messages = backend
                            .list_conversation_messages(&conversation_id)
                            .await
                            .unwrap_or_default();
                        let sessions = backend
                            .list_conversation_agent_sessions(&conversation_id)
                            .await
                            .unwrap_or_default();
                        let _ = tx.send(AppEvent::ConversationOpened {
                            project_id,
                            conversation_id: conversation_id.clone(),
                            messages,
                            sessions,
                        });
                        match backend
                            .start_agent_in_conversation(&conversation_id, agent, workspace)
                            .await
                        {
                            Ok(outcome) => {
                                let _ = tx.send(AppEvent::ConversationAgentStarted {
                                    conversation_id,
                                    agent,
                                    thread_id: outcome.thread_id,
                                    cwd: outcome.cwd,
                                    text: prompt,
                                });
                            }
                            Err(e) => {
                                let _ = tx.send(AppEvent::ProjectFailed(e.to_string()));
                            }
                        }
                    });
                    return false;
                }
                let _ = self
                    .backend
                    .append_conversation_message(
                        &conversation_id,
                        None,
                        "user",
                        None,
                        &message_body,
                    )
                    .await;
                let messages = self
                    .backend
                    .list_conversation_messages(&conversation_id)
                    .await
                    .unwrap_or_default();
                let sessions = self
                    .backend
                    .list_conversation_agent_sessions(&conversation_id)
                    .await
                    .unwrap_or_default();
                let (change, effects) = crate::update::update(
                    &mut self.state,
                    &mut self.ui,
                    Action::EffectCompleted(crate::action::EffectResult::ConversationOpened {
                        project_id,
                        conversation_id: conversation_id.clone(),
                        messages,
                        sessions,
                    }),
                );
                debug_assert!(effects.is_empty());
                let mut redraw = change.needs_redraw;
                match self
                    .backend
                    .start_agent_in_conversation(&conversation_id, agent, workspace)
                    .await
                {
                    Ok(outcome) => {
                        let (change, effects) = crate::update::update(
                            &mut self.state,
                            &mut self.ui,
                            Action::EffectCompleted(
                                crate::action::EffectResult::ConversationAgentStarted {
                                    conversation_id,
                                    agent,
                                    thread_id: outcome.thread_id,
                                    cwd: outcome.cwd,
                                    text: prompt,
                                },
                            ),
                        );
                        redraw |= change.needs_redraw;
                        redraw |= self.execute_non_recursive_effects(effects).await;
                    }
                    Err(error) => {
                        self.ui.set_error(error.to_string());
                        redraw = true;
                    }
                }
                redraw
            }
            Effect::OpenConversation { conversation_id } => {
                if let Some(tx) = self.event_tx.clone() {
                    let backend = Arc::clone(&self.backend);
                    let project_id = self
                        .ui
                        .nav_level()
                        .project_id()
                        .map(str::to_owned)
                        .unwrap_or_default();
                    tokio::spawn(async move {
                        let messages =
                            match backend.list_conversation_messages(&conversation_id).await {
                                Ok(messages) => messages,
                                Err(e) => {
                                    let _ = tx.send(AppEvent::ProjectFailed(e.to_string()));
                                    return;
                                }
                            };
                        let sessions = match backend
                            .list_conversation_agent_sessions(&conversation_id)
                            .await
                        {
                            Ok(sessions) => sessions,
                            Err(e) => {
                                let _ = tx.send(AppEvent::ProjectFailed(e.to_string()));
                                return;
                            }
                        };
                        let _ = tx.send(AppEvent::ConversationOpened {
                            project_id,
                            conversation_id,
                            messages,
                            sessions,
                        });
                    });
                }
                false
            }
            Effect::OpenAgentSession { thread_id } => {
                self.ensure_conversation_agent_session_visible(&thread_id)
                    .await;
                true
            }
        }
    }

    pub(super) async fn apply_action(&mut self, action: Action) -> bool {
        let (change, effects) = crate::update::update(&mut self.state, &mut self.ui, action);
        let effects_redraw = self.execute_effects(effects).await;
        let needs_redraw = change.needs_redraw || effects_redraw;
        if needs_redraw {
            self.request_frame();
        }
        needs_redraw
    }

    pub(super) async fn handle_paste(&mut self, text: String) -> bool {
        let text = crate::input::normalize_pasted_text(text.as_str());
        if text.is_empty() {
            return false;
        }

        let target = match self.ui.focus.current() {
            PaneId::Input
                if matches!(
                    self.ui.nav_level(),
                    crate::nav::NavLevel::AgentDetail { .. }
                ) =>
            {
                InputTarget::Agent
            }
            PaneId::Input => InputTarget::Room,
            _ => return false,
        };
        let sync_agent_picker = matches!(target, InputTarget::Room);
        let (change, effects) = crate::update::update(
            &mut self.state,
            &mut self.ui,
            Action::Input(target, InputAction::InsertText(text)),
        );
        debug_assert!(effects.is_empty());
        if sync_agent_picker {
            self.sync_input_agent_picker();
        }
        if change.needs_redraw {
            self.request_frame();
        }
        change.needs_redraw
    }

    pub(super) async fn handle_ctrl_c(&mut self) -> bool {
        if self.ui.focus.is(PaneId::MainChat) {
            if let Some(chat) = self.ui.current_chat() {
                if chat.selection.is_some() {
                    let width = self.ui.panel_areas.agent_chat.width.saturating_sub(2);
                    if let Some(text) =
                        crate::ui::chat::selected_text(chat, width, &self.ui.render_cache)
                    {
                        let _ = copy_to_clipboard(&text);
                        self.ui.flash_copied();
                    }
                    return true;
                }
            }
        }

        if self.current_thread_is_interruptible() {
            if let Some(thread_id) = self.ui.current_thread_id().map(String::from) {
                if let Err(error) = self.backend.interrupt_thread(&thread_id).await {
                    self.ui
                        .set_error(format!("Failed to interrupt thread: {error}"));
                }
                return true;
            }
        }

        self.should_quit = true;
        false
    }

    pub(super) async fn handle_mouse(&mut self, mouse: MouseEvent) -> bool {
        if state::current_chat_selection_active(&self.ui) {
            match mouse.kind {
                MouseEventKind::Drag(MouseButton::Left) | MouseEventKind::Up(MouseButton::Left) => {
                    return self
                        .apply_action(Action::Global(GlobalAction::MouseDrag {
                            x: mouse.column,
                            y: mouse.row,
                            release: matches!(mouse.kind, MouseEventKind::Up(MouseButton::Left)),
                        }))
                        .await;
                }
                _ => {}
            }
        }

        if rect_contains(self.ui.panel_areas.room_list, mouse.column, mouse.row) {
            return match mouse.kind {
                MouseEventKind::ScrollUp => {
                    self.apply_action(Action::Global(GlobalAction::MouseScroll {
                        target: ScrollTarget::RoomList,
                        direction: ScrollDirection::Up,
                    }))
                    .await
                }
                MouseEventKind::ScrollDown => {
                    self.apply_action(Action::Global(GlobalAction::MouseScroll {
                        target: ScrollTarget::RoomList,
                        direction: ScrollDirection::Down,
                    }))
                    .await
                }
                MouseEventKind::Down(MouseButton::Left) => {
                    self.apply_action(Action::Global(GlobalAction::MouseClick {
                        target: crate::action::ClickTarget::RoomList,
                        x: mouse.column,
                        y: mouse.row,
                    }))
                    .await
                }
                _ => false,
            };
        }

        if rect_contains(self.ui.panel_areas.room_chat, mouse.column, mouse.row) {
            return match mouse.kind {
                MouseEventKind::ScrollUp => {
                    self.apply_action(Action::Global(GlobalAction::MouseScroll {
                        target: ScrollTarget::GroupChat,
                        direction: ScrollDirection::Up,
                    }))
                    .await
                }
                MouseEventKind::ScrollDown => {
                    self.apply_action(Action::Global(GlobalAction::MouseScroll {
                        target: ScrollTarget::GroupChat,
                        direction: ScrollDirection::Down,
                    }))
                    .await
                }
                MouseEventKind::Down(MouseButton::Left) => {
                    self.apply_action(Action::Global(GlobalAction::MouseClick {
                        target: crate::action::ClickTarget::GroupChat,
                        x: mouse.column,
                        y: mouse.row,
                    }))
                    .await
                }
                _ => false,
            };
        }

        if rect_contains(self.ui.panel_areas.agent_list, mouse.column, mouse.row) {
            return match mouse.kind {
                MouseEventKind::ScrollUp => {
                    self.apply_action(Action::Global(GlobalAction::MouseScroll {
                        target: ScrollTarget::AgentList,
                        direction: ScrollDirection::Up,
                    }))
                    .await
                }
                MouseEventKind::ScrollDown => {
                    self.apply_action(Action::Global(GlobalAction::MouseScroll {
                        target: ScrollTarget::AgentList,
                        direction: ScrollDirection::Down,
                    }))
                    .await
                }
                MouseEventKind::Down(MouseButton::Left) => {
                    self.apply_action(Action::Global(GlobalAction::MouseClick {
                        target: crate::action::ClickTarget::AgentList,
                        x: mouse.column,
                        y: mouse.row,
                    }))
                    .await
                }
                _ => false,
            };
        }

        if rect_contains(self.ui.panel_areas.agent_chat, mouse.column, mouse.row) {
            return match mouse.kind {
                MouseEventKind::ScrollUp => {
                    self.apply_action(Action::Global(GlobalAction::MouseScroll {
                        target: ScrollTarget::AgentChat,
                        direction: ScrollDirection::Up,
                    }))
                    .await
                }
                MouseEventKind::ScrollDown => {
                    self.apply_action(Action::Global(GlobalAction::MouseScroll {
                        target: ScrollTarget::AgentChat,
                        direction: ScrollDirection::Down,
                    }))
                    .await
                }
                MouseEventKind::Down(MouseButton::Left) => {
                    self.apply_action(Action::Global(GlobalAction::MouseClick {
                        target: crate::action::ClickTarget::AgentChat,
                        x: mouse.column,
                        y: mouse.row,
                    }))
                    .await
                }
                MouseEventKind::Drag(MouseButton::Left) | MouseEventKind::Up(MouseButton::Left) => {
                    self.apply_action(Action::Global(GlobalAction::MouseDrag {
                        x: mouse.column,
                        y: mouse.row,
                        release: matches!(mouse.kind, MouseEventKind::Up(MouseButton::Left)),
                    }))
                    .await
                }
                MouseEventKind::ScrollLeft | MouseEventKind::ScrollRight => false,
                _ => false,
            };
        }

        if rect_contains(self.ui.input_metrics[0].outer, mouse.column, mouse.row) {
            return match mouse.kind {
                MouseEventKind::Down(MouseButton::Left) => {
                    self.apply_action(Action::Global(GlobalAction::MouseClick {
                        target: crate::action::ClickTarget::RoomInput,
                        x: mouse.column,
                        y: mouse.row,
                    }))
                    .await
                }
                _ => false,
            };
        }

        if rect_contains(self.ui.input_metrics[1].outer, mouse.column, mouse.row) {
            return match mouse.kind {
                MouseEventKind::Down(MouseButton::Left) => {
                    self.apply_action(Action::Global(GlobalAction::MouseClick {
                        target: crate::action::ClickTarget::AgentInput,
                        x: mouse.column,
                        y: mouse.row,
                    }))
                    .await
                }
                _ => false,
            };
        }

        match mouse.kind {
            MouseEventKind::ScrollUp => {
                self.apply_action(Action::Global(GlobalAction::MouseScroll {
                    target: ScrollTarget::GroupChat,
                    direction: ScrollDirection::Up,
                }))
                .await
            }
            MouseEventKind::ScrollDown => {
                self.apply_action(Action::Global(GlobalAction::MouseScroll {
                    target: ScrollTarget::GroupChat,
                    direction: ScrollDirection::Down,
                }))
                .await
            }
            MouseEventKind::ScrollLeft | MouseEventKind::ScrollRight => false,
            _ => false,
        }
    }
}

fn conversation_title_from_prompt(prompt: &str) -> String {
    prompt
        .lines()
        .next()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .unwrap_or("Untitled conversation")
        .chars()
        .take(80)
        .collect()
}
