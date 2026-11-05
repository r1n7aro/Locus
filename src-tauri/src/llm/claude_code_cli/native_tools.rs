//! First native-tool adapters. These deliberately reject unsupported semantics
//! rather than silently performing a different operation or falling back to CLI.
use serde_json::{json, Value};

#[derive(Debug, Clone)]
pub struct NativeCall {
    pub id: String,
    pub tool: String,
    pub input: Value,
    pub locus_name: &'static str,
    pub locus_input: Value,
}

/// Model-facing duplicates of capabilities already provided by the pinned CLI
/// builtins. Keep this explicit: TaskOutput/TaskStop also dispatch through
/// `python`, but they do not replace the Python SDK tool itself. Native task
/// tools own their state and the Mod synchronizes it back to Locus.
pub(crate) fn native_equivalent_for_locus_tool(name: &str) -> Option<&'static str> {
    match name {
        "read" => Some("Read"),
        "write" => Some("Write"),
        "edit" => Some("Edit"),
        "bash" => Some("Bash"),
        "grep" => Some("Grep"),
        "list" => Some("Glob"),
        "ask_user_question" => Some("AskUserQuestion"),
        "todowrite" => Some("TaskCreate"),
        _ => None,
    }
}

pub fn adapt(id: &str, tool: &str, input: &Value) -> Result<NativeCall, String> {
    if id.is_empty() || !input.is_object() {
        return Err("Native tool call requires an ID and object input".into());
    }
    let string = |key: &str| {
        input
            .get(key)
            .and_then(Value::as_str)
            .ok_or_else(|| format!("{tool} requires {key}"))
    };
    let (locus_name, locus_input) = match tool {
        "Read" => {
            if input.get("pages").is_some() {
                return Err("PDF page reads are not adapted yet".into());
            }
            (
                "read",
                json!({"filePath":string("file_path")?,
                "offset":input.get("offset").cloned().unwrap_or(json!(1)),
                "limit":input.get("limit").cloned().unwrap_or(json!(2000))}),
            )
        }
        "Write" => (
            "write",
            json!({"filePath":string("file_path")?,"content":string("content")?}),
        ),
        "Edit" => {
            if string("old_string")?.is_empty() {
                return Err(
                    "Use Write to create a file; empty Edit replacements are not adapted yet"
                        .into(),
                );
            }
            (
                "edit",
                json!({"filePath":string("file_path")?,"edits":[{
                    "oldString":string("old_string")?,"newString":string("new_string")?,
                    "replaceAll":input.get("replace_all").and_then(Value::as_bool).unwrap_or(false)
                }]}),
            )
        }
        "Bash" => (
            "bash",
            json!({"command":string("command")?,
                "workdir":".","readonly":false,
                // Claude promises that a background command re-invokes the
                // model when it exits, also after the turn ends.
                "async":if input["run_in_background"].as_bool().unwrap_or(false){"notify"}else{"sync"},
                "description":input.get("description").cloned().unwrap_or(json!("")),
                "timeout":input.get("timeout").cloned().unwrap_or(json!(120000))}),
        ),
        "Glob" | "Grep" => {
            let mut args = input.clone();
            args["path"] = json!(input.get("path").and_then(Value::as_str).unwrap_or("."));
            let _ = string("pattern")?;
            (if tool == "Glob" { "list" } else { "grep" }, args)
        }
        "TaskOutput" | "TaskStop" => {
            let id = string("task_id")?;
            if id.is_empty() {
                return Err("Task ID is required".into());
            }
            let method = if tool == "TaskStop" {
                "cancel_task"
            } else {
                "get_task_status"
            };
            (
                "python",
                json!({"code":format!("await locus.{method}({})",json!(id.trim_start_matches("locus:"))),"readonly":true}),
            )
        }
        // The Mod routes messages for Claude's own agents to Claude; the
        // host receives only those for a Locus agent (`locus:<task id>`).
        "SendMessage" => {
            if !string("to")?.starts_with("locus:") {
                return Err("SendMessage runs in Locus only for a Locus agent (to: \"locus:<task id>\")".into());
            }
            let _ = string("message")?;
            ("SendMessage", input.clone())
        }
        // Only Locus-routed types reach the host; the Agent host maps the
        // native input onto its pinned delegation directory.
        "Agent" => {
            let _ = string("subagent_type")?;
            let _ = string("prompt")?;
            // Claude's parser may fill unset options; they select nothing.
            let mut fields = input.as_object().cloned().unwrap_or_default();
            fields.retain(|key, value| !value.is_null() && !(key == "run_in_background" && value == false));
            ("Agent", Value::Object(fields))
        }
        "AskUserQuestion" => {
            let questions = input["questions"]
                .as_array()
                .filter(|q| !q.is_empty())
                .ok_or("AskUserQuestion requires questions")?;
            let form = crate::user_question::QuestionForm::parse(input)?;
            let mut prompts = std::collections::HashSet::new();
            if form.questions.iter().any(|q| !prompts.insert(q.question.clone())) {
                return Err("Native AskUserQuestion requires distinct question text for its answer map".into());
            }
            ("ask_user_question", json!({"questions": questions}))
        }
        _ => {
            return Err(format!(
                "Native tool {tool} is not adapted for local execution"
            ))
        }
    };
    Ok(NativeCall {
        id: id.into(),
        tool: tool.into(),
        input: input.clone(),
        locus_name,
        locus_input,
    })
}

pub fn locus_name(name: &str) -> &str {
    match name {
        "Read" => "read",
        "Write" => "write",
        "Edit" => "edit",
        "Bash" => "bash",
        "Glob" => "list",
        "Grep" => "grep",
        "TaskOutput" | "TaskStop" => "python",
        "AskUserQuestion" => "ask_user_question",
        other => other,
    }
}

pub fn finish_result(
    call: &NativeCall,
    result: &super::ClaudeCodeToolResult,
    structured: Option<Value>,
) -> Value {
    let mut answer = finish_result_inner(call, result, structured);
    if answer.get("deny").is_some() {
        // Failed executions also carry queued notifications. Never turn an
        // error/interruption into a successful structured result.
        let mut message = answer["deny"].as_str().unwrap_or_default().to_string();
        for context in &result.context {
            if !context.is_empty() && !message.contains(context) {
                message.push_str("\n\n");
                message.push_str(context);
            }
        }
        answer["deny"] = json!(message);
        if result.output == crate::session::history::INTERRUPTED_TOOL_RESULT
            || result.output == super::native_journal::INTERRUPTED_DISPATCH_MESSAGE
        {
            answer["interrupted"] = json!(true);
        }
    } else {
        let mut context = answer["context"].as_array().cloned().unwrap_or_default();
        for text in &result.context {
            let item = json!(text);
            if !text.is_empty() && !context.contains(&item) { context.push(item); }
        }
        if !context.is_empty() { answer["context"] = json!(context); }
    }
    answer
}

fn finish_result_inner(
    call: &NativeCall,
    result: &super::ClaudeCodeToolResult,
    structured: Option<Value>,
) -> Value {
    if result.is_error || result.output == crate::session::history::INTERRUPTED_TOOL_RESULT {
        return json!({"callId":call.id,"deny":result.output});
    }
    if let Some(structured) = structured {
        let mut answer = json!({"callId":call.id,"result":structured});
        if call.tool == "Write" || result.output.starts_with("<persisted-output>") {
            answer["context"] = json!([result.output]);
        }
        return answer;
    }
    if call.tool == "Read" {
        if let Some(image) = result.images.as_ref().and_then(|images| images.first()) {
            let size = image.data.len() * 3 / 4
                - image.data.chars().rev().take_while(|c| *c == '=').count();
            return json!({"callId":call.id,"result":{"type":"image","file":{"base64":image.data,"type":image.mime_type,"originalSize":size}},"context":[result.output]});
        }
    }
    if call.tool == "Bash" && call.input["run_in_background"] == true {
        if let Some(id) = result
            .output
            .strip_prefix("Async task: id=")
            .and_then(|s| s.split_whitespace().next())
        {
            let id = serde_json::from_str::<String>(id).unwrap_or_else(|_| id.to_string());
            let code = format!("task = await locus.wait_task({}, timeout=30)\nprint(task.status, task.output)", json!(id));
            return json!({"callId":call.id,"result":{"stdout":"","stderr":"","interrupted":false,"backgroundTaskId":id},
                "context":[result.output, format!("This background task runs in Locus. Its result is delivered to you automatically when it finishes, even after this turn ends; do not poll it. Only if this turn cannot continue without the result, wait with mcp__locus__python, readonly=true, code: {code}. Use locus.get_task_status with the same task ID for a nonblocking check, or native TaskStop to stop it (a stopped task sends no result). Task APIs are scoped to the current Locus session.")]});
        }
        // Immediate commands can finish during the background startup handshake.
    }
    finish(call, &result.output, false)
}

/// Claude's schema parser can fill optional defaults before firing the Mod.
/// Compare the actual Locus operation, not JSON spelling (e.g. replace_all=false).
pub fn same_operation(id: &str, tool: &str, observed: &Value, forwarded: &Value) -> bool {
    // These adapters consume fields outside the canonical Locus arguments.
    if tool == "AskUserQuestion" {
        return match (
            crate::user_question::QuestionForm::parse(observed),
            crate::user_question::QuestionForm::parse(forwarded),
        ) {
            (Ok(left), Ok(right)) => serde_json::to_value(left.questions).ok() == serde_json::to_value(right.questions).ok(),
            _ => false,
        };
    }
    if matches!(tool, "TaskOutput" | "TaskStop") {
        return observed.as_object().is_some_and(|fields| {
            fields
                .iter()
                .all(|(key, value)| forwarded.get(key) == Some(value))
        }) && forwarded.as_object().is_some_and(|fields| {
            fields.iter().all(|(key, value)| {
                observed.get(key) == Some(value)
                    || (tool == "TaskOutput"
                        && ((key == "block" && value == true)
                            || (key == "timeout" && value == 30000)))
            })
        });
    }
    match (adapt(id, tool, observed), adapt(id, tool, forwarded)) {
        (Ok(left), Ok(right)) => left.locus_input == right.locus_input,
        _ => observed == forwarded,
    }
}

/// Convert only the Locus result that passed its permission/execution pipeline.
/// Never re-read a path here: that could bypass scope checks or race the tool.
pub fn finish(call: &NativeCall, output: &str, is_error: bool) -> Value {
    let deny = |reason: &str| json!({"callId":call.id,"deny":reason});
    if is_error {
        return deny(output);
    }
    let mut context = Vec::new();
    let result = match call.tool.as_str() {
        "Bash" => json!({"stdout":output,"stderr":"","interrupted":false,"isImage":false}),
        "Write" => json!({"type":"create","filePath":call.input["file_path"],
            "content":call.input["content"],"structuredPatch":[],"originalFile":null}),
        "Edit" => json!({"filePath":call.input["file_path"],"oldString":call.input["old_string"],
            "newString":call.input["new_string"],"originalFile":null,"structuredPatch":[],
            "userModified":false,"replaceAll":call.input.get("replace_all").and_then(Value::as_bool).unwrap_or(false)}),
        "Read" => {
            // Locus text reads frame each physical line as <number>\t<content>.
            // Only strip a sequential prefix, preserving tabs/numbers in content.
            // Skill activation/runtime notes and async notifications follow the
            // frame. They are model context, not file lines; retain them separately.
            let Some((body, trailing)) = output
                .strip_prefix("<content>\n")
                .and_then(|s| s.split_once("\n</content>"))
            else {
                return deny("Read output is not a supported text frame; use a supported file type or a smaller read range");
            };
            if !trailing.trim().is_empty() {
                context.push(trailing.trim().to_string());
            }
            let Some((numbered_lines, summary)) = body.rsplit_once("\n\n") else {
                return deny("Read output was capped or unrecognized; retry with a smaller limit");
            };
            let start = call
                .input
                .get("offset")
                .and_then(Value::as_u64)
                .unwrap_or(1)
                .max(1);
            let mut lines = Vec::new();
            for line in numbered_lines.lines() {
                let prefix = format!("{}\t", start + lines.len() as u64);
                if let Some(content) = line.strip_prefix(&prefix) {
                    lines.push(content);
                } else {
                    return deny("Read output has inconsistent line numbers; retry with a smaller limit");
                }
            }
            let total = summary
                .strip_prefix("(End of file — ")
                .and_then(|s| s.strip_suffix(" lines total)"))
                .and_then(|s| s.parse::<u64>().ok())
                .or_else(|| {
                    summary
                        .split_once(" of ")
                        .and_then(|(_, s)| s.split('.').next())
                        .and_then(|s| s.parse::<u64>().ok())
                });
            let Some(total) = total else {
                return deny("Read output was capped or unrecognized; retry with a smaller limit");
            };
            // Byte-capped reads are valid partial results when the real total is
            // known. Keep the cap and continuation hint outside the file content.
            if summary.starts_with("(Output capped at ") {
                context.push(summary.to_string());
            }
            json!({"type":"text","file":{"filePath":call.input["file_path"],
                "content":lines.join("\n"),"numLines":lines.len(),"startLine":start,"totalLines":total}})
        }
        _ => return deny("Unsupported native result"),
    };
    let mut answer = json!({"callId":call.id,"result":result});
    // Writes may include Unity/import or knowledge metadata which the native
    // result schema cannot carry. Read carries only file lines in its result;
    // its supplemental context must survive outside that schema.
    if matches!(call.tool.as_str(), "Write" | "Edit") {
        context.push(output.to_string());
    }
    if !context.is_empty() {
        answer["context"] = json!(context);
    }
    answer
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_equivalents_are_declared_builtins_and_do_not_hide_sdk_or_specialized_tools() {
        for name in ["read", "write", "edit", "bash", "grep", "list", "ask_user_question", "todowrite"] {
            let native = native_equivalent_for_locus_tool(name).unwrap();
            assert!(super::super::NATIVE_BUILTIN_TOOLS.split(',').any(|declared| declared == native), "missing builtin for {name}");
        }
        for name in ["python", "web_fetch", "unity_execute", "skill_list", "create_skill_package", "subagent", "my_package_edit"] {
            assert_eq!(native_equivalent_for_locus_tool(name), None, "{name} is not fully covered by a native tool");
        }
    }
    #[test]
    fn interrupted_locus_output_never_becomes_a_successful_native_write() {
        let call = adapt("write-cancelled","Write",&json!({"file_path":"a","content":"new content"})).unwrap();
        let output = super::super::ClaudeCodeToolResult {
            output:crate::session::history::INTERRUPTED_TOOL_RESULT.into(),is_error:false,images:None,context:Vec::new(),
        };
        let reply = finish_result(&call,&output,Some(json!({"type":"create"})));
        assert_eq!(reply["deny"],crate::session::history::INTERRUPTED_TOOL_RESULT);
        assert!(reply.get("result").is_none());
    }

    #[test]
    fn background_result_unquotes_locus_ids_and_accepts_local_task_control(){
        let call=adapt("b1","Bash",&json!({"command":"sleep 1","run_in_background":true})).unwrap();
        // Background commands deliver their result and wake an idle session.
        assert_eq!(call.locus_input["async"],"notify");
        assert_eq!(adapt("b0","Bash",&json!({"command":"echo ok"})).unwrap().locus_input["async"],"sync");
        let output=super::super::ClaudeCodeToolResult{output:"Async task: id=\"t4\" status=queued notify=true\nResult sent automatically.".into(),is_error:false,images:None,context:Vec::new()};
        let result = finish_result(&call,&output,None);
        assert_eq!(result["result"]["backgroundTaskId"],"t4");
        let context = result["context"][1].as_str().unwrap();
        assert!(context.contains("delivered to you automatically"));
        assert!(context.contains("mcp__locus__python"));
        assert!(context.contains("readonly=true"));
        assert!(context.contains("await locus.wait_task(\"t4\", timeout=30)"));
        assert!(context.contains("print(task.status, task.output)"));
        assert!(adapt("s1","TaskStop",&json!({"task_id":"t4"})).is_ok());
        assert!(adapt("r1","TaskOutput",&json!({"task_id":"t4"})).is_ok());
    }
    #[test]
    fn only_messages_for_locus_agents_reach_the_host() {
        let input = json!({"to":"locus:t3","message":"Also check the tests","summary":"tests"});
        let call = adapt("m1", "SendMessage", &input).unwrap();
        assert_eq!((call.locus_name, &call.locus_input), ("SendMessage", &input));
        // Claude's own agents (and the main agent) are never Locus's.
        assert!(adapt("m2", "SendMessage", &json!({"to":"a1b2","message":"hi"})).is_err());
        assert!(adapt("m3", "SendMessage", &json!({"to":"locus:t3"})).is_err());
    }
    #[test]
    fn edits_use_locus_atomic_batch_schema() {
        let call = adapt(
            "t1",
            "Edit",
            &json!({"file_path":"a.txt","old_string":"old","new_string":"new","replace_all":true}),
        )
        .unwrap();
        assert_eq!(call.locus_name, "edit");
        assert_eq!(call.locus_input["edits"][0]["replaceAll"], true);
        assert_eq!(
            finish(&call, "permission denied", true)["deny"],
            "permission denied"
        );
    }
    #[test]
    fn text_read_preserves_numbered_lines_and_tabs() {
        let call = adapt("t2", "Read", &json!({"file_path":"a.txt","offset":2})).unwrap();
        let answer = finish(
            &call,
            "<content>\n2\t1\tvalue\n3\t\n\n(End of file — 3 lines total)\n</content>",
            false,
        );
        assert_eq!(answer["result"]["file"]["content"], "1\tvalue\n");
        assert_eq!(answer["result"]["file"]["totalLines"], 3);
    }
    #[tokio::test]
    async fn text_read_preserves_skill_runtime_and_notification_context() {
        let root = tempfile::tempdir().unwrap();
        let text = "---\ntools: [bash, read]\n---\n\n# Workflow\n\nLiteral </content>\n1\tkeep this tab\n";
        std::fs::write(root.path().join("SKILL.md"), text).unwrap();
        let call = adapt("skill-read", "Read", &json!({"file_path":"SKILL.md"})).unwrap();
        let registry = crate::tool::ToolRegistry::with_builtins();
        let mut result: super::super::ClaudeCodeToolResult = registry.execute_with_context(
            call.locus_name,
            &call.locus_input,
            crate::tool::ToolExecutionContext {
                working_dir: Some(root.path().to_string_lossy().into_owned()),
                ..Default::default()
            },
        ).await.into();
        assert!(!result.is_error, "{}", result.output);
        let context = "Locus Skill runtime: Unity C# scripts are not ready.\nfixture error\n\nLoaded Skill document tools for the next step: bash, read\n\n<system-reminder>Background task finished.</system-reminder>";
        result.output.push_str(&format!("\n\n{context}"));
        let answer = finish_result(&call, &result, None);
        assert!(answer.get("deny").is_none(), "{answer}");
        assert_eq!(answer["result"]["file"]["content"], text.trim_end_matches('\n'));
        assert_eq!(answer["result"]["file"]["numLines"], 8);
        assert_eq!(answer["result"]["file"]["totalLines"], 8);
        assert_eq!(answer["context"], json!([context]));
    }

    #[test]
    fn text_read_accepts_empty_files_partial_ranges_and_trailing_whitespace() {
        let empty = adapt("empty", "Read", &json!({"file_path":"empty.txt"})).unwrap();
        let answer = finish(&empty, "<content>\n\n\n(End of file — 0 lines total)\n</content>\n", false);
        assert_eq!(answer["result"]["file"]["content"], "");
        assert_eq!(answer["result"]["file"]["numLines"], 0);
        assert_eq!(answer["result"]["file"]["totalLines"], 0);
        assert!(answer.get("context").is_none());

        let partial = adapt("partial", "Read", &json!({"file_path":"SKILL.md","offset":3,"limit":2})).unwrap();
        let answer = finish(&partial, "<content>\n3\t</content>\n4\t\n\n(Showing lines 3-4 of 9. Use offset=5 to continue.)\n</content>\n\nLoaded Skill document tools for the next step: read", false);
        assert_eq!(answer["result"]["file"]["content"], "</content>\n");
        assert_eq!(answer["result"]["file"]["startLine"], 3);
        assert_eq!(answer["result"]["file"]["numLines"], 2);
        assert_eq!(answer["result"]["file"]["totalLines"], 9);
        assert_eq!(answer["context"], json!(["Loaded Skill document tools for the next step: read"]));
    }

    #[test]
    fn text_read_accepts_capped_frames_and_preserves_context() {
        let call = adapt("capped-read", "Read", &json!({"file_path":"file.txt","offset":7})).unwrap();
        let summary = "(Output capped at 50KB. Showing lines 7-8 of 100. Use offset=9 to continue.)";
        let result = super::super::ClaudeCodeToolResult {
            output: format!("<content>\n7\t段落\n8\t1\tkeep this tab\n\n{summary}\n</content>\n\nSkill runtime ready"),
            is_error: false,
            images: None,
            context: vec!["Background task completed".into()],
        };
        let answer = finish_result(&call, &result, None);
        assert!(answer.get("deny").is_none(), "{answer}");
        assert_eq!(answer["result"]["file"], json!({
            "filePath":"file.txt", "content":"段落\n1\tkeep this tab",
            "numLines":2, "startLine":7, "totalLines":100,
        }));
        assert_eq!(answer["context"], json!([
            "Skill runtime ready", summary, "Background task completed",
        ]));
    }

    #[tokio::test]
    async fn text_read_byte_cap_round_trips_builtin_output_and_continuation() {
        let root = tempfile::tempdir().unwrap();
        let expected: Vec<String> = (1..=160)
            .map(|line| format!("{line}\t{}", "段落".repeat(150)))
            .collect();
        std::fs::write(root.path().join("capped.md"), expected.join("\n")).unwrap();
        let registry = crate::tool::ToolRegistry::with_builtins();
        let mut collected = Vec::new();
        let mut offset = 1;
        let mut capped_reads = 0;
        while offset <= expected.len() {
            let call = adapt("capped-read", "Read", &json!({
                "file_path":"capped.md", "offset":offset, "limit":2000,
            })).unwrap();
            let result: super::super::ClaudeCodeToolResult = registry.execute_with_context(
                call.locus_name,
                &call.locus_input,
                crate::tool::ToolExecutionContext {
                    working_dir: Some(root.path().to_string_lossy().into_owned()),
                    ..Default::default()
                },
            ).await.into();
            assert!(!result.is_error, "{}", result.output);
            let answer = finish_result(&call, &result, None);
            assert!(answer.get("deny").is_none(), "{answer}");
            let file = &answer["result"]["file"];
            assert_eq!(file["startLine"], json!(offset));
            assert_eq!(file["totalLines"], json!(expected.len()));
            let count = file["numLines"].as_u64().unwrap() as usize;
            assert!(count > 0 && count <= expected.len() - offset + 1);
            assert_eq!(file["content"], expected[offset - 1..offset - 1 + count].join("\n"));
            let next = offset + count;
            if next <= expected.len() {
                let summary = format!(
                    "(Output capped at 50KB. Showing lines {}-{} of {}. Use offset={} to continue.)",
                    offset, next - 1, expected.len(), next,
                );
                assert!(result.output.contains(&summary));
                assert_eq!(answer["context"], json!([summary]));
                capped_reads += 1;
            } else {
                assert!(result.output.contains("(End of file — 160 lines total)"));
                assert!(answer.get("context").is_none());
            }
            collected.extend(file["content"].as_str().unwrap().lines().map(str::to_string));
            offset = next;
        }
        assert!(capped_reads >= 2, "fixture must exercise capped reads at noninitial offsets");
        assert_eq!(collected, expected);
    }

    #[test]
    fn text_read_rejects_unknown_totals_or_malformed_frames_without_silent_data_loss() {
        let call = adapt("bad-read", "Read", &json!({"file_path":"file.txt"})).unwrap();
        for output in [
            "<content>\n1\tline\n\n(Output capped at 50KB. Showing lines 1-1. Use offset=2 to continue.)\n</content>",
            "<content>\n1\tline\n\n(End of file — 1 lines total)",
            "<content>\n1\tfirst\n3\tmissing second line\n\n(End of file — 3 lines total)\n</content>\n\nextra context",
            "<persisted-output>Read the saved full output</persisted-output>",
        ] {
            let answer = finish(&call, output, false);
            assert!(answer["deny"].is_string(), "{answer}");
            assert!(answer.get("result").is_none(), "{answer}");
        }
    }

    #[test]
    fn native_read_keeps_images_and_execution_errors_out_of_text_parsing() {
        let call = adapt("image", "Read", &json!({"file_path":"image.png"})).unwrap();
        let mut result = super::super::ClaudeCodeToolResult {
            output: "Image file read. Image attached.".into(),
            context: Vec::new(),
            is_error: false,
            images: Some(vec![crate::session::models::ImageData {
                data: "aQ==".into(), mime_type: "image/png".into(), capture: None,
            }]),
        };
        let answer = finish_result(&call, &result, None);
        assert_eq!(answer["result"]["type"], "image");
        assert_eq!(answer["result"]["file"]["originalSize"], 1);
        result.is_error = true;
        result.output = "permission denied".into();
        let answer = finish_result(&call, &result, None);
        assert_eq!(answer["deny"], "permission denied");
        assert!(answer.get("result").is_none());
    }

    #[test]
    fn every_native_result_branch_preserves_supplemental_context() {
        let reminder = "<system-reminder>Background task completed.</system-reminder>";
        let cases = [
            ("Write", json!({"file_path":"a.md","content":"body"}), Some(json!({"type":"create","content":"body"})), "Created a.md\nKnowledge index: updated"),
            ("Glob", json!({"pattern":"*"}), Some(json!({"filenames":["a.md"],"numFiles":1})), "Found 1 files"),
            ("Grep", json!({"pattern":"match"}), Some(json!({"mode":"content","content":"match"})), "Found 1 matches"),
            ("TaskOutput", json!({"task_id":"t1"}), Some(json!({"task":{"output":"done"}})), "Task t1: completed"),
            ("TaskStop", json!({"task_id":"t1"}), Some(json!({"message":"Cancellation requested"})), "Cancellation requested"),
            ("AskUserQuestion", json!({"questions":[{"question":"Continue?","options":[]}]}), Some(json!({"answers":{"Continue?":"Yes"}})), "User answered: Yes"),
            ("Edit", json!({"file_path":"a.md","old_string":"old","new_string":"new"}), None, "Edited a.md\nKnowledge index: immediate update failed; filesystem watcher will retry"),
            ("Read", json!({"file_path":"a.txt"}), None, "<content>\n1\tline\n\n(End of file — 1 lines total)\n</content>\n\nSkill runtime ready"),
            ("Bash", json!({"command":"echo ok"}), None, "Exit code: 0\nok"),
            ("Bash", json!({"command":"echo ok","run_in_background":true}), None, "Async task: id=\"t1\" status=queued notify=true"),
        ];
        for (tool, input, structured, output) in cases {
            let call = adapt("notification", tool, &input).unwrap();
            let result = super::super::ClaudeCodeToolResult {
                output: output.into(), is_error: false, images: None, context: vec![reminder.into()],
            };
            let answer = finish_result(&call, &result, structured);
            assert!(answer.get("deny").is_none(), "{tool}: {answer}");
            let context = answer["context"].as_array().unwrap();
            assert_eq!(context.iter().filter(|value| value.as_str() == Some(reminder)).count(), 1, "{tool}: {answer}");
            if matches!(tool, "Write" | "Edit") { assert!(context.contains(&json!(output))); }
            if tool == "Read" { assert!(context.contains(&json!("Skill runtime ready"))); }
        }
        let call = adapt("image", "Read", &json!({"file_path":"a.png"})).unwrap();
        let result = super::super::ClaudeCodeToolResult {
            output: "Image attached; runtime note".into(), is_error: false,
            images: Some(vec![crate::session::models::ImageData { data:"aQ==".into(), mime_type:"image/png".into(), capture:None }]),
            context: vec![reminder.into()],
        };
        let answer = finish_result(&call, &result, None);
        assert_eq!(answer["result"]["file"]["base64"], "aQ==");
        assert!(answer["context"].as_array().unwrap().contains(&json!(reminder)));
        assert!(answer["context"].as_array().unwrap().contains(&json!(result.output)));
    }

    #[test]
    fn native_adapter_errors_keep_the_reason_and_notifications() {
        let call = adapt("bad-read", "Read", &json!({"file_path":"a.txt"})).unwrap();
        let mut result: super::super::ClaudeCodeToolResult = crate::tool::ToolResult { output:"invalid frame".into(), is_error:false }.into();
        result.context.push("Task completed".into());
        let answer = finish_result(&call, &result, None);
        assert!(answer["deny"].as_str().unwrap().contains("not a supported text frame"));
        assert!(answer["deny"].as_str().unwrap().contains("Task completed"));
        result.is_error = true;
        result.output = "Permission denied".into();
        let answer = finish_result(&call, &result, Some(json!({"must":"not succeed"})));
        assert_eq!(answer["deny"], "Permission denied\n\nTask completed");
        assert!(answer.get("result").is_none());
        result.output = crate::session::history::INTERRUPTED_TOOL_RESULT.into();
        let answer = finish_result(&call, &result, None);
        assert_eq!(answer["interrupted"], true);
        assert!(answer["deny"].as_str().unwrap().contains("Task completed"));
    }

    #[test]
    fn persisted_structured_results_keep_the_full_output_reference() {
        let call = adapt("large", "Glob", &json!({"pattern":"*"})).unwrap();
        let result: super::super::ClaudeCodeToolResult = crate::tool::ToolResult {
            output:"<persisted-output>Full output saved to: /tmp/result.txt</persisted-output>".into(), is_error:false,
        }.into();
        let answer = finish_result(&call, &result, Some(json!({"filenames":[],"numFiles":0,"truncated":true})));
        assert_eq!(answer["context"], json!([result.output]));
    }

    #[test]
    fn questions_are_forwarded_as_one_form_with_native_defaults_and_multi_select() {
        let input = json!({"questions":[
            {"question":"Targets?","header":"Targets","multiSelect":true,"options":[{"label":"A","description":"First"},{"label":"B","description":"Second"}]},
            {"question":"Why?","kind":"text"}
        ]});
        let call = adapt("form", "AskUserQuestion", &input).unwrap();
        assert_eq!(call.locus_input["questions"], input["questions"]);
        assert_eq!(call.locus_input["questions"].as_array().unwrap().len(), 2);
        let mut defaults = input.clone();
        defaults["questions"][1]["multiSelect"] = json!(false);
        assert!(same_operation("form", "AskUserQuestion", &input, &defaults));
        defaults["questions"][0]["multiSelect"] = json!(false);
        assert!(!same_operation("form", "AskUserQuestion", &input, &defaults));
        assert!(adapt("duplicate", "AskUserQuestion", &json!({"questions":[{"question":"Same?"},{"question":"Same?"}]})).is_err());
    }

    #[test]
    fn unsupported_semantics_never_reach_the_host() {
        assert!(adapt(
            "t",
            "Bash",
            &json!({"command":"echo ok","run_in_background":true})
        )
        .is_ok());
        assert!(adapt("t", "Read", &json!({"file_path":"a.pdf","pages":"1"})).is_err());
        assert!(adapt("t", "Unknown", &json!({})).is_err());
    }

    #[test]
    fn correlation_accepts_schema_defaults_but_not_changed_operations() {
        let original = json!({"file_path":"a","old_string":"a","new_string":"b"});
        let mut forwarded = original.clone();
        forwarded["replace_all"] = json!(false);
        assert!(same_operation("id", "Edit", &original, &forwarded));
        forwarded["new_string"] = json!("other");
        assert!(!same_operation("id", "Edit", &original, &forwarded));
    }
}
