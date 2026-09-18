use std::collections::{BTreeSet, HashMap};

use serde::Serialize;

use crate::session::models::{AssistantRenderPart, ChatMessage, MessageRole, SessionEventRecord};

/// IDs, rather than timestamps, keep turns distinct even when several messages
/// and provider attempts are persisted in the same second.
#[derive(Debug, Serialize)]
pub(super) struct MessageTurnSelection {
    kind: &'static str,
    pub selected_message_id: String,
    pub message_ids: BTreeSet<String>,
    pub run_ids: BTreeSet<String>,
}

impl MessageTurnSelection {
    pub fn resolve(
        messages: &[ChatMessage],
        timeline: &[SessionEventRecord],
        message_id: &str,
    ) -> Result<Self, String> {
        let selected = messages
            .iter()
            .position(|message| message.id == message_id)
            .ok_or_else(|| format!("Message not found in session: {message_id}"))?;
        let start = messages[..=selected]
            .iter()
            .rposition(|message| message.role == MessageRole::User)
            .unwrap_or(0);
        let end = messages[selected + 1..]
            .iter()
            .position(|message| message.role == MessageRole::User)
            .map(|index| selected + 1 + index)
            .unwrap_or(messages.len());

        let mut message_runs: HashMap<&str, BTreeSet<&str>> = HashMap::new();
        for message in messages {
            for part in message.render_parts.iter().flatten() {
                let order = match part {
                    AssistantRenderPart::Thinking { order, .. }
                    | AssistantRenderPart::Text { order, .. }
                    | AssistantRenderPart::ToolCall { order, .. }
                    | AssistantRenderPart::KnowledgeProposal { order, .. } => order,
                };
                if !order.run_id.is_empty() && order.run_id != super::EMPTY {
                    message_runs
                        .entry(&message.id)
                        .or_default()
                        .insert(&order.run_id);
                }
            }
        }
        for event in timeline {
            if event.run_id.is_empty() || event.run_id == super::EMPTY {
                continue;
            }
            // Read only the event's message identity, never IDs in arbitrary
            // tool output, user text, or provider payloads.
            let id = match event.event_type.as_str() {
                "userMessage" | "knowledgeProposal" => event.payload.pointer("/message/id"),
                "toolCallRoundDone" | "done" | "pendingInputAccepted" | "cancelled" => {
                    event.payload.get("messageId")
                }
                _ => None,
            }
            .and_then(serde_json::Value::as_str);
            if let Some(id) = id {
                message_runs.entry(id).or_default().insert(&event.run_id);
            }
        }

        let mut selection = Self {
            kind: "message_turn",
            selected_message_id: message_id.to_string(),
            message_ids: messages[start..end]
                .iter()
                .map(|message| message.id.clone())
                .collect(),
            run_ids: BTreeSet::new(),
        };
        // Steering inputs can add user messages to an existing run. Include
        // their shared run, and associate separately persisted tool outputs by
        // call ID, so selecting either side of that turn yields the same scope.
        loop {
            let before = (selection.message_ids.len(), selection.run_ids.len());
            for id in &selection.message_ids {
                if let Some(runs) = message_runs.get(id.as_str()) {
                    selection
                        .run_ids
                        .extend(runs.iter().map(|id| id.to_string()));
                }
            }
            for message in messages {
                if message_runs
                    .get(message.id.as_str())
                    .is_some_and(|runs| runs.iter().any(|id| selection.run_ids.contains(*id)))
                {
                    selection.message_ids.insert(message.id.clone());
                }
            }
            let tool_ids: BTreeSet<&str> = messages
                .iter()
                .filter(|message| selection.message_ids.contains(&message.id))
                .flat_map(|message| {
                    message
                        .tool_calls
                        .iter()
                        .flatten()
                        .map(|call| call.id.as_str())
                        .chain(message.render_parts.iter().flatten().filter_map(
                            |part| match part {
                                AssistantRenderPart::ToolCall { tool_call, .. } => {
                                    Some(tool_call.id.as_str())
                                }
                                _ => None,
                            },
                        ))
                })
                .collect();
            for message in messages {
                if message
                    .tool_call_id
                    .as_deref()
                    .is_some_and(|id| tool_ids.contains(id))
                {
                    selection.message_ids.insert(message.id.clone());
                }
            }
            if before == (selection.message_ids.len(), selection.run_ids.len()) {
                break;
            }
        }
        Ok(selection)
    }
}
