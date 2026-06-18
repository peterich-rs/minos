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

pub(super) fn summarize_tool_args(tool_name: &str, args_json: &str) -> String {
    let Some(value) = parse_tool_args(args_json) else {
        return truncate_str(&one_line(args_json), 180);
    };

    if value.is_null() {
        return String::new();
    }

    let lower_name = tool_name.to_ascii_lowercase();
    let mut pieces = Vec::new();

    if let Some(value) = find_stringish(
        &value,
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
    ) {
        pieces.push(summary_piece("file", &value, 90));
    }

    if let Some(value) = find_stringish(&value, &["cmd", "command", "script", "shell"]) {
        pieces.push(summary_piece("cmd", &value, 90));
    }

    if lower_name.contains("task")
        || lower_name == "todo"
        || lower_name == "todowrite"
        || lower_name == "todo_write"
    {
        if let Some(value) = find_stringish(
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
            pieces.push(summary_piece("task", &value, 110));
        }
    } else if let Some(value) = find_stringish(&value, &["task", "description"]) {
        pieces.push(summary_piece("task", &value, 110));
    }

    if lower_name.contains("skill") {
        if let Some(value) = find_stringish(
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
            pieces.push(summary_piece("skill", &value, 90));
        }
    } else if let Some(value) = find_stringish(&value, &["skill", "skill_name", "skillName"]) {
        pieces.push(summary_piece("skill", &value, 90));
    }

    if let Some(count) = array_len_for_keys(&value, &["todos", "todo", "items"]) {
        pieces.push(format!("items={count}"));
    }

    if pieces.is_empty() {
        compact_tool_args(args_json).unwrap_or_default()
    } else {
        truncate_str(&pieces.join(" "), 180)
    }
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
        let add = trimmed
            .lines()
            .filter(|line| line.starts_with('+') && !line.starts_with("+++"))
            .count();
        let del = trimmed
            .lines()
            .filter(|line| line.starts_with('-') && !line.starts_with("---"))
            .count();
        return format!("diff +{add} -{del}");
    }
    truncate_str(&one_line(trimmed), 220)
}

pub(super) fn tool_output_detail(output: &str) -> Option<String> {
    let trimmed = output.trim();
    if trimmed.is_empty() {
        return None;
    }
    if is_diff_like(trimmed) || trimmed.len() > 220 || trimmed.contains('\n') {
        return Some(truncate_str(trimmed, 6000));
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
        || text
            .lines()
            .any(|line| line.starts_with("+++ ") || line.starts_with("--- "))
}

fn parse_tool_args(args_json: &str) -> Option<serde_json::Value> {
    let trimmed = args_json.trim();
    if trimmed.is_empty() {
        return None;
    }
    serde_json::from_str(trimmed).ok()
}

fn summary_piece(label: &str, value: &str, max_len: usize) -> String {
    format!("{label}={}", truncate_str(&one_line(value), max_len))
}

fn one_line(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
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
