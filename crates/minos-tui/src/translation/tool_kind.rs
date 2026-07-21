//! Semantic tool classification for AgentDetail transcript rendering.
//!
//! Maps heterogeneous agent tool names onto Grok-style verbs and body layouts
//! without forking Grok's full scrollback block types.

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ToolKind {
    Read,
    Edit,
    Execute,
    Search,
    List,
    WebFetch,
    WebSearch,
    Skill,
    Other,
}

impl ToolKind {
    /// Classify a tool by name (case-insensitive heuristics shared by paint + projection).
    ///
    /// Input is the unified translator `ToolCallPlaced.name` — any agent (Codex /
    /// Claude / Grok / …). Prefer an explicit leading kind token when present
    /// (`"read: path"`, `"execute: cargo test"`) so display stays agent-agnostic.
    pub fn from_tool_name(name: &str) -> Self {
        let n = name.to_ascii_lowercase();
        // Translator convention: "{kind}: {subject}" — classify by the kind token
        // first so subject text (e.g. a path containing "search") cannot mis-route.
        if let Some(prefix) = n.split_once(':').map(|(p, _)| p.trim()) {
            if let Some(kind) = Self::from_kind_token(prefix) {
                return kind;
            }
        }
        Self::from_kind_token(n.trim()).unwrap_or_else(|| Self::from_name_heuristic(&n))
    }

    /// Strip a leading `"kind: "` display prefix if present (leave subject only).
    #[must_use]
    pub fn subject_from_tool_name(name: &str) -> &str {
        if let Some((prefix, rest)) = name.split_once(':') {
            let token = prefix.trim().to_ascii_lowercase();
            if Self::from_kind_token(&token).is_some() {
                let subject = rest.trim();
                if !subject.is_empty() {
                    return subject;
                }
            }
        }
        name.trim()
    }

    fn from_kind_token(token: &str) -> Option<Self> {
        match token {
            "read" | "read_file" | "readfile" | "cat" => Some(Self::Read),
            "edit" | "write" | "diff" | "search_replace" | "apply_patch" | "str_replace" => {
                Some(Self::Edit)
            }
            "execute" | "terminal" | "bash" | "shell" | "run" | "command" => Some(Self::Execute),
            "search" | "grep" | "glob" | "find" | "rg" => Some(Self::Search),
            "list" | "list_dir" | "listdir" | "list_directory" | "ls" => Some(Self::List),
            "web_fetch" | "webfetch" | "fetch" => Some(Self::WebFetch),
            "web_search" | "websearch" => Some(Self::WebSearch),
            "skill" => Some(Self::Skill),
            "other" => Some(Self::Other),
            _ => None,
        }
    }

    fn from_name_heuristic(n: &str) -> Self {
        if n.contains("skill") {
            return Self::Skill;
        }
        if n.contains("web_search") || n == "websearch" {
            return Self::WebSearch;
        }
        if n.contains("web_fetch") || n.contains("webfetch") || n == "fetch" {
            return Self::WebFetch;
        }
        if n.contains("list_dir")
            || n.contains("listdir")
            || n.contains("list_directory")
            || n == "ls"
            || n.contains("glob_file")
        {
            return Self::List;
        }
        // Edit before search: names like `search_replace` contain "search".
        if n.contains("write")
            || n.contains("edit")
            || n.contains("apply_patch")
            || n.contains("str_replace")
            || n.contains("search_replace")
            || n.contains("create_file")
            || n.contains("delete_file")
        {
            return Self::Edit;
        }
        if n.contains("grep")
            || n.contains("search")
            || n.contains("glob")
            || n.contains("find")
            || n.contains("rg")
        {
            return Self::Search;
        }
        if n.contains("read") || n == "cat" || n.ends_with("_read") {
            return Self::Read;
        }
        if n.contains("bash")
            || n.contains("shell")
            || n.contains("exec")
            || n.contains("command")
            || n == "run_terminal_command"
            || n == "run"
        {
            return Self::Execute;
        }
        Self::Other
    }

    /// Grok verb-group row verb: present tense while running, past otherwise.
    pub fn verb(self, running: bool) -> &'static str {
        let (past, present) = match self {
            Self::Read | Self::Skill => ("Read", "Reading"),
            Self::Search | Self::WebSearch => ("Searched", "Searching"),
            Self::List => ("Listed", "Listing"),
            Self::WebFetch => ("Fetched", "Fetching"),
            Self::Edit => ("Edited", "Editing"),
            Self::Execute | Self::Other => ("Ran", "Running"),
        };
        if running {
            present
        } else {
            past
        }
    }

    /// Skill uses "Skill" for both tenses (matches Grok skill header).
    pub fn header_verb(self, running: bool) -> &'static str {
        if matches!(self, Self::Skill) {
            "Skill"
        } else {
            self.verb(running)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_unified_kind_prefix_from_any_agent() {
        assert_eq!(
            ToolKind::from_tool_name("read: src/main.rs"),
            ToolKind::Read
        );
        assert_eq!(
            ToolKind::from_tool_name("execute: cargo test"),
            ToolKind::Execute
        );
        assert_eq!(
            ToolKind::from_tool_name("search: foo in crates/"),
            ToolKind::Search
        );
        // Subject containing "search" must not override kind prefix.
        assert_eq!(
            ToolKind::from_tool_name("read: search_results.txt"),
            ToolKind::Read
        );
    }

    #[test]
    fn subject_strips_kind_prefix() {
        assert_eq!(
            ToolKind::subject_from_tool_name("read: src/main.rs"),
            "src/main.rs"
        );
        assert_eq!(
            ToolKind::subject_from_tool_name("execute: cargo test"),
            "cargo test"
        );
        assert_eq!(ToolKind::subject_from_tool_name("read_file"), "read_file");
    }

    #[test]
    fn bare_cli_tool_names_still_work() {
        assert_eq!(ToolKind::from_tool_name("Read"), ToolKind::Read);
        assert_eq!(ToolKind::from_tool_name("Bash"), ToolKind::Execute);
        assert_eq!(ToolKind::from_tool_name("search_replace"), ToolKind::Edit);
        assert_eq!(ToolKind::from_tool_name("grep"), ToolKind::Search);
    }
}
