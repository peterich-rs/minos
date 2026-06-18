pub(super) fn find_array_by_key<'a>(
    value: &'a serde_json::Value,
    key: &str,
) -> Option<&'a Vec<serde_json::Value>> {
    match value {
        serde_json::Value::Object(map) => {
            if let Some(array) = map.get(key).and_then(serde_json::Value::as_array) {
                return Some(array);
            }
            map.values().find_map(|child| find_array_by_key(child, key))
        }
        serde_json::Value::Array(values) => values
            .iter()
            .find_map(|child| find_array_by_key(child, key)),
        _ => None,
    }
}

pub(super) fn find_string_by_keys(value: &serde_json::Value, keys: &[&str]) -> Option<String> {
    find_string_by_keys_inner(value, keys, 0)
}

pub(super) fn direct_string_by_keys(value: &serde_json::Value, keys: &[&str]) -> Option<String> {
    let serde_json::Value::Object(map) = value else {
        return None;
    };
    keys.iter()
        .find_map(|key| map.get(*key).and_then(json_value_summary))
}

fn find_string_by_keys_inner(
    value: &serde_json::Value,
    keys: &[&str],
    depth: usize,
) -> Option<String> {
    if depth > 5 {
        return None;
    }
    match value {
        serde_json::Value::Object(map) => {
            for key in keys {
                if let Some(text) = map.get(*key).and_then(json_value_summary) {
                    return Some(text);
                }
            }
            map.values()
                .find_map(|child| find_string_by_keys_inner(child, keys, depth + 1))
        }
        serde_json::Value::Array(values) => values
            .iter()
            .find_map(|child| find_string_by_keys_inner(child, keys, depth + 1)),
        _ => None,
    }
}

pub(super) fn json_value_summary(value: &serde_json::Value) -> Option<String> {
    match value {
        serde_json::Value::String(text) => (!text.trim().is_empty()).then(|| text.to_owned()),
        serde_json::Value::Number(number) => Some(number.to_string()),
        serde_json::Value::Bool(value) => Some(value.to_string()),
        serde_json::Value::Array(values) => {
            let parts = values
                .iter()
                .filter_map(json_value_summary)
                .collect::<Vec<_>>();
            (!parts.is_empty()).then(|| parts.join(" "))
        }
        serde_json::Value::Object(map) => {
            for key in [
                "title", "name", "path", "command", "cmd", "message", "reason",
            ] {
                if let Some(text) = map.get(key).and_then(json_value_summary) {
                    return Some(text);
                }
            }
            None
        }
        serde_json::Value::Null => None,
    }
}
