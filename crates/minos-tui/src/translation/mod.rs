mod chat_item;
mod chat_state;
mod json_helpers;
mod pending_request;
mod selection;
mod tool_kind;
mod tool_summary;
mod verb_group;

pub use chat_item::{ChatItem, TextPart};
pub use chat_state::ChatState;
pub use pending_request::{PendingAgentRequest, PendingAgentRequestKind, PendingQuestionSpec};
pub use selection::{ChatSelection, ChatSelectionPoint};
pub use tool_kind::ToolKind;
pub(crate) use tool_summary::parse_diffstat;
pub(crate) use verb_group::{find_runs, header_label, paint_mode_with_runs};
pub use verb_group::{PaintMode, VerbGroupRun};

#[cfg(test)]
#[path = "translation_tests.rs"]
mod tests;
