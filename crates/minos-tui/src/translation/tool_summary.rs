use super::tool_kind::ToolKind;

fn truncate_str(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_owned()
    } else {
        let mut end = max_len;
        while !s.is_char_boundary(end) {
            end -= 1;
        }
        format!("{}…", &s[..end])
    }
}

/// Primary one-line target for a tool header (bare path/command/pattern — no `file=` labels).
pub(super) fn summarize_tool_args(tool_name: &str, args_json: &str) -> String {
    let Some(value) = parse_tool_args(args_json) else {
        return truncate_str(&one_line(args_json), 180);
    };

    if value.is_null() {
        return String::new();
    }

    let kind = ToolKind::from_tool_name(tool_name);

    match kind {
        ToolKind::Read | ToolKind::Edit | ToolKind::List => {
            if let Some(path) = find_path(&value) {
                return truncate_str(&one_line(&path), 120);
            }
        }
        ToolKind::Execute => {
            if let Some(cmd) = find_stringish(&value, &["cmd", "command", "script", "shell"]) {
                return truncate_str(&one_line(&cmd), 120);
            }
        }
        ToolKind::Search => {
            let pattern = find_stringish(
                &value,
                &["pattern", "query", "regex", "search", "grep", "needle"],
            );
            let path = find_path(&value);
            return match (pattern, path) {
                (Some(p), Some(path)) => {
                    truncate_str(&format!("{} in {}", one_line(&p), one_line(&path)), 140)
                }
                (Some(p), None) => truncate_str(&one_line(&p), 120),
                (None, Some(path)) => truncate_str(&one_line(&path), 120),
                (None, None) => String::new(),
            };
        }
        ToolKind::WebSearch | ToolKind::WebFetch => {
            if let Some(q) = find_stringish(&value, &["query", "url", "uri", "href", "q"]) {
                return truncate_str(&one_line(&q), 120);
            }
        }
        ToolKind::Skill => {
            if let Some(skill) = find_stringish(
                &value,
                &[
                    "skill",
                    "skill_name",
                    "skillName",
                    "name",
                    "skill_path",
                    "skillPath",
                ],
            ) {
                return truncate_str(&one_line(&skill), 90);
            }
        }
        ToolKind::Other => {}
    }

    // Task / todo tools: prefer human description.
    if tool_name.to_ascii_lowercase().contains("task")
        || tool_name.eq_ignore_ascii_case("todo")
        || tool_name.eq_ignore_ascii_case("todowrite")
        || tool_name.eq_ignore_ascii_case("todo_write")
    {
        if let Some(task) = find_stringish(
            &value,
            &[
                "task",
                "description",
                "prompt",
                "instructions",
                "question",
                "subagent_type",
                "subagentType",
            ],
        ) {
            return truncate_str(&one_line(&task), 110);
        }
    }

    // Generic: first useful scalar (path/cmd/description), else compact JSON.
    if let Some(path) = find_path(&value) {
        return truncate_str(&one_line(&path), 120);
    }
    if let Some(cmd) = find_stringish(&value, &["cmd", "command", "script", "shell"]) {
        return truncate_str(&one_line(&cmd), 120);
    }
    if let Some(desc) = find_stringish(&value, &["description", "task", "prompt", "query"]) {
        return truncate_str(&one_line(&desc), 120);
    }
    if let Some(count) = array_len_for_keys(&value, &["todos", "todo", "items"]) {
        return format!("{count} items");
    }

    compact_tool_args(args_json).unwrap_or_default()
}

pub(super) fn compact_tool_args(args_json: &str) -> Option<String> {
    let value = parse_tool_args(args_json)?;
    if value.is_null() {
        return Some(String::new());
    }
    serde_json::to_string(&value)
        .ok()
        .map(|text| truncate_str(&one_line(&text), 500))
}

pub(super) fn summarize_tool_output(output: &str) -> String {
    let trimmed = output.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    if is_diff_like(trimmed) {
        let (add, del) = count_diff_lines(trimmed);
        // Compact form for header diffstat painting (` +N/-M`).
        return format!("+{add}/-{del}");
    }
    truncate_str(&one_line(trimmed), 220)
}

/// Parse `+N/-M` or legacy `diff +N -M` summary into insert/delete counts.
pub(crate) fn parse_diffstat(summary: &str) -> Option<(usize, usize)> {
    let s = summary.trim();
    // `+12/-3`
    if let Some(rest) = s.strip_prefix('+') {
        if let Some((add, del)) = rest.split_once("/-") {
            let add = add.trim().parse().ok()?;
            let del = del.trim().parse().ok()?;
            return Some((add, del));
        }
    }
    // legacy `diff +12 -3`
    if let Some(rest) = s.strip_prefix("diff ") {
        let mut add = None;
        let mut del = None;
        for part in rest.split_whitespace() {
            if let Some(n) = part.strip_prefix('+') {
                add = n.parse().ok();
            } else if let Some(n) = part.strip_prefix('-') {
                del = n.parse().ok();
            }
        }
        if let (Some(a), Some(d)) = (add, del) {
            return Some((a, d));
        }
    }
    None
}

pub(super) fn tool_output_detail(output: &str) -> Option<String> {
    let trimmed = output.trim();
    if trimmed.is_empty() {
        return None;
    }
    // Diffs keep more context for AgentDetail fold expansion; other long
    // outputs still cap so a single tool cannot blow the render cache.
    let limit = if is_diff_like(trimmed) { 16_000 } else { 8_000 };
    if is_diff_like(trimmed) || trimmed.len() > 220 || trimmed.contains('\n') {
        return Some(truncate_str(trimmed, limit));
    }
    None
}

pub(super) fn is_diff_like(text: &str) -> bool {
    text.contains("diff --git")
        || text.contains("\n@@")
        || text.starts_with("@@")
        || text.contains("*** Begin Patch")
        || text.contains("*** Update File:")
        || text.contains("*** Add File:")
        || text.contains("*** Delete File:")
        || text.contains("*** End Patch")
        || text
            .lines()
            .any(|line| line.starts_with("+++ ") || line.starts_with("--- "))
}

fn count_diff_lines(text: &str) -> (usize, usize) {
    let add = text
        .lines()
        .filter(|line| line.starts_with('+') && !line.starts_with("+++"))
        .count();
    let del = text
        .lines()
        .filter(|line| line.starts_with('-') && !line.starts_with("---"))
        .count();
    (add, del)
}

fn parse_tool_args(args_json: &str) -> Option<serde_json::Value> {
    let trimmed = args_json.trim();
    if trimmed.is_empty() {
        return None;
    }
    serde_json::from_str(trimmed).ok()
}

fn one_line(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn find_path(value: &serde_json::Value) -> Option<String> {
    find_stringish(
        value,
        &[
            "file_path",
            "filePath",
            "filepath",
            "path",
            "absolute_path",
            "absolutePath",
            "relative_path",
            "relativePath",
            "target_file",
            "targetFile",
            "file",
            "uri",
        ],
    )
}

fn find_stringish(value: &serde_json::Value, keys: &[&str]) -> Option<String> {
    find_stringish_inner(value, keys, 0)
}

fn find_stringish_inner(value: &serde_json::Value, keys: &[&str], depth: usize) -> Option<String> {
    if depth > 4 {
        return None;
    }

    match value {
        serde_json::Value::Object(map) => {
            for key in keys {
                if let Some(found) = map.get(*key).and_then(value_to_summary_text) {
                    return Some(found);
                }
            }
            for child_key in [
                "input",
                "args",
                "arguments",
                "params",
                "tool_input",
                "toolInput",
                "metadata",
                "state",
            ] {
                if let Some(found) = map
                    .get(child_key)
                    .and_then(|child| find_stringish_inner(child, keys, depth + 1))
                {
                    return Some(found);
                }
            }
            map.values()
                .find_map(|child| find_stringish_inner(child, keys, depth + 1))
        }
        serde_json::Value::Array(items) => items
            .iter()
            .find_map(|child| find_stringish_inner(child, keys, depth + 1)),
        _ => None,
    }
}

fn value_to_summary_text(value: &serde_json::Value) -> Option<String> {
    match value {
        serde_json::Value::String(text) => (!text.trim().is_empty()).then(|| text.to_owned()),
        serde_json::Value::Number(number) => Some(number.to_string()),
        serde_json::Value::Bool(boolean) => Some(boolean.to_string()),
        serde_json::Value::Array(items) => {
            let values = items
                .iter()
                .filter_map(value_to_summary_text)
                .collect::<Vec<_>>();
            (!values.is_empty()).then(|| values.join(","))
        }
        serde_json::Value::Object(map) => {
            for key in [
                "name",
                "path",
                "file_path",
                "filePath",
                "description",
                "task",
                "prompt",
            ] {
                if let Some(text) = map.get(key).and_then(value_to_summary_text) {
                    return Some(text);
                }
            }
            None
        }
        serde_json::Value::Null => None,
    }
}

fn array_len_for_keys(value: &serde_json::Value, keys: &[&str]) -> Option<usize> {
    match value {
        serde_json::Value::Object(map) => {
            for key in keys {
                if let Some(len) = map
                    .get(*key)
                    .and_then(|value| value.as_array().map(Vec::len))
                {
                    return Some(len);
                }
            }
            map.values()
                .find_map(|child| array_len_for_keys(child, keys))
        }
        serde_json::Value::Array(items) => items
            .iter()
            .find_map(|child| array_len_for_keys(child, keys)),
        _ => None,
    }
}
