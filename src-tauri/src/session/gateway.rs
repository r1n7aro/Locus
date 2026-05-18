use tauri::{AppHandle, Emitter, Manager};

use crate::commands::{StreamEvent, StreamEventEnvelope};
use crate::session::store::{
    SessionEventAppend, SessionEventMerge, SessionRunStatusUpdate, SessionStore,
};

const RUN_STATUS_RUNNING: &str = "running";
pub(crate) const RUN_STATUS_CANCELLING: &str = "cancelling";
const RUN_STATUS_WAITING_INPUT: &str = "waiting_input";
const RUN_STATUS_DONE: &str = "done";
const RUN_STATUS_CANCELLED: &str = "cancelled";
const RUN_STATUS_ERROR: &str = "error";

fn event_session_id(event: &StreamEvent) -> &str {
    match event {
        StreamEvent::RunStart { session_id }
        | StreamEvent::UserMessage { session_id, .. }
        | StreamEvent::PendingInputQueued { session_id, .. }
        | StreamEvent::PendingInputAccepted { session_id, .. }
        | StreamEvent::TextDelta { session_id, .. }
        | StreamEvent::ThinkingDelta { session_id, .. }
        | StreamEvent::ToolCallStart { session_id, .. }
        | StreamEvent::ToolCallDone { session_id, .. }
        | StreamEvent::ToolCallDelta { session_id, .. }
        | StreamEvent::ToolCallProgress { session_id, .. }
        | StreamEvent::SubagentToolCallStart { session_id, .. }
        | StreamEvent::SubagentToolCallDone { session_id, .. }
        | StreamEvent::ToolCallRoundDone { session_id, .. }
        | StreamEvent::Done { session_id, .. }
        | StreamEvent::KnowledgeProposal { session_id, .. }
        | StreamEvent::UsageUpdate { session_id, .. }
        | StreamEvent::AskUser { session_id, .. }
        | StreamEvent::ToolConfirm { session_id, .. }
        | StreamEvent::InputAnswered { session_id, .. }
        | StreamEvent::UndoAvailable { session_id, .. }
        | StreamEvent::CompactStart { session_id, .. }
        | StreamEvent::CompactDone { session_id, .. }
        | StreamEvent::Cancelled { session_id, .. }
        | StreamEvent::Error { session_id, .. } => session_id,
    }
}

fn event_type(event: &StreamEvent) -> &'static str {
    match event {
        StreamEvent::RunStart { .. } => "runStart",
        StreamEvent::UserMessage { .. } => "userMessage",
        StreamEvent::PendingInputQueued { .. } => "pendingInputQueued",
        StreamEvent::PendingInputAccepted { .. } => "pendingInputAccepted",
        StreamEvent::TextDelta { .. } => "textDelta",
        StreamEvent::ThinkingDelta { .. } => "thinkingDelta",
        StreamEvent::ToolCallStart { .. } => "toolCallStart",
        StreamEvent::ToolCallDone { .. } => "toolCallDone",
        StreamEvent::ToolCallDelta { .. } => "toolCallDelta",
        StreamEvent::ToolCallProgress { .. } => "toolCallProgress",
        StreamEvent::SubagentToolCallStart { .. } => "subagentToolCallStart",
        StreamEvent::SubagentToolCallDone { .. } => "subagentToolCallDone",
        StreamEvent::ToolCallRoundDone { .. } => "toolCallRoundDone",
        StreamEvent::Done { .. } => "done",
        StreamEvent::KnowledgeProposal { .. } => "knowledgeProposal",
        StreamEvent::UsageUpdate { .. } => "usageUpdate",
        StreamEvent::AskUser { .. } => "askUser",
        StreamEvent::ToolConfirm { .. } => "toolConfirm",
        StreamEvent::InputAnswered { .. } => "inputAnswered",
        StreamEvent::UndoAvailable { .. } => "undoAvailable",
        StreamEvent::CompactStart { .. } => "compactStart",
        StreamEvent::CompactDone { .. } => "compactDone",
        StreamEvent::Cancelled { .. } => "cancelled",
        StreamEvent::Error { .. } => "error",
    }
}

fn run_status_for_event(event: &StreamEvent) -> Option<(&'static str, Option<String>)> {
    match event {
        StreamEvent::RunStart { .. }
        | StreamEvent::UserMessage { .. }
        | StreamEvent::TextDelta { .. }
        | StreamEvent::ThinkingDelta { .. }
        | StreamEvent::ToolCallStart { .. }
        | StreamEvent::ToolCallDone { .. }
        | StreamEvent::ToolCallDelta { .. }
        | StreamEvent::ToolCallProgress { .. }
        | StreamEvent::SubagentToolCallStart { .. }
        | StreamEvent::SubagentToolCallDone { .. }
        | StreamEvent::ToolCallRoundDone { .. }
        | StreamEvent::UsageUpdate { .. }
        | StreamEvent::InputAnswered { .. }
        | StreamEvent::UndoAvailable { .. }
        | StreamEvent::CompactStart { .. }
        | StreamEvent::CompactDone { .. } => Some((RUN_STATUS_RUNNING, None)),
        StreamEvent::AskUser { .. } | StreamEvent::ToolConfirm { .. } => {
            Some((RUN_STATUS_WAITING_INPUT, None))
        }
        StreamEvent::Done { .. } => Some((RUN_STATUS_DONE, None)),
        StreamEvent::Cancelled { .. } => Some((RUN_STATUS_CANCELLED, None)),
        StreamEvent::Error { error, .. } => Some((RUN_STATUS_ERROR, Some(error.message.clone()))),
        StreamEvent::KnowledgeProposal { .. }
        | StreamEvent::PendingInputQueued { .. }
        | StreamEvent::PendingInputAccepted { .. } => None,
    }
}

fn is_terminal_run_status(status: &str) -> bool {
    matches!(
        status,
        RUN_STATUS_DONE | RUN_STATUS_CANCELLED | RUN_STATUS_ERROR
    )
}

fn event_merge(
    run_id: &str,
    session_id: &str,
    event_kind: &str,
    event: &StreamEvent,
) -> Option<SessionEventMerge> {
    match event {
        StreamEvent::TextDelta { text, .. } => Some(SessionEventMerge {
            key: format!("{}\u{0}{}\u{0}{}", session_id, run_id, event_kind),
            field: "text".to_string(),
            value: text.clone(),
        }),
        StreamEvent::ThinkingDelta { text, .. } => Some(SessionEventMerge {
            key: format!("{}\u{0}{}\u{0}{}", session_id, run_id, event_kind),
            field: "text".to_string(),
            value: text.clone(),
        }),
        StreamEvent::ToolCallDelta {
            tool_call_id,
            delta,
            ..
        } => Some(SessionEventMerge {
            key: format!(
                "{}\u{0}{}\u{0}{}\u{0}{}",
                session_id, run_id, event_kind, tool_call_id
            ),
            field: "delta".to_string(),
            value: delta.clone(),
        }),
        _ => None,
    }
}

#[cfg(debug_assertions)]
#[derive(Debug, PartialEq, Eq)]
enum RunSessionValidation {
    Match,
    Mismatch { run_session_id: String },
    UnknownRun,
}

#[cfg(debug_assertions)]
fn validate_run_session(
    store: &SessionStore,
    run_id: &str,
    event_session_id: &str,
) -> Result<RunSessionValidation, String> {
    match store.session_id_for_run(run_id)? {
        Some(run_session_id) if run_session_id == event_session_id => {
            Ok(RunSessionValidation::Match)
        }
        Some(run_session_id) => Ok(RunSessionValidation::Mismatch { run_session_id }),
        None => Ok(RunSessionValidation::UnknownRun),
    }
}

#[cfg(debug_assertions)]
fn warn_if_run_session_mismatch(
    store: &SessionStore,
    run_id: &str,
    event_session_id: &str,
    event_kind: &str,
) {
    match validate_run_session(store, run_id, event_session_id) {
        Ok(RunSessionValidation::Mismatch { run_session_id }) => {
            eprintln!(
                "[Locus] warning: stream event session/run mismatch: event={} event_session={} run={} run_session={}",
                event_kind, event_session_id, run_id, run_session_id
            );
        }
        Ok(RunSessionValidation::Match | RunSessionValidation::UnknownRun) => {}
        Err(error) => {
            eprintln!(
                "[Locus] warning: failed to validate stream event session/run ownership: event={} event_session={} run={} error={}",
                event_kind, event_session_id, run_id, error
            );
        }
    }
}

pub fn emit_stream(app_handle: &AppHandle, store: &SessionStore, run_id: &str, event: StreamEvent) {
    let session_id = event_session_id(&event).to_string();
    let event_kind = event_type(&event);
    if matches!(
        &event,
        StreamEvent::Cancelled { .. } | StreamEvent::Error { .. }
    ) {
        if let Some(queue_state) = app_handle.try_state::<crate::PendingInputQueueHandle>() {
            match queue_state.lock() {
                Ok(mut queue) => queue.clear_run(&session_id, run_id),
                Err(error) => eprintln!(
                    "[Locus] failed to clear pending input queue for session {} run {}: {}",
                    session_id, run_id, error
                ),
            }
        }
    }
    #[cfg(debug_assertions)]
    warn_if_run_session_mismatch(store, run_id, &session_id, event_kind);

    let mut run_status =
        run_status_for_event(&event).map(|(status, error_message)| SessionRunStatusUpdate {
            run_id: run_id.to_string(),
            status: status.to_string(),
            error_message,
        });
    let merge = event_merge(run_id, &session_id, event_kind, &event);

    if run_status
        .as_ref()
        .is_some_and(|status| is_terminal_run_status(&status.status))
    {
        if let Some(status) = run_status.take() {
            if let Err(error) = store.update_run_status(
                &status.run_id,
                &status.status,
                status.error_message.as_deref(),
            ) {
                eprintln!(
                    "[Locus] failed to update terminal run status {} for session {} run {}: {}",
                    status.status, session_id, run_id, error
                );
            }
        }
    }

    let event_for_persist = event.clone();
    let _ = app_handle.emit(
        "stream-event",
        StreamEventEnvelope {
            run_id: run_id.to_string(),
            event,
        },
    );

    match serde_json::to_string(&event_for_persist) {
        Ok(payload_json) => {
            if let Err(error) = store.enqueue_session_event(
                SessionEventAppend {
                    session_id: session_id.clone(),
                    run_id: run_id.to_string(),
                    event_type: event_kind.to_string(),
                    payload_json,
                },
                merge,
                run_status,
            ) {
                eprintln!(
                    "[Locus] failed to queue session event {} for session {} run {}: {}",
                    event_kind, session_id, run_id, error
                );
            }
        }
        Err(error) => {
            eprintln!(
                "[Locus] failed to serialize session event {} for session {} run {}: {}",
                event_kind, session_id, run_id, error
            );
        }
    }
}

#[cfg(all(test, debug_assertions))]
mod tests {
    use tempfile::tempdir;

    use super::{validate_run_session, RunSessionValidation};
    use crate::session::store::SessionStore;

    #[test]
    fn validate_run_session_detects_known_owner_mismatch() {
        let dir = tempdir().expect("create temp dir");
        let store = SessionStore::new(dir.path()).expect("initialize store");
        let run_session_id = store
            .create_session("Run Owner", None, None, "chat", None)
            .expect("create run session");
        let other_session_id = store
            .create_session("Other", None, None, "chat", None)
            .expect("create other session");

        store
            .try_start_run(&run_session_id, "run-1")
            .expect("start run");

        assert_eq!(
            validate_run_session(&store, "run-1", &run_session_id).expect("validate matching run"),
            RunSessionValidation::Match
        );
        assert_eq!(
            validate_run_session(&store, "run-1", &other_session_id)
                .expect("validate mismatched run"),
            RunSessionValidation::Mismatch {
                run_session_id: run_session_id.clone()
            }
        );
        assert_eq!(
            validate_run_session(&store, "knowledge_1", &other_session_id)
                .expect("validate unknown run"),
            RunSessionValidation::UnknownRun
        );
    }
}
