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

/// First up-to-8 bytes of a thread id for display / mention tokens.
///
/// Thread ids are ASCII hex (or similar); this returns a borrowed slice so
/// render paths never allocate.
pub(crate) fn short_thread_id(thread_id: &str) -> &str {
    let mut end = thread_id.len().min(8);
    while end > 0 && !thread_id.is_char_boundary(end) {
        end -= 1;
    }
    &thread_id[..end]
}

pub(crate) fn thread_can_receive_message(state: &ThreadState) -> bool {
    !matches!(state, ThreadState::Closed { .. })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_thread_id_returns_borrowed_prefix() {
        let id = "abcdef0123456789";
        assert_eq!(short_thread_id(id), "abcdef01");
        assert_eq!(short_thread_id("abc"), "abc");
        assert_eq!(short_thread_id(""), "");
    }

    #[test]
    fn short_thread_id_respects_utf8_char_boundary() {
        // "abcdef" (6) + 你 (3 bytes at 6..=8) → raw cut at 8 is mid-char.
        let id = "abcdef你g";
        let short = short_thread_id(id);
        assert_eq!(short, "abcdef");
        assert!(id.is_char_boundary(short.len()));
    }
}
