//! Agent inbox (bot mailbox) planning and transactional enqueue helpers.
//!
//! Live collab writes co-commit `bot_message_deliveries` with the origin message.
//! Selection rules: reply-to agent → structured mentions → sole-agent membership.
//! Body text never invents delivery targets.

use minos_protocol::{ChatMessageSummary, SenderType};

use crate::error::BackendError;
use crate::store::{agent_dispatch_queue, social, StoreHandle};

/// Max bot→bot automation hops from a human root message.
pub const MAX_AUTOMATION_HOP: i32 = 3;

#[derive(Clone)]
pub struct AgentDispatchPlan {
    pub agent: social::AgentRow,
    pub session_id: Option<String>,
    pub forwarded_text: String,
    pub mention_sender: bool,
}

/// Plan agent inbox deliveries for a message that is about to be (or was) committed.
///
/// Selection order:
/// 1. reply-to agent message → that agent (human senders only)
/// 2. structured `mentioned_agent_ids` (client order; other-bots only for bot authors)
/// 3. sole-agent: group + 1 human + 1 active agent + empty structured agent mentions
/// 4. empty
pub async fn plan_agent_deliveries(
    store: &StoreHandle,
    conversation_id: &str,
    message: &ChatMessageSummary,
    text: &str,
    reply_target: Option<&social::ChatMessageRow>,
) -> Result<Vec<AgentDispatchPlan>, BackendError> {
    let conversation = social::get_conversation(store, conversation_id)
        .await?
        .ok_or_else(|| BackendError::StoreQuery {
            operation: "plan_agent_deliveries".into(),
            message: "conversation not found".into(),
        })?;
    let agents = social::list_conversation_agents_active(store, conversation_id).await?;
    if agents.is_empty() {
        return Ok(Vec::new());
    }
    let human_members = social::list_conversation_members(store, conversation_id).await?;
    let mention_sender = human_members.len() > 1;
    let origin_is_agent = message.sender.is_bot() || message.sender_type == SenderType::Agent;
    let origin_agent_id = message.sender.bot_id();

    if !origin_is_agent {
        if let Some(reply_target) = reply_target {
            if reply_target.sender_type == "agent" {
                if let Some(agent_id) = reply_target
                    .sender_agent_id
                    .as_deref()
                    .filter(|s| !s.is_empty())
                {
                    let agent = agents.iter().find(|a| a.agent_id == agent_id).cloned();
                    let agent = match agent {
                        Some(a) => Some(a),
                        None => social::get_agent(store, agent_id)
                            .await?
                            .filter(|a| a.is_active()),
                    };
                    if let Some(agent) = agent {
                        let session_id =
                            social::lookup_session_id_for_message(store, &reply_target.message_id)
                                .await?;
                        return Ok(vec![AgentDispatchPlan {
                            agent,
                            session_id,
                            forwarded_text: text.to_string(),
                            mention_sender,
                        }]);
                    }
                }
            }
        }
    }

    if !message.mentioned_agent_ids.is_empty() {
        let mut plans = Vec::with_capacity(message.mentioned_agent_ids.len());
        let mut seen = std::collections::HashSet::new();
        for agent_id in &message.mentioned_agent_ids {
            if !seen.insert(agent_id.clone()) {
                continue;
            }
            if origin_agent_id.is_some_and(|self_id| self_id == agent_id.as_str()) {
                continue;
            }
            let Some(agent) = agents.iter().find(|a| a.agent_id == *agent_id).cloned() else {
                continue;
            };
            let session_short =
                crate::conversations::use_case::session_short_hint_for_agent(text, &agent);
            let session_id =
                resolve_dispatch_session_id(store, conversation_id, &agent, session_short).await?;
            plans.push(AgentDispatchPlan {
                agent,
                session_id,
                forwarded_text: text.to_string(),
                mention_sender: if origin_is_agent {
                    false
                } else {
                    mention_sender
                },
            });
        }
        return Ok(plans);
    }

    if origin_is_agent {
        return Ok(Vec::new());
    }

    if conversation.kind == "group" && human_members.len() == 1 && agents.len() == 1 {
        let agent = agents[0].clone();
        let session_id = resolve_dispatch_session_id(store, conversation_id, &agent, None).await?;
        return Ok(vec![AgentDispatchPlan {
            agent,
            session_id,
            forwarded_text: text.to_string(),
            mention_sender: false,
        }]);
    }

    Ok(Vec::new())
}

/// Resolve which formal agent session should receive a mailbox delivery.
pub async fn resolve_dispatch_session_id(
    store: &StoreHandle,
    conversation_id: &str,
    agent: &social::AgentRow,
    session_short_id: Option<&str>,
) -> Result<Option<String>, BackendError> {
    if let Some(short) = session_short_id.map(str::trim).filter(|s| !s.is_empty()) {
        if let Some(session_id) = crate::store::agent_sessions::find_reusable_by_short_id(
            store,
            conversation_id,
            &agent.agent_id,
            &agent.runtime_agent,
            short,
        )
        .await?
        {
            return Ok(Some(session_id));
        }
    }

    if let Some(session_id) =
        social::lookup_latest_session_id_for_conversation_agent(store, conversation_id, &agent.agent_id)
            .await?
    {
        return Ok(Some(session_id));
    }

    crate::store::agent_sessions::latest_reusable_for_conversation_agent(
        store,
        conversation_id,
        &agent.agent_id,
        &agent.runtime_agent,
    )
    .await
}

/// Build durable inbox rows for planned deliveries (stable multi-@ order).
pub fn build_dispatch_rows(
    plans: Vec<AgentDispatchPlan>,
    message: &ChatMessageSummary,
    conversation_id: &str,
    account_id: &str,
    sender_minos_id: Option<String>,
    automation_hop: i32,
    now_ms: i64,
) -> Vec<agent_dispatch_queue::AgentDispatchRow> {
    plans
        .into_iter()
        .enumerate()
        .map(|(ordinal, plan)| {
            let ordered_ms = now_ms.saturating_add(ordinal as i64);
            agent_dispatch_queue::AgentDispatchRow {
                dispatch_id: uuid::Uuid::new_v4().to_string(),
                origin_message_id: message.message_id.clone(),
                conversation_id: conversation_id.to_string(),
                account_id: account_id.to_string(),
                agent_id: plan.agent.agent_id,
                session_id: plan.session_id,
                forwarded_text: plan.forwarded_text,
                mention_sender: plan.mention_sender,
                sender_minos_id: sender_minos_id.clone(),
                status: agent_dispatch_queue::STATUS_PENDING.to_string(),
                attempts: 0,
                next_attempt_at_ms: now_ms,
                last_error: None,
                created_at_ms: ordered_ms,
                updated_at_ms: ordered_ms,
                lease_owner_host_id: None,
                lease_expires_at_ms: None,
                automation_hop,
            }
        })
        .collect()
}

/// Enqueue planned rows inside an open write transaction. Returns whether any row was inserted.
pub async fn enqueue_plans_in_tx(
    tx: &mut crate::app::tx::DbTx<'_>,
    rows: &[agent_dispatch_queue::AgentDispatchRow],
) -> Result<bool, BackendError> {
    let mut any = false;
    for row in rows {
        if agent_dispatch_queue::enqueue_in_tx(tx, row).await? {
            any = true;
        }
    }
    Ok(any)
}
