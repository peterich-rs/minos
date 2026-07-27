//! Conversation roster briefing + coordination system-message helpers.

use crate::store::ConversationAgentMemberRow;

/// Max brief length enforced when building protocol / briefing text.
pub const MAX_ROSTER_BRIEF_CHARS: usize = 500;

/// Build developer/system briefing text injected at agent session start.
///
/// `self_agent` is the runtime label of the session being started (`codex`, …).
pub fn format_roster_briefing(self_agent: &str, members: &[ConversationAgentMemberRow]) -> String {
    let self_agent = self_agent.trim();
    let mut lines = Vec::new();
    lines.push("## Conversation roster".to_string());
    lines.push(format!(
        "You are **{self_agent}** in this Minos conversation with other CLI agents."
    ));
    if let Some(self_row) = members.iter().find(|m| m.agent == self_agent) {
        if let Some(brief) = self_row
            .brief
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            lines.push(format!("Your role brief: {brief}"));
        }
    }
    lines.push(
        "Teammates (prefer `list_conversation_roster` for the live list; \
         use `delegate_to_agent` or conversation @mentions for the matching teammate):"
            .to_string(),
    );
    let mut any = false;
    for m in members {
        if m.agent == self_agent {
            continue;
        }
        any = true;
        match m.brief.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
            Some(brief) => lines.push(format!("- **{}**: {brief}", m.agent)),
            None => lines.push(format!(
                "- **{}**: (no brief; check conversation messages / ask the user)",
                m.agent
            )),
        }
    }
    if !any {
        lines.push("- (no other roster members yet)".to_string());
    }
    lines.push(
        "Do not assume this snapshot is complete after roster changes; \
         call `list_conversation_roster` when coordination depends on who is present."
            .to_string(),
    );
    lines.join("\n")
}

/// Conversation-timeline system message body when a member is removed.
pub fn format_roster_removed_system_message(agent: &str) -> String {
    format!(
        "[minos:system] Roster updated: **{}** left this conversation. \
         Remaining agents should use `list_conversation_roster` before further delegation.",
        agent.trim()
    )
}

/// Conversation-timeline system message when the initial roster is set.
pub fn format_roster_established_system_message(members: &[ConversationAgentMemberRow]) -> String {
    if members.is_empty() {
        return "[minos:system] Roster updated: no agents on this conversation.".to_string();
    }
    let mut parts = Vec::new();
    for m in members {
        match m.brief.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
            Some(brief) => parts.push(format!("**{}** ({brief})", m.agent)),
            None => parts.push(format!("**{}**", m.agent)),
        }
    }
    format!(
        "[minos:system] Roster established: {}. \
         Call `list_conversation_roster` for structured membership.",
        parts.join(", ")
    )
}

/// Merge optional profile/extra instructions with a roster briefing.
pub fn merge_launch_instructions(
    existing: Option<String>,
    roster_briefing: &str,
) -> Option<String> {
    let briefing = roster_briefing.trim();
    if briefing.is_empty() {
        return existing.and_then(|s| {
            let t = s.trim().to_owned();
            if t.is_empty() {
                None
            } else {
                Some(t)
            }
        });
    }
    match existing.and_then(|s| {
        let t = s.trim().to_owned();
        if t.is_empty() {
            None
        } else {
            Some(t)
        }
    }) {
        Some(extra) => Some(format!("{extra}\n\n{briefing}")),
        None => Some(briefing.to_owned()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn briefing_lists_teammates_and_self() {
        let members = vec![
            ConversationAgentMemberRow {
                agent: "codex".into(),
                brief: Some("implements features in the worktree".into()),
                joined_at_ms: 1,
            },
            ConversationAgentMemberRow {
                agent: "claude".into(),
                brief: Some("reviews pull requests".into()),
                joined_at_ms: 2,
            },
        ];
        let text = format_roster_briefing("codex", &members);
        assert!(text.contains("You are **codex**"));
        assert!(text.contains("implements features"));
        assert!(text.contains("**claude**: reviews pull requests"));
        assert!(text.contains("list_conversation_roster"));
    }

    #[test]
    fn merge_instructions_appends_briefing() {
        let merged =
            merge_launch_instructions(Some("Be concise.".into()), "## Conversation roster\n…")
                .unwrap();
        assert!(merged.starts_with("Be concise."));
        assert!(merged.contains("Conversation roster"));
    }
}
