use minos_agent_runtime::SessionState;
use minos_domain::AgentName;

/// Host agent profile fields needed for @-routing / mention insert (desktop parity).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MentionProfile {
    pub id: String,
    pub name: String,
    pub runtime_agent: AgentName,
    pub updated_at_ms: i64,
}

/// Resolved @-route target.
/// - bare agent: `profile_id` unset — **reuse** open top-level session if any; else start with newest profile
/// - agent#short: continue that session (`profile_id` unset)
/// - profile mention: `profile_id` + agent from profile.runtime_agent (always new session)
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AgentRouteTarget {
    pub(crate) agent: AgentName,
    pub(crate) session_short_id: Option<String>,
    /// Explicit host agent profile id when routed via @ProfileName or @p/<id>.
    pub(crate) profile_id: Option<String>,
}

pub(crate) fn parse_agent_routing(
    text: &str,
    profiles: &[MentionProfile],
) -> Option<(AgentRouteTarget, String)> {
    let trimmed = text.trim_start();
    let rest = trimmed.strip_prefix('@')?;
    let split_at = rest
        .char_indices()
        .find(|(_, ch)| ch.is_whitespace())
        .map(|(index, _)| index)
        .unwrap_or(rest.len());
    let target = parse_agent_route_target(&rest[..split_at], profiles)?;
    let body = rest[split_at..].trim_start().to_owned();
    Some((target, body))
}

/// Parse a single route token (no leading `@`).
///
/// Resolution order (desktop parity):
/// 1. `p/<id>` or `p:<id>` → profile by id
/// 2. `agent#short` → continue session
/// 3. known runtime agent name → bare agent
/// 4. unique profile name (case-insensitive) → profile
pub(crate) fn parse_agent_route_target(
    value: &str,
    profiles: &[MentionProfile],
) -> Option<AgentRouteTarget> {
    let raw = value.trim();
    if raw.is_empty() {
        return None;
    }

    // Explicit profile id form: p/<id> or p:<id>
    if let Some(rest) = raw
        .strip_prefix("p/")
        .or_else(|| raw.strip_prefix("P/"))
        .or_else(|| raw.strip_prefix("p:"))
        .or_else(|| raw.strip_prefix("P:"))
    {
        let id = rest.trim();
        if id.is_empty() {
            return None;
        }
        let profile = profiles.iter().find(|p| p.id == id)?;
        return Some(AgentRouteTarget {
            agent: profile.runtime_agent,
            session_short_id: None,
            profile_id: Some(profile.id.clone()),
        });
    }

    let (agent_part, session_short_id) = match raw.split_once('#') {
        Some((agent_name, session_short_id)) if !session_short_id.is_empty() => {
            (agent_name, Some(session_short_id.to_owned()))
        }
        Some(_) => return None,
        None => (raw, None),
    };

    // Runtime agents win over same-named profiles (colliding profiles use @p/id).
    if let Some(agent) = parse_agent_name(agent_part) {
        return Some(AgentRouteTarget {
            agent,
            session_short_id,
            profile_id: None,
        });
    }

    // Profile by unique name (case-insensitive). Ignore when #session form.
    if session_short_id.is_some() {
        return None;
    }
    let key = normalize_profile_name(agent_part);
    if key.is_empty() {
        return None;
    }
    let name_matches: Vec<&MentionProfile> = profiles
        .iter()
        .filter(|p| normalize_profile_name(&p.name) == key)
        .collect();
    if name_matches.len() != 1 {
        return None;
    }
    let profile = name_matches[0];
    Some(AgentRouteTarget {
        agent: profile.runtime_agent,
        session_short_id: None,
        profile_id: Some(profile.id.clone()),
    })
}

pub(crate) fn parse_agent_name(value: &str) -> Option<AgentName> {
    let normalized = value.to_ascii_lowercase();
    AgentName::all()
        .iter()
        .copied()
        .find(|agent| agent.bin_name() == normalized.as_str())
}

pub(crate) fn normalize_profile_name(name: &str) -> String {
    name.trim().to_ascii_lowercase()
}

/// Whether a profile display name can be a single `@Name` route token.
/// Whitespace breaks token split; `#` is agent#session form; `@` nests mentions.
pub(crate) fn is_profile_name_clean_token(name: &str) -> bool {
    let n = name.trim();
    if n.is_empty() {
        return false;
    }
    !n.chars().any(|ch| ch.is_whitespace() || ch == '#' || ch == '@')
}

/// Unique among profiles + runtime agent names (case-insensitive).
pub(crate) fn is_profile_name_unique(name: &str, profiles: &[MentionProfile]) -> bool {
    let key = normalize_profile_name(name);
    if key.is_empty() {
        return false;
    }
    if AgentName::all()
        .iter()
        .any(|a| a.bin_name().eq_ignore_ascii_case(&key))
    {
        return false;
    }
    let hits = profiles
        .iter()
        .filter(|p| normalize_profile_name(&p.name) == key)
        .count();
    hits == 1
}

/// Insert token for a profile: `@Name ` when unique **and** a clean token, else `@p/<id> `.
pub(crate) fn profile_mention_insert(profile: &MentionProfile, profiles: &[MentionProfile]) -> String {
    if is_profile_name_clean_token(&profile.name) && is_profile_name_unique(&profile.name, profiles)
    {
        format!("@{} ", profile.name.trim())
    } else {
        format!("@p/{} ", profile.id)
    }
}

/// Newest host profile id for a runtime (by `updated_at_ms`), if any.
pub(crate) fn newest_profile_id_for_agent(
    profiles: &[MentionProfile],
    agent: AgentName,
) -> Option<String> {
    profiles
        .iter()
        .filter(|p| p.runtime_agent == agent)
        .max_by_key(|p| p.updated_at_ms)
        .map(|p| p.id.clone())
}

/// First up-to-8 bytes of a session id for display / mention tokens.
///
/// Session ids are ASCII hex (or similar); this returns a borrowed slice so
/// render paths never allocate.
pub(crate) fn short_session_id(session_id: &str) -> &str {
    let mut end = session_id.len().min(8);
    while end > 0 && !session_id.is_char_boundary(end) {
        end -= 1;
    }
    &session_id[..end]
}

pub(crate) fn thread_can_receive_message(state: &SessionState) -> bool {
    !matches!(state, SessionState::Closed { .. })
}

/// Fields needed to pick a reusable top-level session (desktop bare-`@agent` parity).
#[derive(Debug, Clone, Copy)]
pub(crate) struct SessionReuseCandidate<'a> {
    pub session_id: &'a str,
    pub agent: AgentName,
    pub parent_session_id: Option<&'a str>,
    pub state: &'a SessionState,
    pub last_ts_ms: i64,
}

/// Most recent top-level, non-closed session for `agent`, if any.
///
/// Matches desktop send use-case: bare `@agent` reuses before starting a new run.
/// Subagents (`parent_session_id` set) and closed sessions are excluded.
pub(crate) fn pick_reusable_session_id(
    candidates: &[SessionReuseCandidate<'_>],
    agent: AgentName,
) -> Option<String> {
    candidates
        .iter()
        .filter(|c| c.agent == agent)
        .filter(|c| c.parent_session_id.is_none())
        .filter(|c| thread_can_receive_message(c.state))
        .max_by_key(|c| c.last_ts_ms)
        .map(|c| c.session_id.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn profile(id: &str, name: &str, agent: AgentName, updated: i64) -> MentionProfile {
        MentionProfile {
            id: id.into(),
            name: name.into(),
            runtime_agent: agent,
            updated_at_ms: updated,
        }
    }

    #[test]
    fn short_session_id_returns_borrowed_prefix() {
        let id = "abcdef0123456789";
        assert_eq!(short_session_id(id), "abcdef01");
        assert_eq!(short_session_id("abc"), "abc");
        assert_eq!(short_session_id(""), "");
    }

    #[test]
    fn short_session_id_respects_utf8_char_boundary() {
        // "abcdef" (6) + 你 (3 bytes at 6..=8) → raw cut at 8 is mid-char.
        let id = "abcdef你g";
        let short = short_session_id(id);
        assert_eq!(short, "abcdef");
        assert!(id.is_char_boundary(short.len()));
    }

    #[test]
    fn parses_bare_agent_and_session_short() {
        let (target, body) = parse_agent_routing("@codex hello", &[]).unwrap();
        assert_eq!(target.agent, AgentName::Codex);
        assert!(target.session_short_id.is_none());
        assert!(target.profile_id.is_none());
        assert_eq!(body, "hello");

        let (target, body) = parse_agent_routing("@grok#abcd1234 continue", &[]).unwrap();
        assert_eq!(target.agent, AgentName::Grok);
        assert_eq!(target.session_short_id.as_deref(), Some("abcd1234"));
        assert!(target.profile_id.is_none());
        assert_eq!(body, "continue");
    }

    #[test]
    fn parses_unique_profile_name() {
        let profiles = vec![profile(
            "profile-research",
            "ResearchGrok",
            AgentName::Grok,
            10,
        )];
        let (target, body) = parse_agent_routing("@ResearchGrok dig in", &profiles).unwrap();
        assert_eq!(target.agent, AgentName::Grok);
        assert_eq!(target.profile_id.as_deref(), Some("profile-research"));
        assert_eq!(body, "dig in");
    }

    #[test]
    fn parses_profile_by_id_token() {
        let profiles = vec![profile("profile-research", "ResearchGrok", AgentName::Grok, 1)];
        let (target, _) = parse_agent_routing("@p/profile-research go", &profiles).unwrap();
        assert_eq!(target.profile_id.as_deref(), Some("profile-research"));
        assert_eq!(target.agent, AgentName::Grok);
    }

    #[test]
    fn runtime_name_wins_over_same_named_profile() {
        let profiles = vec![profile("profile-codex", "codex", AgentName::Grok, 1)];
        let (target, _) = parse_agent_routing("@codex hi", &profiles).unwrap();
        assert_eq!(target.agent, AgentName::Codex);
        assert!(target.profile_id.is_none());
    }

    #[test]
    fn ambiguous_profile_name_does_not_parse() {
        let profiles = vec![
            profile("p1", "Helper", AgentName::Grok, 1),
            profile("p2", "Helper", AgentName::Codex, 2),
        ];
        assert!(parse_agent_routing("@Helper hi", &profiles).is_none());
    }

    #[test]
    fn profile_mention_insert_uses_p_id_when_name_collides_with_runtime() {
        let profiles = vec![profile("p1", "codex", AgentName::Grok, 1)];
        let insert = profile_mention_insert(&profiles[0], &profiles);
        assert_eq!(insert, "@p/p1 ");
    }

    #[test]
    fn newest_profile_id_picks_latest_updated() {
        let profiles = vec![
            profile("old", "A", AgentName::Grok, 1),
            profile("new", "B", AgentName::Grok, 99),
            profile("other", "C", AgentName::Codex, 1000),
        ];
        assert_eq!(
            newest_profile_id_for_agent(&profiles, AgentName::Grok).as_deref(),
            Some("new")
        );
        assert_eq!(
            newest_profile_id_for_agent(&profiles, AgentName::Claude),
            None
        );
    }

    #[test]
    fn rejects_unclean_profile_token_chars_for_name_uniqueness_insert() {
        assert!(!is_profile_name_clean_token("has space"));
        assert!(!is_profile_name_clean_token("has#hash"));
        assert!(!is_profile_name_clean_token("has@at"));
        assert!(is_profile_name_clean_token("ResearchGrok"));
    }

    #[test]
    fn pick_reusable_session_prefers_latest_open_top_level() {
        let idle = SessionState::Idle;
        let closed = SessionState::Closed {
            reason: minos_agent_runtime::CloseReason::UserClose,
        };
        let candidates = [
            SessionReuseCandidate {
                session_id: "old-codex",
                agent: AgentName::Codex,
                parent_session_id: None,
                state: &idle,
                last_ts_ms: 10,
            },
            SessionReuseCandidate {
                session_id: "new-codex",
                agent: AgentName::Codex,
                parent_session_id: None,
                state: &idle,
                last_ts_ms: 99,
            },
            SessionReuseCandidate {
                session_id: "child",
                agent: AgentName::Codex,
                parent_session_id: Some("new-codex"),
                state: &idle,
                last_ts_ms: 200,
            },
            SessionReuseCandidate {
                session_id: "closed",
                agent: AgentName::Codex,
                parent_session_id: None,
                state: &closed,
                last_ts_ms: 300,
            },
            SessionReuseCandidate {
                session_id: "grok",
                agent: AgentName::Grok,
                parent_session_id: None,
                state: &idle,
                last_ts_ms: 500,
            },
        ];
        assert_eq!(
            pick_reusable_session_id(&candidates, AgentName::Codex).as_deref(),
            Some("new-codex"),
        );
        assert_eq!(
            pick_reusable_session_id(&candidates, AgentName::Claude),
            None,
        );
    }
}
