//! Prepare a new native transcript first; switch the DB only after it exists.
use super::*;
use super::connection::NativeConnection;
use crate::session::{models::MessageRole, store::SessionStore};

pub enum Target<'a> {
    After(&'a str),
    Before(&'a str),
    LatestTurn,
}
pub struct PreparedRewind {
    id: String,
    expected: Vec<String>,
    keep: usize,
    native_id: Option<String>,
    _guard: tokio::sync::OwnedRwLockWriteGuard<()>,
}
impl PreparedRewind {
    pub async fn commit(self, store: &SessionStore) -> Result<u64, String> {
        if self.keep == self.expected.len() {
            return Ok(0);
        }
        let deleted = store.commit_claude_rewind(
            &self.id,
            &self.expected,
            self.keep,
            self.native_id.as_deref(),
        )?;
        clear_session_id(&self.id).await;
        Ok(deleted)
    }
}
pub async fn rewind(
    store: &SessionStore,
    id: &str,
    boundary: Option<&str>,
) -> Result<Option<u64>, String> {
    match prepare(
        store,
        id,
        boundary.map(Target::After).unwrap_or(Target::LatestTurn),
    )
    .await?
    {
        Some(prepared) => prepared.commit(store).await.map(Some),
        None => Ok(None),
    }
}
pub async fn prepare(
    store: &SessionStore,
    id: &str,
    target: Target<'_>,
) -> Result<Option<PreparedRewind>, String> {
    let Some(source_id) = store.latest_cli_session_id(id, "claude_code")? else {
        return Ok(None);
    };
    let guard = tui::history_gate(id)
        .try_write_owned()
        .map_err(|_| "Stop the active conversation before rewinding history")?;
    if store.active_run_for_session(id)?.is_some() {
        return Err("Stop the active conversation before rewinding history".into());
    }
    let messages = store.get_messages(id)?;
    let keep = match target {
        Target::After(message) => {
            messages
                .iter()
                .position(|m| m.id == message)
                .ok_or("Message not found")?
                + 1
        }
        Target::Before(message) => messages
            .iter()
            .position(|m| m.id == message)
            .ok_or("Message not found")?,
        Target::LatestTurn => match messages
            .iter()
            .rposition(|m| m.role == MessageRole::User && m.tool_call_id.is_none())
        {
            Some(index) => index,
            None => messages.len(),
        },
    };
    let expected: Vec<_> = messages.iter().map(|m| m.id.clone()).collect();
    if keep == messages.len() {
        return Ok(Some(PreparedRewind {
            id: id.into(),
            expected,
            keep,
            native_id: None,
            _guard: guard,
        }));
    }
    let metadata = store.get_response_request_metadata(id)?;
    let todos = messages[..keep]
        .iter()
        .rev()
        .find_map(|m| {
            metadata
                .get(&m.id)
                .and_then(|value| value.get("claude_todos"))
        })
        .cloned();
    let mut new_id = uuid::Uuid::new_v4().to_string();
    if keep > 0 {
        let record = metadata
            .get(&messages[keep - 1].id)
            .ok_or("This message has no acknowledged Claude rewind boundary")?;
        let point = &record["claude_checkpoint"];
        if point["version"] != 1 {
            return Err("This message has no acknowledged Claude rewind boundary".into());
        }
        let at = point["uuid"]
            .as_str()
            .ok_or("Missing Claude checkpoint UUID")?;
        let source_id = point["sessionId"].as_str().unwrap_or(&source_id);
        for value in [source_id, at] {
            uuid::Uuid::parse_str(value).map_err(|_| "Invalid Claude rewind boundary")?;
        }
        let after_compact = record["claude_compaction"]["checkpoint"]["uuid"] == point["uuid"];
        let remote = recovery::recorded_remote(store,id)?;
        let remote = ssh::native_session_connection(id, true, Some(remote))?;
        if let Some(connection) = remote {
            match connection {
                NativeConnection::Ssh(config) => {
                    let (_transport,_)=ssh::Transport::start(&config,json!({"mode":"rewind","authorizedSetup":true,
                        "sessionKey":ssh::session_key(id,&config),"sourceId":source_id,"newId":new_id,"at":at,"afterCompact":after_compact,"todos":todos}),None).await?;
                }
                NativeConnection::Server{binding_id} => {
                    #[cfg(feature="server-client")]
                    {
                        tui::close_session(id);
                        let reply=super::server_transport::operation(&binding_id,id,"rewind",json!({"sourceId":source_id,"at":at,"afterCompact":after_compact,"todos":todos})).await?;
                        new_id=reply["nativeId"].as_str().ok_or("Server did not return the rewound conversation identity")?.into();
                        uuid::Uuid::parse_str(&new_id).map_err(|_|"Invalid rewind identity")?;
                    }
                    #[cfg(not(feature="server-client"))]
                    {let _=binding_id;return Err("This conversation requires the internal Locus Server client".into());}
                }
            }
        } else {
            let root = ssh::profile_root(id)?;
            prepare_local(&root, source_id, &new_id, at, after_compact)?;
            restore_tasks(&root, &new_id, todos.as_ref())?;
        }
    }
    if keep==0 {
        if let Some(NativeConnection::Server{binding_id})=recovery::recorded_remote(store,id)? {
            #[cfg(feature="server-client")]
            {
                tui::close_session(id);
                super::server_transport::operation(&binding_id,id,"reset",json!({"sourceId":source_id})).await?;
            }
            #[cfg(not(feature="server-client"))]
            {let _=binding_id;return Err("This conversation requires the internal Locus Server client".into());}
        }
    }
    Ok(Some(PreparedRewind {
        id: id.into(),
        expected,
        keep,
        native_id: (keep > 0).then_some(new_id),
        _guard: guard,
    }))
}

pub(super) fn restore_tasks(
    root: &Path,
    id: &str,
    snapshot: Option<&serde_json::Value>,
) -> Result<(), String> {
    let Some(snapshot) = snapshot else {
        return Ok(());
    };
    if snapshot["version"] != 1 {
        return Err("Unsupported Claude todo snapshot".into());
    }
    if snapshot["kind"] != "tasks" {
        return Ok(());
    }
    let tasks = snapshot["tasks"]
        .as_array()
        .ok_or("Invalid native task snapshot")?;
    let target = root.join("profile/tasks").join(id);
    std::fs::create_dir_all(&target).map_err(|e| e.to_string())?;
    for task in tasks {
        let task_id = task["id"]
            .as_str()
            .filter(|id| {
                !id.is_empty()
                    && id.len() <= 128
                    && id
                        .bytes()
                        .all(|c| c.is_ascii_alphanumeric() || b"_-".contains(&c))
            })
            .ok_or("Invalid native task ID")?;
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(target.join(format!("{task_id}.json")))
            .map_err(|e| e.to_string())?;
        use std::io::Write;
        file.write_all(task.to_string().as_bytes())
            .and_then(|_| file.sync_all())
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

pub(super) fn prepare_local(
    root: &Path,
    source_id: &str,
    new_id: &str,
    at: &str,
    after_compact: bool,
) -> Result<(), String> {
    let source = match fork::find_transcript(root, source_id) {
        Ok(path) => path,
        Err(error) => {
            let state: fork::ForkState = std::fs::read(root.join("fork.json"))
                .ok()
                .and_then(|v| serde_json::from_slice(&v).ok())
                .ok_or(error)?;
            if state.version != 1 || state.new_id != source_id {
                return Err("Claude source transcript is unavailable".into());
            }
            fork::find_transcript(root, &state.source_id)?
        }
    };
    let snapshot = fork::snapshot(
        &std::fs::read_to_string(&source).map_err(|e| e.to_string())?,
        at,
        after_compact,
    )?;
    let output = reidentify(&snapshot, new_id)?;
    let target = source
        .parent()
        .ok_or("Missing transcript directory")?
        .join(format!("{new_id}.jsonl"));
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(target)
        .map_err(|e| e.to_string())?;
    use std::io::Write;
    file.write_all(output.as_bytes())
        .and_then(|_| file.sync_all())
        .map_err(|e| e.to_string())
}
fn reidentify(snapshot: &str, id: &str) -> Result<String, String> {
    let mut output = String::new();
    let mut pending = std::collections::HashSet::new();
    for line in snapshot.lines() {
        let mut value: serde_json::Value = serde_json::from_str(line).map_err(|e| e.to_string())?;
        let content = value["message"]["content"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        if value["isCompactSummary"] == true
            || (value["type"] == "user"
                && value["isMeta"] != true
                && !content.iter().any(|block| block["type"] == "tool_result"))
        {
            pending.clear();
        }
        for block in content {
            if block["type"] == "tool_use" {
                if let Some(id) = block["id"].as_str() {
                    pending.insert(id.to_string());
                }
            }
            if block["type"] == "tool_result" {
                if let Some(id) = block["tool_use_id"].as_str() {
                    pending.remove(id);
                }
            }
        }
        value["sessionId"] = json!(id);
        if value.get("session_id").is_some() {
            value["session_id"] = json!(id);
        }
        output.push_str(&value.to_string());
        output.push('\n');
    }
    if !pending.is_empty() {
        return Err("Choose a rewind boundary after all tool results; pending native tools cannot be replayed".into());
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rewind_preserves_source_and_rejects_pending_tool_replay() {
        let input=json!({"type":"assistant","sessionId":"old","message":{"content":[{"type":"tool_use","id":"call","name":"Write"}]}}).to_string();
        assert!(reidentify(&input, "new").is_err());
        let complete = format!(
            "{input}\n{}",
            json!({"type":"user","sessionId":"old","message":{"content":[{"type":"tool_result","tool_use_id":"call","content":"done"}]}})
        );
        let output = reidentify(&complete, "new").unwrap();
        assert!(output
            .lines()
            .all(
                |line| serde_json::from_str::<serde_json::Value>(line).unwrap()["sessionId"]
                    == "new"
            ));
        assert!(complete.contains("old"));
    }
}
