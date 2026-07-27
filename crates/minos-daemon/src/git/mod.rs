//! Host-local git operations for conversation work units.
//!
//! Phase 1–2 surface: worktree isolation, live status/diff, identity gate.
//! Phase 3: structured git activity payloads for the conversation timeline.

pub mod activity;
pub mod diff;
pub mod exec;
pub mod identity;
pub mod snapshot;
pub mod worktree;

pub use activity::{activity_summary, format_activity_body, parse_activity_body};
pub use diff::{get_diff, recent_commit_subjects, DiffFile, DiffResult};
pub use identity::{read_identity, GitIdentity};
pub use snapshot::{
    current_branch_name, detect_git_snapshot, detect_live_status, resolve_work_path, LiveGitStatus,
};
pub use worktree::{
    create_conversation_worktree, default_branch_name, remove_conversation_worktree,
    slugify_segment, worktrees_root_for_repo, WorktreeCreateResult,
};
