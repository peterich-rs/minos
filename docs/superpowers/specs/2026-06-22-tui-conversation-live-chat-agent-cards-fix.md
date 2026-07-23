# Conversation 主时间线与 Teamwork MCP 迁移

> 日期: 2026-06-22
> 状态: 已实现
> 类型: Bug 修复 + 旧 group chat 删除
> 关联: `2026-06-18-conversation-centric-hierarchy-design.md` §6.8/§6.9

## 1. 问题

Conversation 视图右侧 agent 卡片能从运行态变为空闲，但左侧 chat list 没显示 agent 回复。进入 agent 会话页可以看到回复，说明 agent ingest 和 agent detail 渲染正常，缺的是完成后写回 conversation 主时间线。

根因是 TUI completion handler 仍沿用旧 group chat 的去重/回写路径；agent 最终回复没有稳定写入 daemon `chat_messages(conversation_id, ...)`，当前打开的 `conversation_messages` 也不会增量刷新。

## 2. 当前实现

- `app/conversation_result.rs` 在 agent turn 完成时读取 `ChatState.last_completed_assistant_text`，生成稳定 `agent-result:{conversation_id}:{session_id}:{message_id}`，调用 backend `append_conversation_message(role="agent")` 持久化，并在当前 conversation 可见时追加到 `ui.conversation_messages`。
- `app/submission.rs` 的 conversation 输入仍负责 user 消息：本地 pending 追加 + daemon `append_conversation_message(role="user")`。
- `backend::{daemon,embedded}` 和 daemon local RPC 都只保留 conversation message API，不再暴露 `read_group_chat`。
- `ui/conversation_view.rs` 渲染 conversation 主时间线；滚动目标统一为 `ConversationChat`。
- 旧 TUI group chat 模块、旧 app group chat handler 和对应测试已删除。

## 3. Teamwork MCP

`minos_teamwork` MCP 现在绑定启动 agent 时的 `conversation_id`：

- 工具集只保留 `list_conversation_messages`、`delegate_to_agent`、`get_delegation_status`、`cancel_delegation`、`post_conversation_update`。
- `list_conversation_messages` 读取当前 conversation 的 daemon/local conversation messages。
- `post_conversation_update` 使用 MCP sidecar 的 source agent/thread metadata 写入当前 conversation 的 agent-role message；body 以 `@agent` 或 `@agent#short_thread` 开头时，同时把 clean body 投递给目标 thread。daemon 模式写入成功后通过 `subscribe_conversation_events()` 的 `ConversationMessageAppended` 事件触发 TUI 刷新当前 conversation。
- `delegate_to_agent` 先通过 `start_agent_in_conversation` 启动目标 agent，并且 clean prompt 成功提交给目标 thread 后，才写入一条带 source agent/thread metadata 的可见消息，body 形如 `@target_agent#short_thread <prompt>`，并把 delegation 状态存在 conversation-scoped teamwork store。
- agent-runtime 的 Codex/OpenCode 实例缓存按 `(workspace, conversation_id, source_session_id)` 隔离，避免同一 workspace/conversation 的不同 agent session 复用错误 MCP 配置；persisted conversation thread 恢复时也会恢复 `mcp_conversation_id`。

## 4. 删除范围

已删除或迁移：

- `minos-protocol` 的 `ReadGroupChat*` 和 `LocalGroupChat*` local RPC 类型。
- daemon/TUI 的 `read_group_chat` RPC 实现和测试。
- TUI `group_chat.rs`、`app/group_chat.rs`、旧 group chat app tests。
- MCP 旧工具：`list_room_messages`、`post_room_update`、`ask_user_question`、`check_user_feedback`、`react_to_message`。
- `minos-chat-store` 的旧 group message、feedback、reaction API；该 crate 现在只提供 conversation-scoped teamwork delegation 存储。

保留：

- daemon/backend/mobile 的产品侧 `chat_messages` 命名，这是 conversation message 的 canonical 表，不属于旧 TUI group chat。

## 5. 验证

已跑通过：

- `cargo test -p minos-chat-store --quiet`
- `cargo test -p minos-tui --quiet`
- `cargo test -p minos-daemon --quiet`
- `cargo test -p minos-agent-runtime --lib -j1 --quiet`
