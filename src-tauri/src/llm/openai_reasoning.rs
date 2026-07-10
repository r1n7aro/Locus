pub fn apply_reasoning_effort(
    body: &mut serde_json::Value,
    model: &str,
    thinking_level: Option<&str>,
) {
    if let Some(effort) = reasoning_effort_for_model(model, thinking_level) {
        body["reasoning"] = serde_json::json!({ "effort": effort });
    }
}

pub fn apply_explicit_reasoning_effort(body: &mut serde_json::Value, effort: Option<&str>) {
    if let Some(effort) = effort.and_then(normalize_explicit_reasoning_effort) {
        body["reasoning"] = serde_json::json!({ "effort": effort });
    }
}

pub fn custom_reasoning_effort(
    thinking_level: Option<&str>,
    supported_efforts: &[String],
) -> Option<String> {
    let level = normalize_explicit_reasoning_effort(thinking_level?)?;
    let supported = supported_efforts
        .iter()
        .filter_map(|value| normalize_explicit_reasoning_effort(value))
        .collect::<std::collections::HashSet<_>>();
    supported.contains(&level).then_some(level)
}

fn normalize_explicit_reasoning_effort(value: &str) -> Option<String> {
    let level = value.trim().to_ascii_lowercase();
    match level.as_str() {
        "low" | "medium" | "high" | "xhigh" | "max" => Some(level),
        _ => None,
    }
}

pub fn apply_text_verbosity_default(body: &mut serde_json::Value, model: &str) {
    let Some(verbosity) = default_text_verbosity_for_model(model) else {
        return;
    };
    let Some(body_obj) = body.as_object_mut() else {
        return;
    };

    let text_value = body_obj
        .entry("text".to_string())
        .or_insert_with(|| serde_json::json!({}));
    if !text_value.is_object() {
        *text_value = serde_json::json!({});
    }
    if let Some(text_obj) = text_value.as_object_mut() {
        text_obj
            .entry("verbosity".to_string())
            .or_insert_with(|| serde_json::json!(verbosity));
    }
}

pub fn default_text_verbosity_for_model(model: &str) -> Option<&'static str> {
    let model = normalize_model_name(model);
    if model.contains("gpt-5") {
        Some("low")
    } else {
        None
    }
}

pub fn reasoning_effort_for_model(
    model: &str,
    thinking_level: Option<&str>,
) -> Option<&'static str> {
    let level = thinking_level?.trim().to_ascii_lowercase();
    if level.is_empty() {
        return None;
    }

    let supported = supported_efforts(model);
    match level.as_str() {
        "none" if supported.none => Some("none"),
        "low" if supported.low => Some("low"),
        "medium" if supported.medium => Some("medium"),
        "high" if supported.high => Some("high"),
        "xhigh" if supported.xhigh => Some("xhigh"),
        "max" if supported.max => Some("max"),
        _ => None,
    }
}

fn normalize_model_name(model: &str) -> String {
    let model = model.trim();
    let model = model.strip_prefix("openai/").unwrap_or(model);
    model.to_ascii_lowercase()
}

#[derive(Clone, Copy)]
struct SupportedEfforts {
    none: bool,
    low: bool,
    medium: bool,
    high: bool,
    xhigh: bool,
    max: bool,
}

const LOW_MEDIUM_HIGH_XHIGH: SupportedEfforts = SupportedEfforts {
    none: false,
    low: true,
    medium: true,
    high: true,
    xhigh: true,
    max: false,
};

const LOW_MEDIUM_HIGH_XHIGH_MAX: SupportedEfforts = SupportedEfforts {
    none: false,
    low: true,
    medium: true,
    high: true,
    xhigh: true,
    max: true,
};

const MEDIUM_HIGH: SupportedEfforts = SupportedEfforts {
    none: false,
    low: false,
    medium: true,
    high: true,
    xhigh: false,
    max: false,
};

const HIGH_ONLY: SupportedEfforts = SupportedEfforts {
    none: false,
    low: false,
    medium: false,
    high: true,
    xhigh: false,
    max: false,
};

const UNSUPPORTED: SupportedEfforts = SupportedEfforts {
    none: false,
    low: false,
    medium: false,
    high: false,
    xhigh: false,
    max: false,
};

fn supported_efforts(model: &str) -> SupportedEfforts {
    let model = model.trim().to_ascii_lowercase();

    if model.contains("gpt-5.6") {
        return LOW_MEDIUM_HIGH_XHIGH_MAX;
    }
    if model.contains("gpt-5.5-pro")
        || model.contains("gpt-5.4-pro")
        || model.contains("gpt-5.2-pro")
    {
        return MEDIUM_HIGH;
    }
    if model.contains("gpt-5-pro") {
        return HIGH_ONLY;
    }
    if model.contains("gpt-5.1-codex-mini") {
        return MEDIUM_HIGH;
    }
    if model.contains("codex") {
        return LOW_MEDIUM_HIGH_XHIGH;
    }
    if model.contains("gpt-5.5")
        || model.contains("gpt-5.4")
        || model.contains("gpt-5.2")
        || model.contains("gpt-5.1")
    {
        return LOW_MEDIUM_HIGH_XHIGH;
    }
    if model.contains("gpt-5") {
        return LOW_MEDIUM_HIGH_XHIGH;
    }

    UNSUPPORTED
}

#[cfg(test)]
mod tests {
    use super::{
        apply_explicit_reasoning_effort, apply_reasoning_effort, apply_text_verbosity_default,
        custom_reasoning_effort, default_text_verbosity_for_model, reasoning_effort_for_model,
    };

    #[test]
    fn gpt55_accepts_xhigh_and_hides_none() {
        assert_eq!(
            reasoning_effort_for_model("gpt-5.5", Some("xhigh")),
            Some("xhigh")
        );
        assert_eq!(reasoning_effort_for_model("gpt-5.5", Some("none")), None);
    }

    #[test]
    fn gpt56_accepts_max_and_hides_none() {
        assert_eq!(
            reasoning_effort_for_model("gpt-5.6-sol", Some("max")),
            Some("max")
        );
        assert_eq!(
            reasoning_effort_for_model("gpt-5.6-terra", Some("xhigh")),
            Some("xhigh")
        );
        assert_eq!(
            reasoning_effort_for_model("gpt-5.6-luna", Some("none")),
            None
        );
    }

    #[test]
    fn codex_accepts_low_medium_high_xhigh_only() {
        assert_eq!(
            reasoning_effort_for_model("gpt-5.3-codex", Some("low")),
            Some("low")
        );
        assert_eq!(
            reasoning_effort_for_model("gpt-5.3-codex", Some("high")),
            Some("high")
        );
        assert_eq!(
            reasoning_effort_for_model("gpt-5.3-codex", Some("xhigh")),
            Some("xhigh")
        );
        assert_eq!(
            reasoning_effort_for_model("gpt-5.3-codex", Some("none")),
            None
        );
    }

    #[test]
    fn unsupported_models_omit_reasoning_effort() {
        assert_eq!(reasoning_effort_for_model("gpt-4.1", Some("high")), None);
    }

    #[test]
    fn gpt5_models_default_to_low_text_verbosity() {
        assert_eq!(default_text_verbosity_for_model("gpt-5.5"), Some("low"));
        assert_eq!(default_text_verbosity_for_model("gpt-5.4"), Some("low"));
        assert_eq!(
            default_text_verbosity_for_model("openai/gpt-5.3-codex"),
            Some("low")
        );
        assert_eq!(default_text_verbosity_for_model("gpt-4.1"), None);
    }

    #[test]
    fn injects_reasoning_effort_into_request_body() {
        let mut body = serde_json::json!({
            "model": "gpt-5.5",
            "input": [],
            "stream": true,
        });

        apply_reasoning_effort(&mut body, "gpt-5.5", Some("xhigh"));

        assert_eq!(body["reasoning"], serde_json::json!({ "effort": "xhigh" }));
    }

    #[test]
    fn custom_reasoning_accepts_explicit_max_effort() {
        let supported = vec![
            "low".to_string(),
            "medium".to_string(),
            "high".to_string(),
            "max".to_string(),
        ];
        assert_eq!(
            custom_reasoning_effort(Some("max"), &supported),
            Some("max".to_string())
        );

        let mut body = serde_json::json!({ "model": "deepseek-v4-pro" });
        apply_explicit_reasoning_effort(&mut body, Some("max"));
        assert_eq!(body["reasoning"], serde_json::json!({ "effort": "max" }));
    }

    #[test]
    fn injects_default_text_verbosity_into_request_body() {
        let mut body = serde_json::json!({
            "model": "gpt-5.4",
            "input": [],
            "stream": true,
        });

        apply_text_verbosity_default(&mut body, "gpt-5.4");

        assert_eq!(body["text"], serde_json::json!({ "verbosity": "low" }));
    }

    #[test]
    fn preserves_existing_text_settings_when_injecting_verbosity() {
        let mut body = serde_json::json!({
            "model": "gpt-5.4",
            "input": [],
            "text": {
                "format": {
                    "type": "json_schema"
                }
            }
        });

        apply_text_verbosity_default(&mut body, "gpt-5.4");

        assert_eq!(body["text"]["verbosity"], serde_json::json!("low"));
        assert_eq!(
            body["text"]["format"]["type"],
            serde_json::json!("json_schema")
        );
    }

    #[test]
    fn leaves_request_body_unchanged_when_effort_is_unsupported() {
        let mut body = serde_json::json!({
            "model": "gpt-4.1",
            "input": [],
            "stream": true,
        });

        apply_reasoning_effort(&mut body, "gpt-4.1", Some("high"));

        assert!(body.get("reasoning").is_none());
    }

    #[test]
    fn leaves_request_body_without_text_for_unsupported_models() {
        let mut body = serde_json::json!({
            "model": "gpt-4.1",
            "input": [],
            "stream": true,
        });

        apply_text_verbosity_default(&mut body, "gpt-4.1");

        assert!(body.get("text").is_none());
    }
}
