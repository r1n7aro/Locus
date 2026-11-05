//! Human-facing projections of native tool results. Protocol payloads must not
//! double as transcript text (notably Write's old/new file bodies).
use serde_json::{json, Value};

fn text(value: &Value) -> &str {
    value.as_str().unwrap_or_default()
}

fn lines(value: &Value) -> String {
    value
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>()
                .join("\n")
        })
        .unwrap_or_default()
}

pub(crate) fn render(tool: &str, value: &Value) -> String {
    match tool {
        "Write" => format!(
            "{} {}",
            if value["type"] == "update" {
                "Edited"
            } else {
                "Created"
            },
            text(&value["filePath"])
        ),
        "Edit" => format!("Edited {}", text(&value["filePath"])),
        "Glob" => format!(
            "Found {} files (showing {}){}\n{}",
            value.get("totalMatches").unwrap_or(&value["numFiles"]),
            value["numFiles"],
            if value["truncated"] == true {
                "; truncated, narrow the pattern"
            } else {
                ""
            },
            lines(&value["filenames"])
        ),
        "Grep" => format!(
            "Found {} matches in {} files; showing {} {} (offset {}, limit {})\n{}",
            value["numMatches"],
            value["totalFiles"],
            if value["mode"] == "files_with_matches" {
                &value["numFiles"]
            } else {
                &value["numLines"]
            },
            if value["mode"] == "files_with_matches" {
                "files"
            } else {
                "lines"
            },
            value["appliedOffset"],
            value["appliedLimit"],
            if value["mode"] == "files_with_matches" {
                lines(&value["filenames"])
            } else {
                text(&value["content"]).to_string()
            }
        ),
        "TaskOutput" => format!(
            "Task {}: {} (retrieval: {})\n{}",
            text(&value["task"]["task_id"]),
            text(&value["task"]["status"]),
            text(&value["retrieval_status"]),
            text(&value["task"]["output"])
        ),
        "TaskStop" => format!("{}: {}", text(&value["message"]), text(&value["task_id"])),
        "AskUserQuestion" => value["answers"]
            .as_object()
            .map(|answers| {
                answers
                    .iter()
                    .map(|(question, answer)| {
                        format!("{question}\nUser answered: {}", text(answer))
                    })
                    .collect::<Vec<_>>()
                    .join("\n\n")
            })
            .unwrap_or_default(),
        "Bash" => [text(&value["stdout"]), text(&value["stderr"])]
            .into_iter()
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join("\n"),
        "Read" if value["type"] == "image" => "Image file read. Image attached.".into(),
        "Read" if value["type"] == "text" => {
            let file = &value["file"];
            let start = file["startLine"].as_u64().unwrap_or(1);
            text(&file["content"])
                .split('\n')
                .enumerate()
                .map(|(i, line)| format!("{}\t{line}", start + i as u64))
                .collect::<Vec<_>>()
                .join("\n")
        }
        _ => readable(value),
    }
}

// Preserve unknown CLI fields, but render embedded text as text, not JSON escapes.
fn readable(value: &Value) -> String {
    match value {
        Value::String(value) => value.clone(),
        Value::Array(values) => values.iter().map(readable).collect::<Vec<_>>().join("\n"),
        Value::Object(values) => values
            .iter()
            .map(|(key, value)| format!("{key}: {}", readable(value)))
            .collect::<Vec<_>>()
            .join("\n"),
        Value::Null => String::new(),
        _ => value.to_string(),
    }
}

/// Session tools execute in Claude. Keep their synchronization snapshot out of
/// display text. The small items envelope still drives the existing todo stream
/// reducer; the complete snapshot is stored separately by the host.
pub(crate) fn external(tool: &str, output: &str) -> String {
    if tool == "Agent" {
        return agent_result_text(output);
    }
    if !matches!(
        tool,
        "TodoWrite"
            | "TaskCreate"
            | "TaskUpdate"
            | "TaskList"
            | "TaskGet"
            | "ToolSearch"
            | "WebSearch"
            | "WebFetch"
    ) {
        return output.into();
    }
    let Ok(value) = serde_json::from_str::<Value>(output) else {
        return output.into();
    };
    if let Some(snapshot) = value.get("locusTodoSnapshot") {
        // Only the full synchronization snapshot is redundant. The native
        // operation result may contain warnings and must remain visible.
        let mut operation = value.clone();
        operation
            .as_object_mut()
            .unwrap()
            .remove("locusTodoSnapshot");
        return json!({"locusTodoSnapshot": {
            "version": snapshot["version"], "kind": snapshot["kind"], "items": snapshot["items"]
        }, "locusOutput": readable(&operation)})
        .to_string();
    }
    let result = value.get("result").unwrap_or(&value);
    if tool == "ToolSearch" {
        // Tool schemas are protocol data. Only names/descriptions belong in the
        // transcript, just as with Locus's own tool_search result.
        fn project(value: &Value, found: &mut Vec<String>) {
            if let Some(name) = value
                .get("tool_name")
                .or_else(|| value.get("name"))
                .and_then(Value::as_str)
            {
                let description = value["description"].as_str().unwrap_or_default();
                found.push(if description.is_empty() {
                    name.into()
                } else {
                    format!("{name}: {description}")
                });
            } else if let Some(values) = value.as_array() {
                for value in values {
                    project(value, found);
                }
            } else if let Some(values) = value.as_object() {
                for (key, value) in values {
                    if !matches!(key.as_str(), "input_schema" | "inputSchema" | "parameters") {
                        project(value, found);
                    }
                }
            }
        }
        let mut found = Vec::new();
        project(result, &mut found);
        if !found.is_empty() {
            return found.join("\n");
        }
    }
    readable(result)
}

/// A native Agent result is protocol data (status, usage, ids). The transcript
/// keeps the subagent's answer; other states keep their readable fields.
fn agent_result_text(output: &str) -> String {
    let Ok(value) = serde_json::from_str::<Value>(output) else {
        return output.into();
    };
    let result = value.get("result").unwrap_or(&value);
    if result["status"] == "completed" {
        if let Some(content) = result["content"].as_array() {
            let text = content.iter().filter(|block| block["type"] == "text")
                .filter_map(|block| block["text"].as_str()).collect::<Vec<_>>().join("\n");
            if !text.trim().is_empty() { return text; }
        }
    }
    readable(result)
}

/// The host has already saved the complete readable output and returned a
/// persisted-output reference. Apply that same boundary to the CLI payload,
/// rather than sending the unbounded pre-storage result through a side channel.
pub(crate) fn apply_persisted_output(tool: &str, value: &mut Value, output: &str) {
    match tool {
        "Grep" | "Glob" => {
            if let Some(files) = value["filenames"].as_array_mut() {
                let mut budget = 2_000usize;
                files.retain(|file| {
                    let size = text(file).chars().count() + 1;
                    if size > budget {
                        return false;
                    }
                    budget -= size;
                    true
                });
                value["numFiles"] = json!(files.len());
            }
            value["truncated"] = json!(true);
            if tool == "Grep" && value["mode"] != "files_with_matches" {
                value["content"] = json!(output);
                value["numLines"] = json!(output.lines().count());
            }
        }
        "TaskOutput" => value["task"]["output"] = json!(output),
        // A Locus subagent's report; the saved copy is its persisted output.
        "Agent" => value["content"] = json!([{"type": "text", "text": output}]),
        _ => {}
    }
}

/// Replaying a journaled operation must restore the same display result and
/// attachments, not serialize its protocol payload into the conversation.
/// Older journals have no locusOutput; project them without re-reading files.
pub(crate) fn replay(tool: &str, reply: &Value) -> super::ClaudeCodeToolResult {
    let value = &reply["result"];
    let mut images = Vec::new();
    let mut blocks = Vec::new();
    if tool.starts_with("mcp__locus__") {
        for block in value.as_array().into_iter().flatten() {
            match block["type"].as_str() {
                Some("text") => blocks.push(text(&block["text"]).to_string()),
                Some("image") => images.push(crate::session::models::ImageData {
                    data: text(&block["data"]).into(),
                    mime_type: text(&block["mimeType"]).into(),
                    capture: None,
                }),
                _ => {}
            }
        }
    } else if tool == "Read" && value["type"] == "image" {
        images.push(crate::session::models::ImageData {
            data: text(&value["file"]["base64"]).into(),
            mime_type: text(&value["file"]["type"]).into(),
            capture: None,
        });
    }
    let output = reply["locusOutput"]
        .as_str()
        .map(str::to_string)
        .unwrap_or_else(|| {
            if let Some(error) = reply["deny"].as_str() {
                error.into()
            } else if tool.starts_with("mcp__locus__") {
                blocks.join("\n\n")
            } else {
                render(tool, value)
            }
        });
    super::ClaudeCodeToolResult {
        output,
        is_error: reply["deny"].is_string(),
        images: (!images.is_empty()).then_some(images),
        context: reply["context"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(str::to_string)
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn summaries_do_not_repeat_protocol_file_bodies_or_question_options() {
        assert_eq!(
            render(
                "Write",
                &json!({"type":"update","filePath":"a.md","content":"NEW_SECRET","originalFile":"OLD_SECRET"})
            ),
            "Edited a.md"
        );
        assert_eq!(
            render(
                "AskUserQuestion",
                &json!({"questions":[{"options":["hidden"]}],"answers":{"Continue?":"Yes"}})
            ),
            "Continue?\nUser answered: Yes"
        );
    }
    #[test]
    fn session_tools_hide_sync_details_and_schemas_but_keep_results() {
        let projected = external("TaskUpdate", &json!({"result":{"success":true,"warning":"pending dependency"},"context":["native note"],"locusTodoSnapshot":{"version":1,"kind":"tasks","items":[{"content":"One","status":"completed"}],"tasks":[{"description":"PRIVATE_DETAIL"}]}}).to_string());
        assert!(projected.contains("pending dependency"));
        assert!(projected.contains("native note"));
        assert!(!projected.contains("PRIVATE_DETAIL"));
        assert_eq!(
            serde_json::from_str::<Value>(&projected).unwrap()["locusTodoSnapshot"]["items"][0]
                ["content"],
            "One"
        );
        assert_eq!(
            external(
                "ToolSearch",
                r#"{"result":{"tools":[{"name":"read","description":"Read files","input_schema":{"large":"schema"}}]}}"#
            ),
            "read: Read files"
        );
        assert_eq!(
            external("WebSearch", r#"{"result":{"content":"first\nsecond"}}"#),
            "content: first\nsecond"
        );
        assert_eq!(
            external("WebFetch", r#"{"result":{"result":"Title\nParagraph","url":"https://example.com/"}}"#),
            "result: Title\nParagraph\nurl: https://example.com/"
        );
    }
    #[test]
    fn replay_preserves_summaries_context_and_images_in_new_and_legacy_journals() {
        let reply = json!({"result":{"type":"create","filePath":"a.md","content":"BODY","originalFile":"OLD"},"locusOutput":"Created a.md\nKnowledge index: updated","context":["Created a.md\nKnowledge index: updated","Task completed"]});
        let result = replay("Write", &reply);
        assert_eq!(
            result.text_with_context(),
            "Created a.md\nKnowledge index: updated\n\nTask completed"
        );
        assert!(!result.output.contains("BODY"));
        let image = replay(
            "Read",
            &json!({"result":{"type":"image","file":{"base64":"aQ==","type":"image/png"}},"context":["Task completed"]}),
        );
        assert_eq!(image.images.unwrap()[0].data, "aQ==");
        assert!(!image.output.contains("aQ=="));
        assert_eq!(image.context, vec!["Task completed"]);
        let mcp = replay(
            "mcp__locus__probe",
            &json!({"result":[{"type":"text","text":"captured"},{"type":"image","data":"aQ==","mimeType":"image/png"}]}),
        );
        assert_eq!(mcp.output, "captured");
        assert_eq!(mcp.images.unwrap().len(), 1);
    }

    #[test]
    fn persisted_search_and_task_payloads_are_bounded_without_losing_counts() {
        let mut value = json!({"mode":"content","filenames":["x".repeat(30000)],"numFiles":1,"totalFiles":1,"content":"x".repeat(40000),"numMatches":55});
        apply_persisted_output(
            "Grep",
            &mut value,
            "<persisted-output>saved</persisted-output>",
        );
        assert!(value.to_string().len() < 1000);
        assert_eq!(value["totalFiles"], 1);
        assert_eq!(value["numMatches"], 55);
        assert_eq!(value["numFiles"], 0);
        let mut task = json!({"task":{"status":"failed","output":"x".repeat(40000)}});
        apply_persisted_output("TaskOutput", &mut task, "saved log");
        assert_eq!(task["task"]["status"], "failed");
        assert_eq!(task["task"]["output"], "saved log");
    }
}
