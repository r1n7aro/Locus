use std::collections::HashMap;
use std::sync::Mutex;

use crate::commands::{StreamEvent, ToolCallOutcome};

use super::models::{
    AssistantRenderPart, PendingQuestion, PendingToolConfirm, RenderOrderKey, SessionRunSummary,
    SessionRuntimeSnapshot, ToolCallDisplay, ToolCallDisplayStatus, ToolCallInfo,
    ToolCallProgressSnapshot,
};

const RUN_STATUS_STARTING: &str = "starting";
const RUN_STATUS_RUNNING: &str = "running";
const RUN_STATUS_WAITING_INPUT: &str = "waiting_input";
const RUN_STATUS_CANCELLING: &str = "cancelling";
const RUN_STATUS_DONE: &str = "done";
const RUN_STATUS_CANCELLED: &str = "cancelled";
const RUN_STATUS_ERROR: &str = "error";

#[derive(Debug, Default)]
pub struct SessionRuntimeRegistry {
    sessions: Mutex<HashMap<String, SessionRuntimeSnapshot>>,
}

impl SessionRuntimeRegistry {
    pub fn start_run(&self, session_id: &str, run_id: &str) {
        let now = now_ts();
        match self.sessions.lock() {
            Ok(mut sessions) => {
                sessions.insert(
                    session_id.to_string(),
                    empty_snapshot(session_id, run_id, RUN_STATUS_STARTING, now),
                );
            }
            Err(error) => {
                eprintln!(
                    "[Locus] failed to start runtime session state for {} run {}: {}",
                    session_id, run_id, error
                );
            }
        }
    }

    pub fn snapshot(&self, session_id: &str) -> Option<SessionRuntimeSnapshot> {
        self.sessions
            .lock()
            .ok()
            .and_then(|sessions| sessions.get(session_id).cloned())
    }

    pub fn update_run_status(&self, run_id: &str, status: &str) {
        match self.sessions.lock() {
            Ok(mut sessions) => {
                if is_terminal_status(status) {
                    sessions.retain(|_, snapshot| snapshot.active_run.run_id != run_id);
                    return;
                }
                let now = now_ts();
                for snapshot in sessions.values_mut() {
                    if snapshot.active_run.run_id == run_id {
                        if snapshot.active_run.status == RUN_STATUS_CANCELLING
                            && status != RUN_STATUS_CANCELLING
                        {
                            continue;
                        }
                        set_snapshot_run_status(snapshot, status);
                        snapshot.active_run.updated_at = now;
                    }
                }
            }
            Err(error) => {
                eprintln!(
                    "[Locus] failed to update runtime run state for {}: {}",
                    run_id, error
                );
            }
        }
    }

    pub fn clear_run_if_current(&self, session_id: &str, run_id: &str) {
        match self.sessions.lock() {
            Ok(mut sessions) => {
                let should_clear = sessions
                    .get(session_id)
                    .map(|snapshot| snapshot.active_run.run_id == run_id)
                    .unwrap_or(false);
                if should_clear {
                    sessions.remove(session_id);
                }
            }
            Err(error) => {
                eprintln!(
                    "[Locus] failed to clear runtime session state for {} run {}: {}",
                    session_id, run_id, error
                );
            }
        }
    }

    pub fn clear_session(&self, session_id: &str) {
        if let Ok(mut sessions) = self.sessions.lock() {
            sessions.remove(session_id);
        }
    }

    pub fn apply_stream_event(&self, run_id: &str, event: &StreamEvent) {
        let session_id = event_session_id(event);
        if is_terminal_event(event) {
            self.update_run_status(
                run_id,
                match event {
                    StreamEvent::Done { .. } => RUN_STATUS_DONE,
                    StreamEvent::Cancelled { .. } => RUN_STATUS_CANCELLED,
                    StreamEvent::Error { .. } => RUN_STATUS_ERROR,
                    _ => RUN_STATUS_RUNNING,
                },
            );
            return;
        }

        let status = runtime_status_for_event(event);
        match self.sessions.lock() {
            Ok(mut sessions) => {
                let snapshot = ensure_snapshot(
                    &mut sessions,
                    session_id,
                    run_id,
                    status.unwrap_or(RUN_STATUS_RUNNING),
                );
                if let Some(status) = status {
                    set_snapshot_run_status(snapshot, status);
                }
                snapshot.active_run.updated_at = now_ts();
                apply_event_to_snapshot(snapshot, run_id, event);
            }
            Err(error) => {
                eprintln!(
                    "[Locus] failed to update runtime session state for {} run {}: {}",
                    session_id, run_id, error
                );
            }
        }
    }
}

fn now_ts() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

fn empty_snapshot(
    session_id: &str,
    run_id: &str,
    status: &str,
    now: i64,
) -> SessionRuntimeSnapshot {
    SessionRuntimeSnapshot {
        active_run: SessionRunSummary {
            run_id: run_id.to_string(),
            session_id: session_id.to_string(),
            status: status.to_string(),
            started_at: now,
            updated_at: now,
            finished_at: None,
            error_message: None,
        },
        active_tool_calls: Vec::new(),
        streaming_text: String::new(),
        streaming_thinking: String::new(),
        live_render_parts: Vec::new(),
        stream_sequence: 0,
        streaming_text_order: 0,
        thinking_order: 0,
        is_thinking: false,
        thinking_duration: 0,
        pending_question: None,
        pending_tool_confirms: Vec::new(),
        is_compacting: false,
    }
}

fn set_snapshot_run_status(snapshot: &mut SessionRuntimeSnapshot, status: &str) {
    if snapshot.active_run.status == RUN_STATUS_CANCELLING && status != RUN_STATUS_CANCELLING {
        return;
    }
    snapshot.active_run.status = status.to_string();
}

fn ensure_snapshot<'a>(
    sessions: &'a mut HashMap<String, SessionRuntimeSnapshot>,
    session_id: &str,
    run_id: &str,
    status: &str,
) -> &'a mut SessionRuntimeSnapshot {
    let replace = sessions
        .get(session_id)
        .map(|snapshot| snapshot.active_run.run_id != run_id)
        .unwrap_or(true);
    if replace {
        sessions.insert(
            session_id.to_string(),
            empty_snapshot(session_id, run_id, status, now_ts()),
        );
    }
    sessions.get_mut(session_id).expect("snapshot exists")
}

fn is_terminal_status(status: &str) -> bool {
    matches!(
        status,
        RUN_STATUS_DONE | RUN_STATUS_CANCELLED | RUN_STATUS_ERROR
    )
}

fn is_terminal_event(event: &StreamEvent) -> bool {
    matches!(
        event,
        StreamEvent::Done { .. } | StreamEvent::Cancelled { .. } | StreamEvent::Error { .. }
    )
}

fn runtime_status_for_event(event: &StreamEvent) -> Option<&'static str> {
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
        | StreamEvent::CompactDone { .. } => Some(RUN_STATUS_RUNNING),
        StreamEvent::AskUser { .. } | StreamEvent::ToolConfirm { .. } => {
            Some(RUN_STATUS_WAITING_INPUT)
        }
        StreamEvent::KnowledgeProposal { .. }
        | StreamEvent::PendingInputQueued { .. }
        | StreamEvent::PendingInputDeleted { .. }
        | StreamEvent::PendingInputAccepted { .. }
        | StreamEvent::Done { .. }
        | StreamEvent::Cancelled { .. }
        | StreamEvent::Error { .. } => None,
    }
}

fn event_session_id(event: &StreamEvent) -> &str {
    match event {
        StreamEvent::RunStart { session_id }
        | StreamEvent::UserMessage { session_id, .. }
        | StreamEvent::PendingInputQueued { session_id, .. }
        | StreamEvent::PendingInputDeleted { session_id, .. }
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

fn apply_event_to_snapshot(
    snapshot: &mut SessionRuntimeSnapshot,
    run_id: &str,
    event: &StreamEvent,
) {
    match event {
        StreamEvent::RunStart { .. } => {
            reset_round_runtime(snapshot);
            snapshot.pending_question = None;
            snapshot.pending_tool_confirms.clear();
            snapshot.is_compacting = false;
        }
        StreamEvent::TextDelta {
            text,
            order,
            part_id,
            render_seq,
            ..
        } => {
            append_text_delta(
                snapshot,
                run_id,
                text,
                *order,
                part_id.as_deref(),
                *render_seq,
            );
        }
        StreamEvent::ThinkingDelta {
            text,
            order,
            part_id,
            render_seq,
            ..
        } => {
            append_thinking_delta(
                snapshot,
                run_id,
                text,
                *order,
                part_id.as_deref(),
                *render_seq,
            );
        }
        StreamEvent::ToolCallStart {
            tool_call_id,
            tool_name,
            arguments,
            order,
            part_id,
            render_seq,
            ..
        } => {
            deactivate_live_thinking_parts(snapshot);
            let resolved_order = (*order).or_else(|| Some(next_stream_order(snapshot)));
            if let Some(order) = resolved_order {
                mark_stream_sequence(snapshot, order);
            }
            upsert_tool_call(
                &mut snapshot.active_tool_calls,
                tool_call_id,
                tool_name,
                arguments,
                resolved_order,
            );
            upsert_live_tool_call_part(
                snapshot,
                run_id,
                part_id.as_deref(),
                tool_call_id,
                resolved_order,
                *render_seq,
            );
        }
        StreamEvent::ToolCallDone {
            tool_call_id,
            tool_name,
            output,
            outcome,
            images,
            ..
        } => {
            update_tool_call_done(
                &mut snapshot.active_tool_calls,
                tool_call_id,
                tool_name,
                *outcome,
                output,
                images.clone(),
            );
            refresh_live_tool_call_part(snapshot, tool_call_id);
        }
        StreamEvent::ToolCallDelta {
            tool_call_id,
            delta,
            ..
        } => {
            if let Some(tool_call) =
                find_tool_call_mut(&mut snapshot.active_tool_calls, tool_call_id)
            {
                let output = tool_call.output.get_or_insert_with(String::new);
                output.push_str(delta);
            }
        }
        StreamEvent::ToolCallProgress {
            tool_call_id,
            title,
            info,
            progress,
            state,
            ..
        } => {
            if let Some(tool_call) =
                find_tool_call_mut(&mut snapshot.active_tool_calls, tool_call_id)
            {
                tool_call.progress = Some(ToolCallProgressSnapshot {
                    title: title.clone(),
                    info: info.clone(),
                    progress: *progress,
                    state: state.clone(),
                });
            }
        }
        StreamEvent::SubagentToolCallStart {
            parent_tool_call_id,
            tool_call_id,
            tool_name,
            arguments,
            order,
            ..
        } => {
            deactivate_live_thinking_parts(snapshot);
            let resolved_order = (*order).or_else(|| Some(next_stream_order(snapshot)));
            if let Some(order) = resolved_order {
                mark_stream_sequence(snapshot, order);
            }
            let parent =
                ensure_parent_tool_call(&mut snapshot.active_tool_calls, parent_tool_call_id);
            upsert_nested_tool_call(parent, tool_call_id, tool_name, arguments, resolved_order);
            refresh_live_tool_call_part(snapshot, parent_tool_call_id);
        }
        StreamEvent::SubagentToolCallDone {
            parent_tool_call_id,
            tool_call_id,
            tool_name,
            output,
            outcome,
            images,
            ..
        } => {
            let parent =
                ensure_parent_tool_call(&mut snapshot.active_tool_calls, parent_tool_call_id);
            update_nested_tool_call_done(
                parent,
                tool_call_id,
                tool_name,
                *outcome,
                output,
                images.clone(),
            );
            refresh_live_tool_call_part(snapshot, parent_tool_call_id);
        }
        StreamEvent::ToolCallRoundDone { .. } => {
            reset_round_runtime(snapshot);
        }
        StreamEvent::AskUser {
            question_id,
            tool_call_id,
            question,
            options,
            sheet,
            ..
        } => {
            snapshot.pending_question = Some(PendingQuestion {
                question_id: question_id.clone(),
                tool_call_id: tool_call_id.clone(),
                question: question.clone(),
                options: options.clone(),
                sheet: sheet.clone(),
            });
        }
        StreamEvent::ToolConfirm {
            question_id,
            tool_call_id,
            display,
            ..
        } => {
            snapshot
                .pending_tool_confirms
                .retain(|confirm| confirm.question_id != *question_id);
            snapshot.pending_tool_confirms.push(PendingToolConfirm {
                question_id: question_id.clone(),
                tool_call_id: tool_call_id.clone(),
                display: display.clone(),
            });
        }
        StreamEvent::InputAnswered { question_id, .. } => {
            if snapshot
                .pending_question
                .as_ref()
                .map(|question| question.question_id.as_str())
                == Some(question_id.as_str())
            {
                snapshot.pending_question = None;
            }
            snapshot
                .pending_tool_confirms
                .retain(|confirm| confirm.question_id != *question_id);
        }
        StreamEvent::CompactStart { .. } => {
            snapshot.is_compacting = true;
        }
        StreamEvent::CompactDone { .. } => {
            snapshot.is_compacting = false;
        }
        StreamEvent::PendingInputQueued { .. }
        | StreamEvent::PendingInputDeleted { .. }
        | StreamEvent::PendingInputAccepted { .. }
        | StreamEvent::UserMessage { .. }
        | StreamEvent::KnowledgeProposal { .. }
        | StreamEvent::UsageUpdate { .. }
        | StreamEvent::UndoAvailable { .. }
        | StreamEvent::Done { .. }
        | StreamEvent::Cancelled { .. }
        | StreamEvent::Error { .. } => {}
    }
}

fn reset_round_runtime(snapshot: &mut SessionRuntimeSnapshot) {
    snapshot.streaming_text.clear();
    snapshot.streaming_thinking.clear();
    snapshot.live_render_parts.clear();
    snapshot.stream_sequence = 0;
    snapshot.streaming_text_order = 0;
    snapshot.thinking_order = 0;
    snapshot.is_thinking = false;
    snapshot.thinking_duration = 0;
    snapshot.active_tool_calls.clear();
}

fn next_stream_order(snapshot: &SessionRuntimeSnapshot) -> u32 {
    snapshot.stream_sequence.saturating_add(1)
}

fn mark_stream_sequence(snapshot: &mut SessionRuntimeSnapshot, order: u32) {
    if order > snapshot.stream_sequence {
        snapshot.stream_sequence = order;
    }
}

fn resolve_stream_order(snapshot: &mut SessionRuntimeSnapshot, explicit_order: Option<u32>) -> u32 {
    let order = explicit_order
        .filter(|order| *order > snapshot.stream_sequence)
        .unwrap_or_else(|| next_stream_order(snapshot));
    mark_stream_sequence(snapshot, order);
    order
}

fn ensure_streaming_text_order(
    snapshot: &mut SessionRuntimeSnapshot,
    explicit_order: Option<u32>,
) -> u32 {
    if snapshot.streaming_text_order == 0 {
        snapshot.streaming_text_order = resolve_stream_order(snapshot, explicit_order);
    }
    snapshot.streaming_text_order
}

fn ensure_thinking_order(
    snapshot: &mut SessionRuntimeSnapshot,
    explicit_order: Option<u32>,
) -> u32 {
    if snapshot.thinking_order == 0 {
        snapshot.thinking_order = resolve_stream_order(snapshot, explicit_order);
    }
    snapshot.thinking_order
}

fn resolve_live_render_order(
    snapshot: &mut SessionRuntimeSnapshot,
    run_id: &str,
    fallback_order: u32,
    render_seq: Option<u32>,
) -> RenderOrderKey {
    let seq = render_seq.filter(|seq| *seq > 0).unwrap_or(fallback_order);
    mark_stream_sequence(snapshot, seq);
    RenderOrderKey {
        run_id: run_id.to_string(),
        seq,
    }
}

fn normalized_part_id(raw: Option<&str>, fallback: String) -> String {
    raw.map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .unwrap_or(fallback)
}

fn append_text_delta(
    snapshot: &mut SessionRuntimeSnapshot,
    run_id: &str,
    text: &str,
    explicit_order: Option<u32>,
    part_id: Option<&str>,
    render_seq: Option<u32>,
) {
    let fallback_order = ensure_streaming_text_order(snapshot, explicit_order);
    let order = resolve_live_render_order(snapshot, run_id, fallback_order, render_seq);
    let id = normalized_part_id(part_id, format!("{run_id}:text"));

    deactivate_live_thinking_parts(snapshot);
    snapshot.streaming_text.push_str(text);
    upsert_live_text_part(&mut snapshot.live_render_parts, id, order, text);
}

fn append_thinking_delta(
    snapshot: &mut SessionRuntimeSnapshot,
    run_id: &str,
    text: &str,
    explicit_order: Option<u32>,
    part_id: Option<&str>,
    render_seq: Option<u32>,
) {
    let fallback_order = ensure_thinking_order(snapshot, explicit_order);
    let order = resolve_live_render_order(snapshot, run_id, fallback_order, render_seq);
    let id = normalized_part_id(part_id, format!("{run_id}:thinking"));

    snapshot.streaming_thinking.push_str(text);
    snapshot.is_thinking = true;
    upsert_live_thinking_part(&mut snapshot.live_render_parts, id, order, text);
}

fn deactivate_live_thinking_parts(snapshot: &mut SessionRuntimeSnapshot) {
    for part in &mut snapshot.live_render_parts {
        if let AssistantRenderPart::Thinking {
            active, duration, ..
        } = part
        {
            *active = Some(false);
            if snapshot.thinking_duration > 0 && duration.is_none() {
                *duration = Some(snapshot.thinking_duration);
            }
        }
    }
    snapshot.is_thinking = false;
}

fn upsert_live_text_part(
    parts: &mut Vec<AssistantRenderPart>,
    id: String,
    order: RenderOrderKey,
    delta: &str,
) {
    for part in parts.iter_mut() {
        if let AssistantRenderPart::Text {
            id: existing_id,
            order: existing_order,
            content,
        } = part
        {
            if *existing_id == id {
                *existing_order = order;
                content.push_str(delta);
                return;
            }
        }
    }
    parts.push(AssistantRenderPart::Text {
        id,
        order,
        content: delta.to_string(),
    });
}

fn upsert_live_thinking_part(
    parts: &mut Vec<AssistantRenderPart>,
    id: String,
    order: RenderOrderKey,
    delta: &str,
) {
    for part in parts.iter_mut() {
        if let AssistantRenderPart::Thinking {
            id: existing_id,
            order: existing_order,
            content,
            active,
            ..
        } = part
        {
            if *existing_id == id {
                *existing_order = order;
                content.push_str(delta);
                *active = Some(true);
                return;
            }
        }
    }
    parts.push(AssistantRenderPart::Thinking {
        id,
        order,
        content: delta.to_string(),
        active: Some(true),
        duration: None,
        signature: None,
    });
}

fn upsert_live_tool_call_part(
    snapshot: &mut SessionRuntimeSnapshot,
    run_id: &str,
    part_id: Option<&str>,
    tool_call_id: &str,
    fallback_order: Option<u32>,
    render_seq: Option<u32>,
) {
    let Some(tool_call) = find_tool_call(&snapshot.active_tool_calls, tool_call_id) else {
        return;
    };
    let fallback_order = fallback_order
        .or(tool_call.order)
        .unwrap_or_else(|| next_stream_order(snapshot));
    let tool_call = tool_call_info_from_display(tool_call);
    let order = resolve_live_render_order(snapshot, run_id, fallback_order, render_seq);
    let id = normalized_part_id(part_id, tool_call_id.to_string());

    for part in snapshot.live_render_parts.iter_mut() {
        if let AssistantRenderPart::ToolCall {
            id: existing_id,
            order: existing_order,
            tool_call: existing_tool_call,
        } = part
        {
            if *existing_id == id {
                *existing_order = order;
                *existing_tool_call = tool_call;
                return;
            }
        }
    }

    snapshot
        .live_render_parts
        .push(AssistantRenderPart::ToolCall {
            id,
            order,
            tool_call,
        });
}

fn refresh_live_tool_call_part(snapshot: &mut SessionRuntimeSnapshot, tool_call_id: &str) {
    let Some(tool_call) =
        find_tool_call(&snapshot.active_tool_calls, tool_call_id).map(tool_call_info_from_display)
    else {
        return;
    };

    for part in snapshot.live_render_parts.iter_mut() {
        if let AssistantRenderPart::ToolCall {
            tool_call: existing_tool_call,
            ..
        } = part
        {
            if existing_tool_call.id == tool_call_id {
                *existing_tool_call = tool_call.clone();
            }
        }
    }
}

fn tool_call_info_from_display(tool_call: &ToolCallDisplay) -> ToolCallInfo {
    ToolCallInfo {
        id: tool_call.id.clone(),
        name: tool_call.name.clone(),
        arguments: tool_call.arguments.clone(),
        order: tool_call.order,
        server_tool: None,
        server_tool_output: None,
        outcome: tool_call_outcome_from_status(&tool_call.status),
        recorded_output: tool_call.output.clone(),
        nested_tool_calls: tool_call
            .nested_tool_calls
            .as_ref()
            .map(|nested| nested.iter().map(tool_call_info_from_display).collect()),
    }
}

fn tool_call_outcome_from_status(status: &ToolCallDisplayStatus) -> Option<ToolCallOutcome> {
    match status {
        ToolCallDisplayStatus::Running => None,
        ToolCallDisplayStatus::Done => Some(ToolCallOutcome::Done),
        ToolCallDisplayStatus::Error => Some(ToolCallOutcome::Error),
        ToolCallDisplayStatus::Interrupted => Some(ToolCallOutcome::Interrupted),
    }
}

fn next_order(tool_calls: &[ToolCallDisplay]) -> u32 {
    tool_calls
        .iter()
        .filter_map(|tool_call| tool_call.order)
        .max()
        .unwrap_or(tool_calls.len() as u32)
        + 1
}

fn new_tool_call(
    id: &str,
    name: &str,
    arguments: &str,
    status: ToolCallDisplayStatus,
    order: Option<u32>,
) -> ToolCallDisplay {
    ToolCallDisplay {
        id: id.to_string(),
        name: name.to_string(),
        arguments: arguments.to_string(),
        status,
        order,
        output: None,
        images: None,
        progress: None,
        nested_tool_calls: None,
    }
}

fn upsert_tool_call(
    tool_calls: &mut Vec<ToolCallDisplay>,
    id: &str,
    name: &str,
    arguments: &str,
    order: Option<u32>,
) {
    if let Some(tool_call) = find_tool_call_mut(tool_calls, id) {
        if !name.trim().is_empty() {
            tool_call.name = name.to_string();
        }
        if !arguments.trim().is_empty() {
            tool_call.arguments = arguments.to_string();
        }
        tool_call.status = ToolCallDisplayStatus::Running;
        if order.is_some() {
            tool_call.order = order;
        }
        return;
    }

    let resolved_order = order.or_else(|| Some(next_order(tool_calls)));
    tool_calls.push(new_tool_call(
        id,
        name,
        arguments,
        ToolCallDisplayStatus::Running,
        resolved_order,
    ));
}

fn update_tool_call_done(
    tool_calls: &mut Vec<ToolCallDisplay>,
    id: &str,
    name: &str,
    outcome: ToolCallOutcome,
    output: &str,
    images: Option<Vec<super::models::ImageData>>,
) {
    if find_tool_call_mut(tool_calls, id).is_none() {
        let order = Some(next_order(tool_calls));
        tool_calls.push(new_tool_call(
            id,
            name,
            "",
            ToolCallDisplayStatus::from_outcome(outcome),
            order,
        ));
    }

    if let Some(tool_call) = find_tool_call_mut(tool_calls, id) {
        if !name.trim().is_empty() {
            tool_call.name = name.to_string();
        }
        tool_call.status = ToolCallDisplayStatus::from_outcome(outcome);
        tool_call.output = Some(output.to_string());
        tool_call.images = images;
        tool_call.progress = None;
    }
}

fn ensure_parent_tool_call<'a>(
    tool_calls: &'a mut Vec<ToolCallDisplay>,
    parent_id: &str,
) -> &'a mut ToolCallDisplay {
    if let Some(index) = tool_calls
        .iter()
        .position(|tool_call| tool_call.id == parent_id)
    {
        return tool_calls.get_mut(index).expect("parent index exists");
    }

    let order = Some(next_order(tool_calls));
    tool_calls.push(new_tool_call(
        parent_id,
        "task",
        "{}",
        ToolCallDisplayStatus::Running,
        order,
    ));
    tool_calls.last_mut().expect("parent inserted")
}

fn upsert_nested_tool_call(
    parent: &mut ToolCallDisplay,
    id: &str,
    name: &str,
    arguments: &str,
    order: Option<u32>,
) {
    let nested = parent.nested_tool_calls.get_or_insert_with(Vec::new);
    if let Some(tool_call) = nested.iter_mut().find(|tool_call| tool_call.id == id) {
        if !name.trim().is_empty() {
            tool_call.name = name.to_string();
        }
        if !arguments.trim().is_empty() {
            tool_call.arguments = arguments.to_string();
        }
        tool_call.status = ToolCallDisplayStatus::Running;
        if order.is_some() {
            tool_call.order = order;
        }
        return;
    }

    let resolved_order = order.or_else(|| Some(next_order(nested)));
    nested.push(new_tool_call(
        id,
        name,
        arguments,
        ToolCallDisplayStatus::Running,
        resolved_order,
    ));
}

fn update_nested_tool_call_done(
    parent: &mut ToolCallDisplay,
    id: &str,
    name: &str,
    outcome: ToolCallOutcome,
    output: &str,
    images: Option<Vec<super::models::ImageData>>,
) {
    let nested = parent.nested_tool_calls.get_or_insert_with(Vec::new);
    if nested.iter().all(|tool_call| tool_call.id != id) {
        let order = Some(next_order(nested));
        nested.push(new_tool_call(
            id,
            name,
            "",
            ToolCallDisplayStatus::from_outcome(outcome),
            order,
        ));
    }
    if let Some(tool_call) = nested.iter_mut().find(|tool_call| tool_call.id == id) {
        if !name.trim().is_empty() {
            tool_call.name = name.to_string();
        }
        tool_call.status = ToolCallDisplayStatus::from_outcome(outcome);
        tool_call.output = Some(output.to_string());
        tool_call.images = images;
        tool_call.progress = None;
    }
}

fn find_tool_call_mut<'a>(
    tool_calls: &'a mut [ToolCallDisplay],
    id: &str,
) -> Option<&'a mut ToolCallDisplay> {
    tool_calls.iter_mut().find(|tool_call| tool_call.id == id)
}

fn find_tool_call<'a>(tool_calls: &'a [ToolCallDisplay], id: &str) -> Option<&'a ToolCallDisplay> {
    tool_calls.iter().find(|tool_call| tool_call.id == id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::{StreamEvent, ToolCallOutcome};

    #[test]
    fn runtime_snapshot_tracks_nested_subagent_tool_calls() {
        let registry = SessionRuntimeRegistry::default();
        registry.start_run("s1", "run-1");
        registry.apply_stream_event(
            "run-1",
            &StreamEvent::ToolCallStart {
                session_id: "s1".to_string(),
                tool_call_id: "task-1".to_string(),
                tool_name: "task".to_string(),
                arguments: "{\"prompt\":\"inspect\"}".to_string(),
                order: Some(1),
                part_id: None,
                render_seq: None,
            },
        );
        registry.apply_stream_event(
            "run-1",
            &StreamEvent::SubagentToolCallStart {
                session_id: "s1".to_string(),
                parent_tool_call_id: "task-1".to_string(),
                tool_call_id: "read-1".to_string(),
                tool_name: "read".to_string(),
                arguments: "{\"path\":\"src/main.ts\"}".to_string(),
                order: Some(2),
                part_id: None,
                render_seq: None,
            },
        );

        let snapshot = registry.snapshot("s1").expect("runtime snapshot");
        assert_eq!(snapshot.active_run.run_id, "run-1");
        assert_eq!(snapshot.active_tool_calls.len(), 1);
        let parent = &snapshot.active_tool_calls[0];
        assert_eq!(parent.id, "task-1");
        let nested = parent
            .nested_tool_calls
            .as_ref()
            .expect("nested tool calls");
        assert_eq!(nested.len(), 1);
        assert_eq!(nested[0].id, "read-1");
        assert_eq!(nested[0].status, ToolCallDisplayStatus::Running);
    }

    #[test]
    fn terminal_event_clears_runtime_snapshot() {
        let registry = SessionRuntimeRegistry::default();
        registry.start_run("s1", "run-1");
        registry.apply_stream_event(
            "run-1",
            &StreamEvent::ToolCallStart {
                session_id: "s1".to_string(),
                tool_call_id: "tc-1".to_string(),
                tool_name: "read".to_string(),
                arguments: "{}".to_string(),
                order: Some(1),
                part_id: None,
                render_seq: None,
            },
        );
        assert!(registry.snapshot("s1").is_some());

        registry.apply_stream_event(
            "run-1",
            &StreamEvent::Done {
                session_id: "s1".to_string(),
                message_id: "m1".to_string(),
                full_text: "done".to_string(),
                content_order: None,
                thinking_order: None,
                render_parts: None,
            },
        );

        assert!(registry.snapshot("s1").is_none());
    }

    #[test]
    fn tool_done_updates_status_and_output() {
        let registry = SessionRuntimeRegistry::default();
        registry.start_run("s1", "run-1");
        registry.apply_stream_event(
            "run-1",
            &StreamEvent::ToolCallDone {
                session_id: "s1".to_string(),
                tool_call_id: "tc-1".to_string(),
                tool_name: "read".to_string(),
                output: "content".to_string(),
                outcome: ToolCallOutcome::Done,
                images: None,
            },
        );

        let snapshot = registry.snapshot("s1").expect("runtime snapshot");
        let tool_call = &snapshot.active_tool_calls[0];
        assert_eq!(tool_call.status, ToolCallDisplayStatus::Done);
        assert_eq!(tool_call.output.as_deref(), Some("content"));
    }

    #[test]
    fn runtime_snapshot_tracks_streaming_text_and_thinking_parts() {
        let registry = SessionRuntimeRegistry::default();
        registry.start_run("s1", "run-1");
        registry.apply_stream_event(
            "run-1",
            &StreamEvent::ThinkingDelta {
                session_id: "s1".to_string(),
                text: "plan".to_string(),
                order: Some(1),
                part_id: Some("thinking-part".to_string()),
                render_seq: Some(1),
            },
        );
        registry.apply_stream_event(
            "run-1",
            &StreamEvent::TextDelta {
                session_id: "s1".to_string(),
                text: "answer".to_string(),
                order: Some(2),
                part_id: Some("text-part".to_string()),
                render_seq: Some(2),
            },
        );

        let snapshot = registry.snapshot("s1").expect("runtime snapshot");
        assert_eq!(snapshot.streaming_thinking, "plan");
        assert_eq!(snapshot.streaming_text, "answer");
        assert_eq!(snapshot.stream_sequence, 2);
        assert_eq!(snapshot.thinking_order, 1);
        assert_eq!(snapshot.streaming_text_order, 2);
        assert!(!snapshot.is_thinking);
        assert_eq!(snapshot.live_render_parts.len(), 2);
        match &snapshot.live_render_parts[0] {
            AssistantRenderPart::Thinking {
                id,
                content,
                active,
                ..
            } => {
                assert_eq!(id, "thinking-part");
                assert_eq!(content, "plan");
                assert_eq!(*active, Some(false));
            }
            other => panic!("expected thinking part, got {:?}", other),
        }
        match &snapshot.live_render_parts[1] {
            AssistantRenderPart::Text { id, content, .. } => {
                assert_eq!(id, "text-part");
                assert_eq!(content, "answer");
            }
            other => panic!("expected text part, got {:?}", other),
        }
    }

    #[test]
    fn cancelling_runtime_status_is_not_overwritten_by_late_stream_events() {
        let registry = SessionRuntimeRegistry::default();
        registry.start_run("s1", "run-1");
        registry.update_run_status("run-1", RUN_STATUS_CANCELLING);
        registry.apply_stream_event(
            "run-1",
            &StreamEvent::TextDelta {
                session_id: "s1".to_string(),
                text: "late".to_string(),
                order: Some(1),
                part_id: None,
                render_seq: None,
            },
        );

        let snapshot = registry.snapshot("s1").expect("runtime snapshot");
        assert_eq!(snapshot.active_run.status, RUN_STATUS_CANCELLING);
        assert_eq!(snapshot.streaming_text, "late");
    }
}
