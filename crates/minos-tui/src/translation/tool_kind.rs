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
    pub fn from_tool_name(name: &str) -> Self {
        let n = name.to_ascii_lowercase();
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
        if n.contains("grep")
            || n.contains("search")
            || n.contains("glob")
            || n.contains("find")
            || n.contains("rg")
        {
            return Self::Search;
        }
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
        if running { present } else { past }
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
