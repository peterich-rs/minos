use super::*;
use ratatui::layout::Rect;

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
            InputTarget::Conversation => (
                &self.ui.inputs.conversation,
                self.ui.panel_areas.conversation_input,
            ),
            InputTarget::Agent => (&self.ui.inputs.agent, self.ui.panel_areas.agent_input),
        };
        let width = area.width.saturating_sub(2).max(1);
        let Some(action) = crate::input::key_to_input_action(key, input, width, target) else {
            return self.handle_unmapped_input_key(key).await;
        };

        let sync_agent_picker = matches!(target, InputTarget::Conversation)
            && conversation_input_action_needs_agent_picker_sync(&action);
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
            Effect::HandleTick => self.handle_tick().await,
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
            Effect::DispatchPromptToAgent {
                agent,
                text,
                message_body,
            } => {
                self.dispatch_prompt_to_agent(agent, text, message_body)
                    .await
            }
            Effect::SendTextToThread {
                thread_id,
                text,
                message_body,
            } => {
                self.send_text_to_thread(thread_id, text, message_body)
                    .await
            }
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
                    self.request_frame_in(crate::ui::UiState::ERROR_FLASH_TTL);
                } else {
                    if let Some(chat) = self.ui.current_chat_mut() {
                        chat.clear_selection();
                    }
                    self.ui.conversation.clear_selection();
                    self.ui.flash_copied();
                    // Clear "copied" indicator when TTL elapses without waiting on tick alone.
                    self.request_frame_in(crate::ui::UiState::COPIED_FLASH_TTL);
                }
                true
            }
            Effect::ResolvePathCandidates {
                target,
                sequence,
                token,
                workspace_root,
            } => {
                if let Some(tx) = self.event_tx.clone() {
                    tokio::task::spawn_blocking(move || {
                        let candidates =
                            crate::path_complete::list_path_candidates(&token, &workspace_root)
                                .unwrap_or_default();
                        let _ = tx.send(AppEvent::PathCandidatesResolved {
                            target,
                            sequence,
                            candidates,
                        });
                    });
                }
                false
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
                self.run_conversation_start_flow(
                    conversation_ops::create_conversation_and_start_agent(
                        Arc::clone(&self.backend),
                        project_id,
                        agent,
                        workspace,
                        message_body,
                        prompt,
                    ),
                )
                .await
            }
            Effect::StartAgentInConversation {
                project_id,
                conversation_id,
                agent,
                workspace,
                message_body,
                prompt,
            } => {
                self.run_conversation_start_flow(
                    conversation_ops::start_agent_in_existing_conversation(
                        Arc::clone(&self.backend),
                        project_id,
                        conversation_id,
                        agent,
                        workspace,
                        message_body,
                        prompt,
                    ),
                )
                .await
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
                        if let Some(session) =
                            conversation_ops::pick_auto_continue_session(&sessions)
                        {
                            if let Err(error) =
                                backend.resume_thread(&session.thread_id, true).await
                            {
                                tracing::warn!(
                                    target: "minos_tui::app",
                                    error = %error,
                                    thread_id = %session.thread_id,
                                    "auto-continue resume_thread failed on open"
                                );
                            }
                        }
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

    async fn run_conversation_start_flow<F>(&mut self, work: F) -> bool
    where
        F: std::future::Future<
                Output = Result<
                    (
                        conversation_ops::OpenedConversation,
                        conversation_ops::StartedAgent,
                    ),
                    String,
                >,
            > + Send
            + 'static,
    {
        if let Some(tx) = self.event_tx.clone() {
            tokio::spawn(async move {
                match work.await {
                    Ok((opened, started)) => {
                        let _ = tx.send(AppEvent::ConversationOpened {
                            project_id: opened.project_id,
                            conversation_id: opened.conversation_id,
                            messages: opened.messages,
                            sessions: opened.sessions,
                        });
                        let _ = tx.send(AppEvent::ConversationAgentStarted {
                            conversation_id: started.conversation_id,
                            agent: started.agent,
                            thread_id: started.thread_id,
                            cwd: started.cwd,
                            text: started.text,
                        });
                    }
                    Err(error) => {
                        let _ = tx.send(AppEvent::ProjectFailed(error));
                    }
                }
            });
            return false;
        }

        match work.await {
            Ok((opened, started)) => {
                let (change, effects) = crate::update::update(
                    &mut self.state,
                    &mut self.ui,
                    Action::EffectCompleted(crate::action::EffectResult::ConversationOpened {
                        project_id: opened.project_id,
                        conversation_id: opened.conversation_id,
                        messages: opened.messages,
                        sessions: opened.sessions,
                    }),
                );
                debug_assert!(effects.is_empty());
                let mut redraw = change.needs_redraw;
                let (change, effects) = crate::update::update(
                    &mut self.state,
                    &mut self.ui,
                    Action::EffectCompleted(
                        crate::action::EffectResult::ConversationAgentStarted {
                            conversation_id: started.conversation_id,
                            agent: started.agent,
                            thread_id: started.thread_id,
                            cwd: started.cwd,
                            text: started.text,
                        },
                    ),
                );
                redraw |= change.needs_redraw;
                redraw |= self.execute_non_recursive_effects(effects).await;
                redraw
            }
            Err(error) => {
                self.ui.set_error(error);
                true
            }
        }
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
            PaneId::Input => InputTarget::Conversation,
            _ => return false,
        };
        let sync_agent_picker = matches!(target, InputTarget::Conversation);
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
            // Prefer active selection in either conversation timeline or agent detail.
            if state::conversation_selection_active(&self.ui) {
                let width = self
                    .ui
                    .panel_areas
                    .conversation_chat
                    .width
                    .saturating_sub(2);
                let revision = self.ui.conversation.messages_revision;
                let selection = self.ui.conversation.selection.clone();
                let text = selection.and_then(|selection| {
                    let conversation = &mut self.ui.conversation;
                    conversation.chat_cache.selected_text(
                        &conversation.messages,
                        width,
                        revision,
                        &selection,
                    )
                });
                if let Some(text) = text {
                    if copy_to_clipboard(&text).is_ok() {
                        self.ui.conversation.clear_selection();
                        self.ui.flash_copied();
                    }
                    return true;
                }
            }

            let selected_text = self.ui.current_chat().and_then(|chat| {
                chat.selection.as_ref()?;
                let width = self.ui.panel_areas.agent_chat.width.saturating_sub(2);
                crate::ui::chat::selected_text(chat, width, &self.ui.render_cache)
            });
            if let Some(text) = selected_text {
                if copy_to_clipboard(&text).is_ok() {
                    if let Some(chat) = self.ui.current_chat_mut() {
                        chat.clear_selection();
                    }
                    self.ui.flash_copied();
                }
                return true;
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

        struct MouseHit {
            area: Rect,
            click: Option<crate::action::ClickTarget>,
            scroll: Option<ScrollTarget>,
            allow_drag: bool,
        }

        let hits = [
            MouseHit {
                area: self.ui.panel_areas.main_list,
                click: Some(crate::action::ClickTarget::MainList),
                scroll: Some(ScrollTarget::MainList),
                allow_drag: false,
            },
            MouseHit {
                area: self.ui.panel_areas.conversation_chat,
                click: Some(crate::action::ClickTarget::ConversationChat),
                scroll: Some(ScrollTarget::ConversationChat),
                allow_drag: true,
            },
            MouseHit {
                area: self.ui.panel_areas.agent_list,
                click: Some(crate::action::ClickTarget::AgentList),
                scroll: Some(ScrollTarget::AgentList),
                allow_drag: false,
            },
            MouseHit {
                area: self.ui.panel_areas.agent_chat,
                click: Some(crate::action::ClickTarget::AgentChat),
                scroll: Some(ScrollTarget::AgentChat),
                allow_drag: true,
            },
            MouseHit {
                area: self.ui.inputs.metrics[0].outer,
                click: Some(crate::action::ClickTarget::ConversationInput),
                scroll: None,
                allow_drag: false,
            },
            MouseHit {
                area: self.ui.inputs.metrics[1].outer,
                click: Some(crate::action::ClickTarget::AgentInput),
                scroll: None,
                allow_drag: false,
            },
        ];

        for hit in hits {
            if !rect_contains(hit.area, mouse.column, mouse.row) {
                continue;
            }
            return match mouse.kind {
                MouseEventKind::ScrollUp => match hit.scroll {
                    Some(target) => {
                        self.apply_action(Action::Global(GlobalAction::MouseScroll {
                            target,
                            direction: ScrollDirection::Up,
                            lines: wheel_lines(mouse),
                        }))
                        .await
                    }
                    None => false,
                },
                MouseEventKind::ScrollDown => match hit.scroll {
                    Some(target) => {
                        self.apply_action(Action::Global(GlobalAction::MouseScroll {
                            target,
                            direction: ScrollDirection::Down,
                            lines: wheel_lines(mouse),
                        }))
                        .await
                    }
                    None => false,
                },
                MouseEventKind::Down(MouseButton::Left) => match hit.click {
                    Some(target) => {
                        self.apply_action(Action::Global(GlobalAction::MouseClick {
                            target,
                            x: mouse.column,
                            y: mouse.row,
                        }))
                        .await
                    }
                    None => false,
                },
                MouseEventKind::Drag(MouseButton::Left) | MouseEventKind::Up(MouseButton::Left)
                    if hit.allow_drag =>
                {
                    self.apply_action(Action::Global(GlobalAction::MouseDrag {
                        x: mouse.column,
                        y: mouse.row,
                        release: matches!(mouse.kind, MouseEventKind::Up(MouseButton::Left)),
                    }))
                    .await
                }
                _ => false,
            };
        }

        match mouse.kind {
            MouseEventKind::ScrollUp => {
                self.apply_action(Action::Global(GlobalAction::MouseScroll {
                    target: ScrollTarget::ConversationChat,
                    direction: ScrollDirection::Up,
                    lines: wheel_lines(mouse),
                }))
                .await
            }
            MouseEventKind::ScrollDown => {
                self.apply_action(Action::Global(GlobalAction::MouseScroll {
                    target: ScrollTarget::ConversationChat,
                    direction: ScrollDirection::Down,
                    lines: wheel_lines(mouse),
                }))
                .await
            }
            MouseEventKind::ScrollLeft | MouseEventKind::ScrollRight => false,
            _ => false,
        }
    }
}

/// Lines to scroll for one wheel event. `coalesce_scroll_batch` stores the
/// merged tick count in `modifiers.bits()` (1 = single event).
fn wheel_lines(mouse: MouseEvent) -> u16 {
    const LINES_PER_TICK: u16 = 3;
    let ticks = u16::from(mouse.modifiers.bits()).max(1);
    LINES_PER_TICK.saturating_mul(ticks)
}
