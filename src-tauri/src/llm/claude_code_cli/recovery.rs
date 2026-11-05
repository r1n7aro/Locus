//! Read the bound remote transcript over SSH. Never replay Locus history into
//! Claude, create a replacement session, or issue an inference request here.
use super::*;
use super::connection::NativeConnection;
use crate::session::{models::{ChatMessage, MessageRole, TodoItem}, store::SessionStore};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[cfg(test)]
#[path = "recovery_tests.rs"]
mod tests;
#[cfg(test)]
#[path = "turn_recovery_tests.rs"]
mod turn_recovery_tests;

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NativeAssistantPart {
    pub uuid: String,
    pub content: Vec<Value>,
    pub timestamp: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NativeMessage {
    pub key: String,
    pub role: MessageRole,
    pub uuids: Vec<String>,
    pub content: Value,
    pub timestamp: Option<String>,
    pub model: Option<String>,
    pub usage: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stop_reason: Option<String>,
    pub tool_call_id: Option<String>,
    #[serde(default)]
    pub is_error: bool,
    #[serde(default)]
    pub compact_summary: bool,
    /// A consumed human input persisted as a native queued_command attachment.
    #[serde(default)]
    pub queued_input: bool,
    // Source boundaries are needed to reconcile the individual tool rounds
    // streamed by the CLI with its grouped API response. Older snapshots may
    // omit them; ambiguous matches in those snapshots still fail closed.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub parts: Vec<NativeAssistantPart>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NativeSnapshot {
    pub version: u32,
    pub native_id: String,
    pub digest: String,
    pub messages: Vec<NativeMessage>,
    pub todos: Vec<TodoItem>,
    #[serde(default)]
    pub task_snapshot: Value,
    pub incomplete_tail: bool,
    pub pending_tool_ids: Vec<String>,
    /// Native user records excluded from display, retained as UUIDs so recovery
    /// can remove older projections without touching the source transcript.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub hidden_user_uuids: Vec<String>,
}
impl NativeSnapshot {
    pub fn validate(&self, expected: Option<&str>) -> Result<(), String> {
        if self.version != 1 || uuid::Uuid::parse_str(&self.native_id).is_err()
            || expected.is_some_and(|id| id != self.native_id)
            || self.digest.len() != 64 || !self.digest.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err("Invalid or mismatched remote Claude snapshot".into());
        }
        let mut hidden = std::collections::HashSet::new();
        let visible: std::collections::HashSet<_> = self.messages.iter().flat_map(|m| &m.uuids).collect();
        for id in &self.hidden_user_uuids {
            if uuid::Uuid::parse_str(id).is_err() || !hidden.insert(id) || visible.contains(id) {
                return Err("Invalid hidden native user record identities".into());
            }
        }
        let mut keys = std::collections::HashSet::new();
        for message in &self.messages {
            if message.key.is_empty() || !keys.insert(&message.key) || message.uuids.is_empty()
                || message.uuids.iter().any(|id| uuid::Uuid::parse_str(id).is_err()) {
                return Err("Remote Claude snapshot contains invalid or duplicate message identities".into());
            }
            if message.role == MessageRole::Tool && message.tool_call_id.as_deref().is_none_or(str::is_empty) {
                return Err("Remote Claude tool result has no call identity".into());
            }
            if !message.parts.is_empty() && (message.role != MessageRole::Assistant
                || message.parts.iter().map(|p| &p.uuid).ne(message.uuids.iter())
                || message.parts.iter().map(|p| &p.uuid).collect::<std::collections::HashSet<_>>().len() != message.parts.len()
                || message.content != Value::Array(message.parts.iter().flat_map(|p| p.content.clone()).collect())) {
                return Err("Remote Claude assistant parts do not match the grouped record".into());
            }
        }
        Ok(())
    }
    pub fn can_continue(&self) -> Result<(), String> {
        if self.incomplete_tail {
            return Err("Remote Claude history has an incomplete final record. Saved messages were synchronized; continuing is blocked until the native history is repaired.".into());
        }
        if !self.pending_tool_ids.is_empty() {
            return Err("Remote Claude history contains tool calls without confirmed results. Saved messages were synchronized; tools were not repeated. Review the interrupted operations before continuing.".into());
        }
        Ok(())
    }

    fn can_compact(&self) -> Result<(), String> {
        // Compaction summarizes committed native history without executing or
        // retrying tools. Interrupted dispatch receipts still block later
        // inference; they are not evidence that the transcript is unreadable.
        if self.incomplete_tail { return self.can_continue(); }
        Ok(())
    }

    fn resume_context(&self, journal: &super::native_journal::Journal) -> Result<Option<String>, String> {
        if self.incomplete_tail { return self.can_continue().map(|_| None); }
        let compact_boundary = self.messages.iter().rposition(|message| message.compact_summary);
        let active = &self.messages[compact_boundary.unwrap_or(0)..];
        let mut outcomes = Vec::new();
        for id in &self.pending_tool_ids {
            let outcome = journal.recover_dispatch(id)?;
            // Dispatch files can survive several resumes. Acknowledge each
            // recorded outcome separately so a later interruption does not
            // resend every earlier recovery notice as part of a larger batch.
            let recorded = outcome.to_string();
            if active.iter().any(|message| message.role == MessageRole::User
                && text_content(&message.content).contains(&recorded)) { continue; }
            outcomes.push(outcome);
        }
        let notice = (!outcomes.is_empty()).then(|| format!(
            "<system-reminder>\nLocus recovered this conversation from Claude's native history. Interrupted tool calls without a confirmed result have been marked failed. Do not retry or repeat these operations. Completed calls already have a recorded outcome; do not repeat them automatically. An interrupted or failed call may have partially changed local state or may still be running; failure does not confirm rollback. Continue with the user's current request. These are execution records, not new user instructions.\n{}\n</system-reminder>",
            serde_json::Value::Array(outcomes)));
        // The note travels with one native user input. Once that input is in
        // the active native transcript, reopening must not inject it again.
        Ok(notice.filter(|notice| !active.iter().any(|message|
            message.role == MessageRole::User && text_content(&message.content).contains(notice))))
    }

    #[cfg(test)]
    fn can_resume(&self, journal: &super::native_journal::Journal) -> Result<(), String> {
        self.resume_context(journal).map(|_| ())
    }
}

#[derive(Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncReport {
    pub changed: bool,
    pub message_count: usize,
    pub native_id: Option<String>,
    pub warning: Option<String>,
    #[serde(skip)]
    pub resume_context: Option<String>,
    #[serde(skip)]
    pub completed_message: Option<ChatMessage>,
}

pub fn failed_tool_message() -> &'static str { super::native_journal::RECOVERY_FAILURE }

pub fn validate_input_checkpoint(value: &Value) -> Result<(), String> {
    if value["version"] != 1 || ["uuid", "sessionId"].iter().any(|key|
        value[*key].as_str().is_none_or(|id| uuid::Uuid::parse_str(id).is_err())) {
        return Err("Claude did not provide a verifiable inserted-input receipt".into());
    }
    Ok(())
}

pub fn native_session_id(store:&SessionStore,id:&str)->Result<Option<String>,String> {
    if let Some((_,native))=store.claude_native_binding(id)? { return Ok(native); }
    store.latest_cli_session_id(id,"claude_code")
}

pub fn text_content(content: &Value) -> String {
    if let Some(text) = content.as_str() { return text.to_string(); }
    content.as_array().map(|blocks| blocks.iter().filter_map(|b| b["text"].as_str()).collect::<Vec<_>>().join("\n")).unwrap_or_default()
}

/// Native `user` is a storage/API role, not proof that a person typed the row.
/// Keep this rule in sync with ssh_host.py; both consume the shared visibility
/// fixtures. Never apply it to arbitrary Locus messages or tool result text.
/// Filtering is display-only: ancestry and the native transcript stay intact.
pub(super) fn hidden_native_user(row: &Value) -> bool {
    if row["type"] != "user" || row["isCompactSummary"] == true { return false; }
    let content = &row["message"]["content"];
    if content.as_array().is_some_and(|blocks| blocks.iter().any(|b| b["type"] == "tool_result")) {
        return false;
    }
    let text = text_content(content);
    if pending_input_token(&text).is_some() { return false; }
    if ["isMeta", "isVisibleInTranscriptOnly", "isVirtual"].iter().any(|key| row[*key] == true) {
        return true;
    }
    // A real prompt may deliberately quote the CLI's wrappers. New Locus
    // inputs carry a pending receipt; native human prompts carry an origin.
    if row["origin"]["kind"] == "human" { return false; }
    let text = text.trim_start();
    ["<command-name>", "<command-message>", "<command-args>",
     "<local-command-stdout>", "<local-command-stderr>", "<local-command-caveat>",
     "<bash-input>", "<bash-stdout>", "<bash-stderr>", "<bash-exit-code>",
     "<task-notification>"].iter().any(|prefix| text.starts_with(*prefix))
}

pub fn assistant_content(content: &Value) -> ClaudeCodeAssistantMessage {
    super::parse_assistant_message(&serde_json::json!({"message":{"content":content}})).unwrap_or_default()
}

pub fn images(content: &Value) -> Vec<ImageData> {
    content.as_array().into_iter().flatten().filter_map(|block| {
        if block["type"] != "image" || block["source"]["type"] != "base64" { return None; }
        Some(ImageData { data: block["source"]["data"].as_str()?.into(), mime_type: block["source"]["media_type"].as_str()?.into(), capture: None })
    }).collect()
}

pub fn recorded_remote(store: &SessionStore, id: &str) -> Result<Option<NativeConnection>, String> {
    recorded_connection(store,id).map(Option::flatten)
}
pub fn recorded_connection(store: &SessionStore, id: &str) -> Result<Option<Option<NativeConnection>>, String> {
    if let Some(remote)=store.claude_active_remote(id)? {
        if let Some(ssh) = remote.as_ssh() { ssh::bind_restored_connection(id,ssh)?; }
        return Ok(Some(Some(remote)));
    }
    let metadata = store.get_response_request_metadata(id)?;
    let mut result: Option<Option<NativeConnection>> = None;
    for value in metadata.values().filter_map(|v| v.get("claude_connection")) {
        let remote: Option<NativeConnection> = serde_json::from_value(value.clone()).map_err(|_| "Invalid recorded Claude connection")?;
        if result.as_ref().is_some_and(|prior| prior != &remote) {
            return Err("Conflicting native Claude server bindings; automatic synchronization stopped".into());
        }
        result = Some(remote);
    }
    Ok(result)
}

pub fn verify_origin_with_pending(messages: &[ChatMessage], metadata: &HashMap<String, Value>, pending: &[String]) -> Result<(), String> {
    let native = |id: &str| metadata.get(id).is_some_and(|v| v["backend"] == "claude_code_cli" || v.get("claude_connection").is_some() || v.get("claude_checkpoint").is_some());
    let native_tools: std::collections::HashSet<_> = messages.iter().filter(|m| native(&m.id))
        .flat_map(|m| m.tool_calls.iter().flatten().map(|t| t.id.as_str())).collect();
    for message in messages {
        if pending.contains(&message.id) && message.role == MessageRole::User { continue; }
        if native(&message.id) || message.tool_call_id.as_deref().is_some_and(|id| native_tools.contains(id)) { continue; }
        return Err("This conversation contains API or unverified history. It cannot be synchronized with or sent to Claude Code CLI.".into());
    }
    Ok(())
}

// A transport receipt, not an instruction or a replacement for native UUIDs.
// Native note rows carry older unsent user text before the final TUI submission;
// the token lets recovery recognize an append whose acknowledgement was lost.
pub fn pending_input_token(text: &str) -> Option<&str> {
    let token = text.strip_prefix("<locus-pending-user id=\"")?.split_once("\">\n")?.0;
    uuid::Uuid::parse_str(token).ok()?;
    Some(token)
}
pub fn pending_input_text(token: &str, text: &str) -> String {
    format!("<locus-pending-user id=\"{token}\">\n{text}\n</locus-pending-user>")
}

const CONTINUATION_OPEN: &str = "<system-reminder>\n<locus-continuation>\n";
const CONTINUATION_CLOSE: &str = "\n</locus-continuation>\n</system-reminder>";

/// Keep the internal-input identity in native history as well as the local
/// prompt suffix, so an authoritative rebuild cannot turn it into user text.
pub(crate) fn continuation_prompt(text: &str) -> String {
    format!("{CONTINUATION_OPEN}{text}{CONTINUATION_CLOSE}")
}

pub(crate) fn is_continuation_prompt(text: &str) -> bool {
    text.strip_prefix(CONTINUATION_OPEN)
        .is_some_and(|body| body.contains(CONTINUATION_CLOSE))
}

pub async fn fetch(id: &str, remote: &(impl Clone + Into<NativeConnection>), native: Option<&str>) -> Result<NativeSnapshot, String> {
    fetch_snapshot(id, &remote.clone().into(), native, false).await
}

async fn fetch_snapshot(id: &str, remote: &NativeConnection, native: Option<&str>, require_idle: bool) -> Result<NativeSnapshot, String> {
    let value = match remote {
        NativeConnection::Ssh(remote) => {
            let (_transport, info) = ssh::Transport::start(remote, serde_json::json!({
                "mode":"history", "sessionKey":ssh::session_key(id, remote), "nativeId":native, "requireIdle":require_idle,
            }), None).await?;
            info.history.ok_or("Remote host did not return a native history snapshot")?
        }
        NativeConnection::Server { binding_id } => {
            #[cfg(feature = "server-client")]
            { super::server_transport::operation(binding_id,id,"history",json!({"nativeId":native,"requireIdle":require_idle})).await? }
            #[cfg(not(feature = "server-client"))]
            { let _ = binding_id; return Err("This conversation requires the internal Locus Server client".into()); }
        }
    };
    let snapshot: NativeSnapshot = serde_json::from_value(value)
        .map_err(|e| format!("Invalid remote history snapshot: {e}"))?;
    snapshot.validate(native)?;
    Ok(snapshot)
}

async fn synchronization_guard(id: &str) -> Result<tokio::sync::OwnedRwLockWriteGuard<()>, String> {
    // Prewarm and another view's synchronization are finite work, not an
    // active conversation. Queue behind them instead of reporting a false
    // busy warning. The live-session check is repeated under this lease.
    tokio::time::timeout(Duration::from_secs(90), tui::history_gate(id).write_owned()).await
        .map_err(|_| "Claude conversation is busy; synchronize after the current operation".into())
}

pub async fn synchronize(store: &SessionStore, id: &str, pending: Option<&str>) -> Result<SyncReport, String> {
    synchronize_inner(store, id, pending, false, false).await
}

pub async fn synchronize_for_compaction(store: &SessionStore, id: &str) -> Result<SyncReport, String> {
    synchronize_inner(store, id, None, true, false).await
}

/// Automatic recovery must fetch the source before deciding whether an input
/// was consumed. It also preserves every proven pending input, not just the
/// prompt that started the interrupted run.
pub(crate) async fn synchronize_for_retry(store: &SessionStore, id: &str, pending: Option<&str>) -> Result<SyncReport, String> {
    synchronize_inner(store, id, pending, false, true).await
}

fn verified_live_session(store: &SessionStore, id: &str, pending: Option<&str>) -> Result<bool, String> {
    if !tui::has_synchronized_session(id) { return Ok(false); }
    let messages=store.get_messages(id)?;
    let metadata=store.get_response_request_metadata(id)?;
    let pending=store.claude_recovery_pending_messages(id,&messages,&metadata,pending)?;
    if store.verify_claude_history_origin(id,&messages,&metadata,&pending).is_ok() { return Ok(true); }
    // An older live connection may have acknowledged a queued attachment
    // without saving its checkpoint. Fetch proof before attempting to send.
    tui::require_recovery(id);
    Ok(false)
}

async fn synchronize_inner(store: &SessionStore, id: &str, pending: Option<&str>, compact: bool, automatic: bool) -> Result<SyncReport, String> {
    synchronize_with_fetch(store, id, pending, compact, automatic, |remote, native| async move {
        if automatic {
            // Wait for an orphaned remote host's exclusive lease to end.
            return fetch_snapshot(id, &remote, native.as_deref(), true).await;
        }
        let archived = async {
            let info=super::archive::backup_for_history(store,id,pending).await?.ok_or("No native archive")?;
            super::archive::project(&info,&store.claude_archive_files(id)?)
        }.await;
        // A corrupt local archive/cache must not prevent reading the source server.
        match archived {
            Ok(snapshot) => Ok(snapshot),
            Err(error) => {
                eprintln!("[Claude] history archive fallback: session={} reason={}", id, error);
                fetch(id,&remote,native.as_deref()).await
            }
        }
    }).await.map_err(|error| super::turn_retry::with_context("History synchronization", error))
}

pub(crate) async fn synchronize_with_fetch<F, Fut>(
    store: &SessionStore, id: &str, pending: Option<&str>, compact: bool, automatic: bool,
    fetch: F,
) -> Result<SyncReport, String>
where
    F: FnOnce(NativeConnection, Option<String>) -> Fut,
    Fut: std::future::Future<Output = Result<NativeSnapshot, String>>,
{
    let passive = pending.is_none() && !compact && !automatic;
    if passive && store.active_run_for_session(id)?.is_some() { return Ok(SyncReport::default()); }
    let Some(remote) = recorded_remote(store, id)? else { return Ok(SyncReport::default()); };
    // A live, successfully streamed session already has its input/output lease.
    // Reuse it; do not open a second SSH connection before every prompt.
    if !automatic && verified_live_session(store,id,pending)? { return Ok(SyncReport::default()); }
    let _guard = synchronization_guard(id).await?;
    if passive && store.active_run_for_session(id)?.is_some() { return Ok(SyncReport::default()); }
    if !automatic && verified_live_session(store,id,pending)? { return Ok(SyncReport::default()); }
    let native = native_session_id(store,id)?;
    if native.is_none() {
        // A new/API conversation has no bound native history to restore.
        let messages=store.get_messages(id)?;
        let metadata=store.get_response_request_metadata(id)?;
        let pending_ids=store.claude_recovery_pending_messages(id,&messages,&metadata,pending)?;
        store.verify_claude_history_origin(id,&messages,&metadata,&pending_ids)?;
        return Ok(SyncReport::default());
    }
    if automatic || !tui::is_prewarmed(id) { tui::close_session(id); }
    let snapshot = fetch(remote.clone(), native.clone()).await?;
    if passive && store.active_run_for_session(id)?.is_some() { return Ok(SyncReport::default()); }
    snapshot.validate(native.as_deref())?;
    if snapshot.messages.is_empty() && !automatic {
        return Err("The remote Claude conversation has no committed messages. Local input was retained and was not automatically resubmitted.".into());
    }
    if snapshot.messages.is_empty() {
        snapshot.can_continue()?;
        // An empty source is proof of non-submission only when there are no
        // acknowledged messages or operations to lose. Never replay accepted
        // input merely because a server returned an empty transcript.
        let messages = store.get_messages(id)?;
        let metadata = store.get_response_request_metadata(id)?;
        let unsent = store.claude_recovery_pending_messages(id, &messages, &metadata, pending)?;
        if messages.iter().any(|message| !unsent.contains(&message.id)) {
            return Err("Native Claude history is empty despite locally recorded activity; automatic recovery stopped without replaying input.".into());
        }
    }
    let journal = super::native_journal::Journal::new(ssh::profile_root(id)?.join("calls"));
    let (resume_context, warning) = match snapshot.resume_context(&journal) {
        Ok(context) => (context, None),
        Err(warning) => (None, Some(warning)),
    };
    let messages=store.get_messages(id)?;
    let metadata=store.get_response_request_metadata(id)?;
    let pending_local=store.claude_recovery_pending_messages(id,&messages,&metadata,pending);
    // Merely opening a conversation must not discard its durably recorded
    // failed input. A later explicit submission still decides what to resume.
    let rebuild_reason = if resume_context.is_some() {
        Some("interrupted_tool_recovery".to_string())
    } else {
        match &pending_local {
            Err(error) => Some(format!("pending_input_validation_failed: {error}")),
            Ok(ids) if pending.is_some() && ids.iter().any(|old| Some(old.as_str()) != pending) =>
                Some("pending_input_replacement".to_string()),
            _ => None,
        }
    };
    let rebuild = |reason: &str| {
        eprintln!("[Claude] history rebuild started: session={} native={} automatic={} local_messages={} native_messages={} reason={}",
            id, snapshot.native_id, automatic, messages.len(), snapshot.messages.len(), reason);
        let result = if automatic {
            store.rebuild_claude_native_for_retry(id, &remote, &snapshot, &messages, pending)
        } else {
            store.rebuild_claude_native(id, &remote, &snapshot, &messages, pending)
        };
        if let Err(error) = &result {
            eprintln!("[Claude] history rebuild failed: session={} reason={} error={}", id, reason, error);
        }
        result
    };
    let changed = if passive {
        // Opening a view is not authorization to discard unsubmitted input.
        // In particular, a run may start while the remote snapshot is fetched.
        store.reconcile_claude_native_if_idle(id, &remote, &snapshot, &messages)?
    } else if let Some(reason) = rebuild_reason {
        rebuild(&reason)?
    } else {
        // Keep stable display identities and verified pre-compaction history in
        // the normal case. Inconsistent local projections are replaceable.
        match store.reconcile_claude_native(id,&remote,&snapshot,&messages,pending) {
            Ok(changed)=>changed,
            Err(error) => rebuild(&format!("reconciliation_failed: {error}"))?,
        }
    };
    store.bind_claude_native_session(id,&remote,&snapshot.native_id)?;
    // A process resumed during prewarm must reload if native history advanced.
    if changed { tui::close_session(id); }
    store_session_id(id, &snapshot.native_id).await;
    if compact { snapshot.can_compact()?; }
    let completed_message = if automatic && warning.is_none() && resume_context.is_none() {
        let current = store.get_messages(id)?;
        let current_metadata = store.get_response_request_metadata(id)?;
        let unsent = store.claude_recovery_pending_messages(id, &current, &current_metadata, pending)?;
        completed_native_message(&snapshot, pending.and_then(|id| metadata.get(id)), &unsent)
            .and_then(|native| current.into_iter().find(|message| current_metadata.get(&message.id)
                .is_some_and(|value| value["claude_checkpoint"]["uuid"].as_str() == native.uuids.last().map(String::as_str))))
    } else { None };
    Ok(SyncReport { changed, message_count: snapshot.messages.len(), native_id: Some(snapshot.native_id), warning, resume_context, completed_message })
}

// A persisted end_turn after the interrupted input proves completion even if
// its final bridge ACK was lost. Missing stop reasons, pending inputs/tools or
// an unrelated old answer are never treated as completion.
fn completed_native_message<'a>(snapshot: &'a NativeSnapshot, input: Option<&Value>, unsent: &[String]) -> Option<&'a NativeMessage> {
    if snapshot.incomplete_tail || !snapshot.pending_tool_ids.is_empty() || !unsent.is_empty() { return None; }
    let input = input?;
    let last = snapshot.messages.last()?;
    if last.role != MessageRole::Assistant || last.stop_reason.as_deref() != Some("end_turn")
        || !assistant_content(&last.content).tool_calls.is_empty() { return None; }
    let found = snapshot.messages[..snapshot.messages.len() - 1].iter().any(|message| {
        message.role == MessageRole::User && (
            input["claude_checkpoint"]["uuid"].as_str().is_some_and(|uuid| message.uuids.iter().any(|id| id == uuid))
            || input["claude_pending_input"]["token"].as_str().is_some_and(|token|
                pending_input_token(&text_content(&message.content)) == Some(token)))
    });
    found.then_some(last)
}
