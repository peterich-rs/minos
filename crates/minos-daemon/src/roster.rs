//! Conversation roster briefing + coordination system-message helpers.

use crate::store::ConversationAgentMemberRow;

/// Max brief length enforced when building protocol / briefing text.
pub const MAX_ROSTER_BRIEF_CHARS: usize = 500;

/// Display label for a roster member: prefer display_name, then runtime, then bot_id.
pub fn member_label(m: &ConversationAgentMemberRow) -> &str {
    m.display_name
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .or_else(|| {
            m.runtime_agent
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
        })
        .unwrap_or(m.bot_id.as_str())
}

/// Build developer/system briefing text injected at agent session start.
///
/// `self_bot_id` is the bot identity of the session being started.
/// `self_label` is the human-facing name shown in the briefing (display/runtime).
pub fn format_roster_briefing(
    self_bot_id: &str,
    self_label: &str,
    members: &[ConversationAgentMemberRow],
) -> String {
    let self_bot_id = self_bot_id.trim();
    let self_label = self_label.trim();
    let mut lines = Vec::new();
    lines.push("## Conversation roster".to_string());
    lines.push(format!(
        "You are **{self_label}** (bot_id=`{self_bot_id}`) in this Minos conversation with other CLI agents."
    ));
    if let Some(self_row) = members.iter().find(|m| m.bot_id == self_bot_id) {
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
        if m.bot_id == self_bot_id {
            continue;
        }
        any = true;
        let label = member_label(m);
        match m.brief.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
            Some(brief) => lines.push(format!("- **{label}** (`{}`): {brief}", m.bot_id)),
            None => lines.push(format!(
                "- **{label}** (`{}`): (no brief; check conversation messages / ask the user)",
                m.bot_id
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
pub fn format_roster_removed_system_message(label: &str) -> String {
    format!(
        "[minos:system] Roster updated: **{}** left this conversation. \
         Remaining agents should use `list_conversation_roster` before further delegation.",
        label.trim()
    )
}

/// Host → **agent session** inject body when roster changes mid-flight.
///
/// Wire shape uses the provider "user input" channel (CLI limitation) but is
/// **not** a conversation user message and must not be written to `chat_messages`
/// as `sender_role=user`. Prefix identifies host coordination.
pub fn format_roster_host_session_inject(
    self_label: &str,
    members: &[ConversationAgentMemberRow],
    change_summary: &str,
) -> String {
    let self_label = self_label.trim();
    let mut lines = vec![
        "[minos:host] kind=roster_changed".to_string(),
        String::new(),
        "Host coordination notice (not a user chat message).".to_string(),
        change_summary.trim().to_owned(),
        format!("You remain **{self_label}** in this conversation."),
        "Current roster:".to_string(),
    ];
    if members.is_empty() {
        lines.push("- (empty roster)".to_string());
    } else {
        for m in members {
            let label = member_label(m);
            match m.brief.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
                Some(brief) => lines.push(format!("- **{label}**: {brief}")),
                None => lines.push(format!("- **{label}**")),
            }
        }
    }
    lines.push(String::new());
    lines.push(
        "Use `list_conversation_roster` for structured membership. \
         Do not treat this as a user task; only adjust plans if they depended on a removed teammate."
            .to_string(),
    );
    lines.join("\n")
}

/// Conversation-timeline system message when a member joins.
pub fn format_roster_joined_system_message(label: &str, brief: Option<&str>) -> String {
    let label = label.trim();
    match brief.map(str::trim).filter(|s| !s.is_empty()) {
        Some(brief) => format!(
            "[minos:system] Roster updated: **{label}** joined ({brief}). \
             Call `list_conversation_roster` for structured membership."
        ),
        None => format!(
            "[minos:system] Roster updated: **{label}** joined this conversation. \
             Call `list_conversation_roster` for structured membership."
        ),
    }
}

/// Conversation-timeline system message when the initial roster is set.
pub fn format_roster_established_system_message(members: &[ConversationAgentMemberRow]) -> String {
    if members.is_empty() {
        return "[minos:system] Roster updated: no agents on this conversation.".to_string();
    }
    let mut parts = Vec::new();
    for m in members {
        let label = member_label(m);
        match m.brief.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
            Some(brief) => parts.push(format!("**{label}** ({brief})")),
            None => parts.push(format!("**{label}**")),
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
                bot_id: "local-rt-codex".into(),
                brief: Some("implements features in the worktree".into()),
                joined_at_ms: 1,
                runtime_agent: Some("codex".into()),
                display_name: Some("codex".into()),
            },
            ConversationAgentMemberRow {
                bot_id: "local-rt-claude".into(),
                brief: Some("reviews pull requests".into()),
                joined_at_ms: 2,
                runtime_agent: Some("claude".into()),
                display_name: Some("claude".into()),
            },
        ];
        let text = format_roster_briefing("local-rt-codex", "codex", &members);
        assert!(text.contains("You are **codex**"));
        assert!(text.contains("implements features"));
        assert!(text.contains("**claude**"));
        assert!(text.contains("reviews pull requests"));
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

    #[test]
    fn host_session_inject_is_marked_and_lists_roster() {
        let members = vec![ConversationAgentMemberRow {
            bot_id: "local-rt-claude".into(),
            brief: Some("reviews PRs".into()),
            joined_at_ms: 1,
            runtime_agent: Some("claude".into()),
            display_name: Some("claude".into()),
        }];
        let text = format_roster_host_session_inject(
            "codex",
            &members,
            "Member **gemini** left the conversation.",
        );
        assert!(text.starts_with("[minos:host] kind=roster_changed"));
        assert!(text.contains("not a user chat message"));
        assert!(text.contains("You remain **codex**"));
        assert!(text.contains("**claude**: reviews PRs"));
        assert!(!text.contains("[minos:system]"));
    }
}
