//! Freeze an acknowledged native transcript, then let Claude --fork-session
//! create the new native identity. Never replay Locus history as model requests.
use super::*;
use super::connection::NativeConnection;
use crate::session::store::SessionStore;

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub(super) struct ForkState {
    pub version: u32,
    pub source_id: String,
    pub new_id: String,
}

pub async fn prepare(store: &SessionStore, source: &str, target: &str) -> Result<(), String> {
    let Some(source_cli) = store.latest_cli_session_id(source, "claude_code")? else {
        return Ok(());
    };
    let metadata = store.get_response_request_metadata(target)?;
    let messages = store.get_messages(target)?;
    let checkpoint=messages.last().and_then(|message|metadata.get(&message.id).and_then(|m|m.get("claude_checkpoint")))
        .ok_or("This Claude conversation has no acknowledged native fork boundary; start a new turn before forking")?;
    // The compact hook settles before core installs its replacement rows.
    // Resolve the following summary when freezing that receipt.
    let after_compact = messages
        .last()
        .and_then(|message| metadata.get(&message.id))
        .is_some_and(|value| {
            value["claude_compaction"]["checkpoint"]["uuid"] == checkpoint["uuid"]
        });
    if checkpoint["version"] != 1 {
        return Err("Unsupported Claude checkpoint version".into());
    }
    let native_id = checkpoint["sessionId"].as_str().unwrap_or(&source_cli);
    let at = checkpoint["uuid"]
        .as_str()
        .ok_or("Missing Claude checkpoint UUID")?;
    for id in [native_id, at] {
        uuid::Uuid::parse_str(id).map_err(|_| "Invalid Claude fork boundary")?;
    }
    let remote = recovery::recorded_remote(store,source)?;
    let remote = ssh::native_session_connection(source, true, Some(remote))?;
    ssh::native_session_connection(target, false, Some(remote.clone()))?;
    if let Some(remote)=&remote {store.set_claude_active_remote(target,remote)?;}
    let mut state = ForkState {
        version: 1,
        source_id: native_id.into(),
        new_id: uuid::Uuid::new_v4().to_string(),
    };
    let todos=messages.iter().rev().find_map(|message|metadata.get(&message.id).and_then(|value|value.get("claude_todos")));
    if let Some(connection) = &remote {
        match connection {
            NativeConnection::Ssh(config) => {
                let (_transport,_)=ssh::Transport::start(config,json!({"mode":"fork","authorizedSetup":true,
                    "sourceKey":ssh::session_key(source,config),"sessionKey":ssh::session_key(target,config),
                    "sourceId":state.source_id,"newId":state.new_id,"at":at,"afterCompact":after_compact,"todos":todos}),None).await?;
            }
            NativeConnection::Server{binding_id} => {
                #[cfg(feature="server-client")]
                {
                    let reply=super::server_transport::operation(binding_id,target,"fork",json!({"sourceSessionId":source,"sourceId":state.source_id,"at":at,"afterCompact":after_compact,"todos":todos})).await?;
                    state.new_id=reply["nativeId"].as_str().ok_or("Server did not return the forked conversation identity")?.into();
                    uuid::Uuid::parse_str(&state.new_id).map_err(|_|"Invalid forked conversation identity")?;
                }
                #[cfg(not(feature="server-client"))]
                {let _=binding_id;return Err("This conversation requires the internal Locus Server client".into());}
            }
        }
    } else {
        let source = ssh::profile_root(source)?;
        let target = ssh::profile_root(target)?;
        for attempt in 0..=20 {
            match prepare_local(&source, &target, &state, at, after_compact) {
                Err(error) if error.starts_with("Claude has not flushed") && attempt < 20 => {
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
                result => { result?; break; }
            }
        }
        super::rewind::restore_tasks(&target,&state.new_id,todos)?;
    }
    super::tool_catalog::copy_for_fork(source, target)?;
    if let Some(remote)=&remote {store.bind_claude_native_session(target,remote,&state.new_id)?;}
    // Existing extensible provider metadata carries this v1 checkpoint and is
    // already copied/exported by SessionStore. No session-table shape changes.
    for message in messages {
        store.set_message_cli_session_id(target, &message.id, "claude_code", &state.new_id)?;
        if let Some(mut value) = metadata.get(&message.id).cloned() {
            if let Some(point) = value.get_mut("claude_checkpoint") {
                point["sessionId"] = json!(state.new_id);
            }
            store.set_message_response_request_metadata(target, &message.id, &value)?;
        }
    }
    Ok(())
}

pub(super) fn find_transcript(root: &Path, id: &str) -> Result<PathBuf, String> {
    let mut pending = vec![root.join("profile/projects")];
    while let Some(path) = pending.pop() {
        for entry in std::fs::read_dir(path).map_err(|e| e.to_string())? {
            let entry = entry.map_err(|e| e.to_string())?;
            let kind = entry.file_type().map_err(|e| e.to_string())?;
            if kind.is_dir() {
                pending.push(entry.path());
            } else if kind.is_file()
                && entry.file_name() == std::ffi::OsString::from(format!("{id}.jsonl"))
            {
                return Ok(entry.path());
            }
        }
    }
    Err("Claude native transcript is unavailable; no fork was created".into())
}
fn prepare_local(
    source: &Path,
    target: &Path,
    state: &ForkState,
    at: &str,
    after_compact: bool,
) -> Result<(), String> {
    if source == target {
        return Err("Cannot fork a Claude profile onto itself".into());
    }
    let path = find_transcript(source, &state.source_id).or_else(|original| {
        let path=source.join("fork.json");
        if !path.is_file() {return Err(original);}
        let prepared:ForkState=serde_json::from_slice(&std::fs::read(path).map_err(|e|e.to_string())?).map_err(|e|e.to_string())?;
        if prepared.version!=1 || prepared.new_id!=state.source_id || prepared.source_id==prepared.new_id {
            return Err("Invalid prepared Claude fork identity".into());
        }
        find_transcript(source,&prepared.source_id)
    })?;
    let input = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
    let snapshot = snapshot(&input, at, after_compact)?;
    let cwd = target.join("terminal").to_string_lossy().into_owned();
    let slug = regex::Regex::new("[^a-zA-Z0-9]")
        .unwrap()
        .replace_all(&cwd, "-");
    let project = target.join("profile/projects").join(slug.as_ref());
    std::fs::create_dir_all(&project).map_err(|e| e.to_string())?;
    std::fs::write(project.join(format!("{}.jsonl", state.source_id)), snapshot)
        .map_err(|e| e.to_string())?;
    std::fs::write(
        target.join("fork.json"),
        serde_json::to_vec(state).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())
}
pub(super) fn snapshot(input: &str, at: &str, after_compact: bool) -> Result<String, String> {
    let mut output = String::new();
    let mut found = false;
    let mut anchored = false;
    for line in input.lines() {
        let value =
            match serde_json::from_str::<serde_json::Value>(line) {
                Ok(value) => value,
                Err(_) if found => break,
                Err(_) => return Err(
                    "Claude transcript contains an incomplete row before the requested boundary"
                        .into(),
                ),
            };
        // Native @file/image attachments are appended immediately AFTER their
        // user row. Preserve them when forking from that user message, but
        // never include the following assistant response or another prompt.
        // After a compaction summary Claude records the /compact command echo
        // and then the post-compaction attachments, including the Locus
        // carry-forward (SessionStart context). They belong to the summary.
        if found
            && (value["attachment"]["type"] == "queued_command"
                || (value["uuid"].is_string()
                    && value["type"] != "attachment"
                    && !(value["type"] == "user" && value["isMeta"] == true)
                    && !(after_compact && super::recovery::hidden_native_user(&value))))
        {
            break;
        }
        output.push_str(line);
        output.push('\n');
        if value["uuid"] == at {
            anchored = true;
        }
        if anchored && (!after_compact || value["isCompactSummary"] == true) {
            found = true;
        }
    }
    if found {
        Ok(output)
    } else {
        Err(
            "Claude has not flushed the requested fork boundary; retry after the turn completes"
                .into(),
        )
    }
}
pub(super) fn resume_args(root: &Path, resume: Option<&str>) -> Result<Vec<String>, String> {
    let Some(resume) = resume else {
        return Ok(Vec::new());
    };
    let path = root.join("fork.json");
    if path.is_file() {
        let state: ForkState =
            serde_json::from_slice(&std::fs::read(path).map_err(|e| e.to_string())?)
                .map_err(|e| e.to_string())?;
        if state.version != 1 {
            return Err("Unsupported Claude fork state".into());
        }
        if state.new_id == resume && find_transcript(root, resume).is_err() {
            return Ok(vec![
                "--resume".into(),
                state.source_id,
                "--fork-session".into(),
                "--session-id".into(),
                state.new_id,
            ]);
        }
    }
    Ok(vec!["--resume".into(), resume.into()])
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_fork_snapshot_never_includes_later_turns() {
        let input =
            "{\"uuid\":\"first\"}\n{\"uuid\":\"boundary\"}\n{\"uuid\":\"future\"}\n{partial";
        let copied = snapshot(input, "boundary", false).unwrap();
        assert!(
            copied.contains("first") && copied.contains("boundary") && !copied.contains("future")
        );
        assert!(snapshot(input, "missing", false).is_err());
        let attached="{\"type\":\"user\",\"uuid\":\"user\"}\n{\"type\":\"attachment\",\"uuid\":\"image\"}\n{\"type\":\"user\",\"isMeta\":true,\"uuid\":\"source\"}\n{\"type\":\"assistant\",\"uuid\":\"reply\"}";
        let copied = snapshot(attached, "user", false).unwrap();
        assert!(copied.contains("image") && copied.contains("source") && !copied.contains("reply"));
        let queued="{\"type\":\"user\",\"uuid\":\"user\"}\n{\"type\":\"attachment\",\"uuid\":\"image\"}\n{\"type\":\"attachment\",\"uuid\":\"future\",\"attachment\":{\"type\":\"queued_command\"}}";
        let copied=snapshot(queued,"user",false).unwrap();
        assert!(copied.contains("image") && !copied.contains("future"));
        let compacted="{\"uuid\":\"command\"}\n{\"uuid\":\"boundary\",\"type\":\"system\"}\n{\"uuid\":\"summary\",\"type\":\"user\",\"isCompactSummary\":true}\n{\"uuid\":\"future\",\"type\":\"user\"}";
        let copied = snapshot(compacted, "command", true).unwrap();
        assert!(copied.contains("summary") && !copied.contains("future"));
        let echoed = [
            r#"{"uuid":"command"}"#,
            r#"{"uuid":"boundary","type":"system"}"#,
            r#"{"uuid":"summary","type":"user","isCompactSummary":true,"message":{"content":"Summary"}}"#,
            r#"{"uuid":"caveat","type":"user","isMeta":true,"message":{"content":"<local-command-caveat>x</local-command-caveat>"}}"#,
            r#"{"uuid":"echo","type":"user","message":{"content":"<command-name>/compact</command-name>"}}"#,
            r#"{"uuid":"stdout","type":"user","message":{"content":"<local-command-stdout>Compacted</local-command-stdout>"}}"#,
            r#"{"uuid":"carry","type":"attachment","attachment":{"type":"hook_additional_context","content":["CARRY"]}}"#,
            r#"{"uuid":"future","type":"user","message":{"content":"next request"}}"#,
        ].join("\n");
        let copied = snapshot(&echoed, "command", true).unwrap();
        assert!(copied.contains("\"carry\"") && copied.contains("\"stdout\"") && !copied.contains("future"));
        // Without a compaction, a command echo still ends the copied turn.
        assert!(!snapshot(&echoed, "caveat", false).unwrap().contains("\"carry\""));
        assert!(snapshot("{\"uuid\":\"command\"}", "command", true).is_err());
    }
}
