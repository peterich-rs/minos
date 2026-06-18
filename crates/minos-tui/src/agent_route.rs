use minos_agent_runtime::ThreadState;
use minos_domain::AgentName;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AgentRouteTarget {
    pub(crate) agent: AgentName,
    pub(crate) thread_short_id: Option<String>,
}

pub(crate) fn parse_agent_routing(text: &str) -> Option<(AgentRouteTarget, String)> {
    let trimmed = text.trim_start();
    let rest = trimmed.strip_prefix('@')?;
    let split_at = rest
        .char_indices()
        .find(|(_, ch)| ch.is_whitespace())
        .map(|(index, _)| index)
        .unwrap_or(rest.len());
    let target = parse_agent_route_target(&rest[..split_at])?;
    let body = rest[split_at..].trim_start().to_owned();
    Some((target, body))
}

pub(crate) fn parse_agent_route_target(value: &str) -> Option<AgentRouteTarget> {
    let (agent_name, thread_short_id) = match value.split_once('#') {
        Some((agent_name, thread_short_id)) if !thread_short_id.is_empty() => {
            (agent_name, Some(thread_short_id.to_owned()))
        }
        Some(_) => return None,
        None => (value, None),
    };
    Some(AgentRouteTarget {
        agent: parse_agent_name(agent_name)?,
        thread_short_id,
    })
}

pub(crate) fn parse_agent_name(value: &str) -> Option<AgentName> {
    let normalized = value.to_ascii_lowercase();
    AgentName::all()
        .iter()
        .copied()
        .find(|agent| agent.bin_name() == normalized.as_str())
}

pub(crate) fn short_thread_id(thread_id: &str) -> String {
    thread_id[..8.min(thread_id.len())].to_owned()
}

pub(crate) fn thread_can_receive_message(state: &ThreadState) -> bool {
    !matches!(state, ThreadState::Closed { .. })
}
