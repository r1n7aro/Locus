use serde_json::Value;

/// Formats one value for a stable `key=value` tool-result field.
/// Strings stay lossless through JSON escaping while arrays and objects use
/// compact JSON so a single logical record always remains on one line.
pub(crate) fn flat_json_value(value: &Value) -> String {
    match value {
        Value::String(value) => serde_json::to_string(value).unwrap_or_else(|_| "\"\"".to_string()),
        Value::Null => "null".to_string(),
        Value::Bool(_) | Value::Number(_) | Value::Array(_) | Value::Object(_) => {
            serde_json::to_string(value).unwrap_or_else(|_| "null".to_string())
        }
    }
}

pub(crate) fn flat_text(value: &str) -> String {
    flat_json_value(&Value::String(value.to_string()))
}

pub(crate) fn append_field(line: &mut String, key: &str, value: impl std::fmt::Display) {
    line.push(' ');
    line.push_str(key);
    line.push('=');
    line.push_str(&value.to_string());
}

pub(crate) fn append_text_field(line: &mut String, key: &str, value: &str) {
    if value.trim().is_empty() {
        return;
    }
    append_field(line, key, flat_text(value));
}

pub(crate) fn append_json_field(line: &mut String, key: &str, value: Option<&Value>) {
    let Some(value) = value else {
        return;
    };
    let empty = value.is_null()
        || value.as_str().is_some_and(|value| value.trim().is_empty())
        || value.as_array().is_some_and(Vec::is_empty)
        || value.as_object().is_some_and(serde_json::Map::is_empty);
    if empty {
        return;
    }
    append_field(line, key, flat_json_value(value));
}

pub(crate) fn push_indented_text(output: &mut String, label: &str, value: &str) {
    if value.trim().is_empty() {
        return;
    }
    output.push('\n');
    output.push_str("  ");
    output.push_str(label);
    output.push_str(":");
    for line in value.lines() {
        output.push('\n');
        output.push_str("    ");
        output.push_str(line);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flat_values_keep_each_record_on_one_line() {
        assert_eq!(flat_text("Replay Timeline"), "\"Replay Timeline\"");
        assert_eq!(flat_text("a\nb"), "\"a\\nb\"");
        assert_eq!(
            flat_json_value(&serde_json::json!([1, "two"])),
            "[1,\"two\"]"
        );
    }

    #[test]
    fn optional_json_fields_skip_empty_values() {
        let mut line = "Result:".to_string();
        append_json_field(&mut line, "empty", Some(&serde_json::json!([])));
        append_json_field(&mut line, "count", Some(&serde_json::json!(2)));
        assert_eq!(line, "Result: count=2");
    }
}
