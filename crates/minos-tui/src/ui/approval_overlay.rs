use ratatui::{
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Clear, Paragraph, Wrap},
    Frame,
};

use crate::translation::{PendingAgentRequest, PendingAgentRequestKind};

use super::theme;

const OVERLAY_HEIGHT: u16 = 8;
const PLAN_OVERLAY_HEIGHT: u16 = 14;

struct ApprovalOption {
    label: String,
    text: String,
}

pub fn is_selectable(request: &PendingAgentRequest) -> bool {
    !options_for_request(request).is_empty()
}

pub fn option_count(request: &PendingAgentRequest) -> usize {
    options_for_request(request).len()
}

pub fn selected_text(request: &PendingAgentRequest, selected: usize) -> Option<String> {
    let options = options_for_request(request);
    options
        .get(selected.min(options.len().saturating_sub(1)))
        .map(|option| option.text.clone())
}

pub fn shortcut_index(request: &PendingAgentRequest, key: char) -> Option<usize> {
    let key = key.to_ascii_lowercase();
    let options = options_for_request(request);
    options.iter().position(|option| {
        let label = option.label.to_ascii_lowercase();
        let text = option.text.to_ascii_lowercase();
        match key {
            'y' => text == "yes" || label.contains("yes") || label.contains("allow"),
            'n' => text == "no" || label.contains("no") || label.contains("deny"),
            'a' => text == "approve" || label.starts_with('a') || label.contains("approve"),
            's' => {
                text == "revise"
                    || label.starts_with('s')
                    || label.contains("request changes")
                    || label.contains("revise")
            }
            'q' => text == "abandon" || label.starts_with('q') || label.contains("abandon"),
            _ => false,
        }
    })
}

pub fn render(
    f: &mut Frame,
    area: Rect,
    request: &PendingAgentRequest,
    selected: usize,
    pending_count: usize,
) {
    let options = options_for_request(request);
    if options.is_empty() || area.width == 0 || area.height == 0 {
        return;
    }

    let is_plan = matches!(
        request.kind,
        PendingAgentRequestKind::GrokPlanApproval { .. }
    );
    let height = if is_plan {
        PLAN_OVERLAY_HEIGHT
    } else {
        OVERLAY_HEIGHT
    }
    .min(area.height);
    let overlay_area = Rect {
        x: area.x,
        y: area.y.saturating_add(area.height.saturating_sub(height)),
        width: area.width,
        height,
    };
    f.render_widget(Clear, overlay_area);

    let selected = selected.min(options.len().saturating_sub(1));
    // Borrow prompt/option labels for this frame — no per-line String clones.
    let mut lines = Vec::new();
    let prompt_line_limit = if is_plan { 8 } else { 2 };
    for prompt_line in request.prompt.lines().take(prompt_line_limit) {
        lines.push(Line::from(Span::raw(prompt_line)));
    }
    lines.push(Line::from(""));
    for (index, option) in options.iter().take(9).enumerate() {
        let style = if index == selected {
            theme::HIGHLIGHTED
        } else {
            Style::default()
        };
        lines.push(Line::from(vec![
            Span::styled(format!("{} ", index + 1), theme::MARKDOWN_CODE),
            Span::styled(option.label.as_str(), style),
        ]));
    }

    let title = if is_plan {
        if pending_count > 1 {
            format!(" Plan approval ({pending_count}) ")
        } else {
            " Plan approval ".to_owned()
        }
    } else if pending_count > 1 {
        format!(" Approval ({pending_count}) ")
    } else {
        " Approval ".to_owned()
    };
    let paragraph = Paragraph::new(lines)
        .block(
            Block::bordered()
                .title(Span::styled(
                    title,
                    Style::default().add_modifier(Modifier::BOLD),
                ))
                .border_style(theme::FOCUSED_BORDER),
        )
        .wrap(Wrap { trim: true });
    f.render_widget(paragraph, overlay_area);
}

fn options_for_request(request: &PendingAgentRequest) -> Vec<ApprovalOption> {
    match &request.kind {
        PendingAgentRequestKind::CodexApproval { .. }
        | PendingAgentRequestKind::OpencodePermission { .. } => yes_no_options(),
        PendingAgentRequestKind::GrokPlanApproval { .. } => plan_approval_options(),
        PendingAgentRequestKind::OpencodeQuestion { questions, .. } => {
            let [question] = questions.as_slice() else {
                return Vec::new();
            };
            if question.multiple || question.options.is_empty() {
                return Vec::new();
            }
            question
                .options
                .iter()
                .map(|option| ApprovalOption {
                    label: option.label.clone(),
                    text: option.label.clone(),
                })
                .collect()
        }
        PendingAgentRequestKind::CodexUserInput { .. } => Vec::new(),
    }
}

fn yes_no_options() -> Vec<ApprovalOption> {
    vec![
        ApprovalOption {
            label: "Yes".to_owned(),
            text: "yes".to_owned(),
        },
        ApprovalOption {
            label: "No".to_owned(),
            text: "no".to_owned(),
        },
    ]
}

/// Align with native Grok plan bar: a = approve, s = request changes, q = abandon.
fn plan_approval_options() -> Vec<ApprovalOption> {
    vec![
        ApprovalOption {
            label: "Approve & implement".to_owned(),
            text: "approve".to_owned(),
        },
        ApprovalOption {
            label: "Request changes".to_owned(),
            text: "revise".to_owned(),
        },
        ApprovalOption {
            label: "Abandon plan".to_owned(),
            text: "abandon".to_owned(),
        },
    ]
}

#[cfg(test)]
mod tests {
    use crate::translation::{PendingAgentRequest, PendingAgentRequestKind};

    use super::*;

    #[test]
    fn approval_options_submit_yes_or_no_text() {
        let request = PendingAgentRequest {
            prompt: "approve?".to_owned(),
            kind: PendingAgentRequestKind::CodexApproval {
                request_id: "r1".to_owned(),
                method: "execCommandApproval".to_owned(),
            },
        };

        assert_eq!(selected_text(&request, 0), Some("yes".to_owned()));
        assert_eq!(selected_text(&request, 1), Some("no".to_owned()));
        assert_eq!(shortcut_index(&request, 'y'), Some(0));
        assert_eq!(shortcut_index(&request, 'n'), Some(1));
    }

    #[test]
    fn plan_approval_options_use_asq_shortcuts() {
        let request = PendingAgentRequest {
            prompt: "plan".to_owned(),
            kind: PendingAgentRequestKind::GrokPlanApproval {
                request_id: "r-plan".to_owned(),
            },
        };
        assert_eq!(option_count(&request), 3);
        assert_eq!(selected_text(&request, 0), Some("approve".to_owned()));
        assert_eq!(selected_text(&request, 1), Some("revise".to_owned()));
        assert_eq!(selected_text(&request, 2), Some("abandon".to_owned()));
        assert_eq!(shortcut_index(&request, 'a'), Some(0));
        assert_eq!(shortcut_index(&request, 's'), Some(1));
        assert_eq!(shortcut_index(&request, 'q'), Some(2));
    }
}
