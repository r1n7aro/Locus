use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

use uuid::Uuid;

use super::models::{
    AssetRefData, ImageData, PendingSessionInput, UserIntentPayload, UserIntentSkill,
};

const STATUS_QUEUED: &str = "queued";
const STATUS_DELIVERING: &str = "delivering";
pub const DELIVERY_AFTER_RUN: &str = "after_run";
pub const DELIVERY_IMMEDIATE: &str = "immediate";

#[derive(Default)]
pub struct PendingInputQueue {
    inputs: HashMap<(String, String), PendingSessionInput>,
}

impl PendingInputQueue {
    pub fn queue_input(&mut self, request: QueuePendingInputRequest) -> PendingSessionInput {
        let key = (request.session_id.clone(), request.run_id.clone());
        let now = now_ts();
        if let Some(existing) = self.inputs.get_mut(&key) {
            existing.text = join_pending_text(&existing.text, &request.text);
            existing.display_text =
                join_pending_text(&existing.display_text, &request.display_text);
            existing.delivery =
                merge_pending_delivery(&existing.delivery, request.delivery.as_deref()).to_string();
            existing.images = merge_optional_vec(existing.images.take(), request.images);
            existing.asset_refs =
                merge_optional_vec(existing.asset_refs.take(), request.asset_refs);
            existing.mode = merge_pending_mode(existing.mode.take(), request.mode);
            existing.client_message_id = existing
                .client_message_id
                .take()
                .or(request.client_message_id.clone());
            existing.user_intent = merge_user_intents(
                existing.user_intent.take(),
                request.user_intent,
                existing.client_message_id.clone(),
            );
            existing.updated_at = now;
            return existing.clone();
        }

        let input = PendingSessionInput {
            id: Uuid::new_v4().to_string(),
            session_id: request.session_id,
            run_id: request.run_id,
            merge_group_id: request.merge_group_id,
            status: STATUS_QUEUED.to_string(),
            delivery: normalize_delivery(request.delivery.as_deref()).to_string(),
            text: request.text,
            display_text: request.display_text,
            images: empty_to_none(request.images),
            asset_refs: empty_to_none(request.asset_refs),
            mode: request.mode,
            user_intent: request.user_intent,
            client_message_id: request.client_message_id,
            message_id: None,
            created_at: now,
            updated_at: now,
        };
        self.inputs.insert(key, input.clone());
        input
    }

    pub fn claim_immediate(&mut self, session_id: &str, run_id: &str) -> Vec<PendingSessionInput> {
        let key = (session_id.to_string(), run_id.to_string());
        match self.inputs.remove(&key) {
            Some(mut input)
                if input.status == STATUS_QUEUED && input.delivery == DELIVERY_IMMEDIATE =>
            {
                input.status = STATUS_DELIVERING.to_string();
                input.updated_at = now_ts();
                vec![input]
            }
            Some(input) => {
                self.inputs.insert(key, input);
                Vec::new()
            }
            None => Vec::new(),
        }
    }

    pub fn claim_after_run(
        &mut self,
        session_id: &str,
        run_id: &str,
    ) -> Option<PendingSessionInput> {
        let key = (session_id.to_string(), run_id.to_string());
        match self.inputs.remove(&key) {
            Some(mut input)
                if input.status == STATUS_QUEUED && input.delivery == DELIVERY_AFTER_RUN =>
            {
                input.status = STATUS_DELIVERING.to_string();
                input.updated_at = now_ts();
                Some(input)
            }
            Some(input) => {
                self.inputs.insert(key, input);
                None
            }
            None => None,
        }
    }

    pub fn promote_to_immediate(
        &mut self,
        session_id: &str,
        run_id: &str,
        pending_input_id: Option<&str>,
    ) -> Option<PendingSessionInput> {
        let key = (session_id.to_string(), run_id.to_string());
        let input = self.inputs.get_mut(&key)?;
        if input.status != STATUS_QUEUED {
            return None;
        }
        if pending_input_id.is_some_and(|id| id != input.id) {
            return None;
        }
        input.delivery = DELIVERY_IMMEDIATE.to_string();
        input.updated_at = now_ts();
        Some(input.clone())
    }

    pub fn restore_claimed(&mut self, inputs: Vec<PendingSessionInput>) {
        for mut input in inputs {
            input.status = STATUS_QUEUED.to_string();
            input.updated_at = now_ts();
            self.inputs
                .insert((input.session_id.clone(), input.run_id.clone()), input);
        }
    }

    pub fn clear_run(&mut self, session_id: &str, run_id: &str) {
        self.inputs
            .remove(&(session_id.to_string(), run_id.to_string()));
    }

    pub fn list_session(&self, session_id: &str) -> Vec<PendingSessionInput> {
        let mut inputs = self
            .inputs
            .values()
            .filter(|input| input.session_id == session_id && input.status == STATUS_QUEUED)
            .cloned()
            .collect::<Vec<_>>();
        inputs.sort_by_key(|input| (input.created_at, input.updated_at));
        inputs
    }
}

pub struct QueuePendingInputRequest {
    pub session_id: String,
    pub run_id: String,
    pub merge_group_id: String,
    pub text: String,
    pub display_text: String,
    pub images: Vec<ImageData>,
    pub asset_refs: Vec<AssetRefData>,
    pub mode: Option<String>,
    pub user_intent: Option<UserIntentPayload>,
    pub client_message_id: Option<String>,
    pub delivery: Option<String>,
}

fn now_ts() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or(0)
}

fn empty_to_none<T>(items: Vec<T>) -> Option<Vec<T>> {
    if items.is_empty() {
        None
    } else {
        Some(items)
    }
}

fn merge_optional_vec<T>(existing: Option<Vec<T>>, next: Vec<T>) -> Option<Vec<T>> {
    let mut merged = existing.unwrap_or_default();
    merged.extend(next);
    empty_to_none(merged)
}

fn join_pending_text(existing: &str, next: &str) -> String {
    let existing_trimmed = existing.trim();
    let next_trimmed = next.trim();
    match (existing_trimmed.is_empty(), next_trimmed.is_empty()) {
        (true, true) => String::new(),
        (true, false) => next.to_string(),
        (false, true) => existing.to_string(),
        (false, false) => format!("{}\n{}", existing, next),
    }
}

fn merge_pending_mode(existing: Option<String>, next: Option<String>) -> Option<String> {
    if existing.as_deref() == Some("plan") || next.as_deref() == Some("plan") {
        return Some("plan".to_string());
    }
    next.filter(|value| !value.trim().is_empty()).or(existing)
}

fn normalize_delivery(value: Option<&str>) -> &'static str {
    match value {
        Some(DELIVERY_IMMEDIATE) => DELIVERY_IMMEDIATE,
        _ => DELIVERY_AFTER_RUN,
    }
}

fn merge_pending_delivery(existing: &str, next: Option<&str>) -> &'static str {
    if existing == DELIVERY_IMMEDIATE || normalize_delivery(next) == DELIVERY_IMMEDIATE {
        DELIVERY_IMMEDIATE
    } else {
        DELIVERY_AFTER_RUN
    }
}

fn merge_user_intents(
    existing: Option<UserIntentPayload>,
    next: Option<UserIntentPayload>,
    client_message_id: Option<String>,
) -> Option<UserIntentPayload> {
    match (existing, next) {
        (None, None) => client_message_id.map(|id| UserIntentPayload {
            kind: "user_intent_v1".to_string(),
            mode: "build".to_string(),
            skills: Vec::new(),
            client_message_id: Some(id),
        }),
        (Some(mut existing), None) => {
            if existing.client_message_id.is_none() {
                existing.client_message_id = client_message_id;
            }
            Some(existing)
        }
        (None, Some(mut next)) => {
            if next.client_message_id.is_none() {
                next.client_message_id = client_message_id;
            }
            Some(next)
        }
        (Some(mut existing), Some(next)) => {
            if next.mode == "plan" || existing.mode == "plan" {
                existing.mode = "plan".to_string();
            }
            let mut seen = HashMap::<String, ()>::new();
            let mut skills = Vec::<UserIntentSkill>::new();
            for skill in existing.skills.into_iter().chain(next.skills) {
                let key = format!("{}:{}", skill.source, skill.dir_name);
                if seen.insert(key, ()).is_some() {
                    continue;
                }
                skills.push(skill);
            }
            existing.skills = skills;
            if existing.client_message_id.is_none() {
                existing.client_message_id = next.client_message_id.or(client_message_id);
            }
            Some(existing)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(run_id: &str, group_id: &str, text: &str) -> QueuePendingInputRequest {
        QueuePendingInputRequest {
            session_id: "session-1".to_string(),
            run_id: run_id.to_string(),
            merge_group_id: group_id.to_string(),
            text: text.to_string(),
            display_text: text.to_string(),
            images: Vec::new(),
            asset_refs: Vec::new(),
            mode: Some("build".to_string()),
            user_intent: None,
            client_message_id: Some(group_id.to_string()),
            delivery: Some(DELIVERY_AFTER_RUN.to_string()),
        }
    }

    #[test]
    fn queue_merges_by_run_even_with_different_groups() {
        let mut queue = PendingInputQueue::default();
        let first = queue.queue_input(request("run-1", "group-a", "first"));
        let second = queue.queue_input(request("run-1", "group-b", "second"));

        assert_eq!(second.id, first.id);
        assert_eq!(second.merge_group_id, "group-a");
        assert_eq!(second.text, "first\nsecond");
        assert_eq!(queue.list_session("session-1").len(), 1);
    }

    #[test]
    fn claim_removes_until_restored() {
        let mut queue = PendingInputQueue::default();
        let mut request = request("run-1", "group-a", "first");
        request.delivery = Some(DELIVERY_IMMEDIATE.to_string());
        let first = queue.queue_input(request);

        let claimed = queue.claim_immediate("session-1", "run-1");
        assert_eq!(claimed.len(), 1);
        assert_eq!(claimed[0].id, first.id);
        assert!(queue.list_session("session-1").is_empty());

        queue.restore_claimed(claimed);
        assert_eq!(queue.list_session("session-1").len(), 1);
    }

    #[test]
    fn after_run_inputs_are_not_claimed_until_promoted_or_completed() {
        let mut queue = PendingInputQueue::default();
        let first = queue.queue_input(request("run-1", "group-a", "first"));

        assert!(queue.claim_immediate("session-1", "run-1").is_empty());
        let promoted = queue
            .promote_to_immediate("session-1", "run-1", Some(&first.id))
            .expect("promote queued input");
        assert_eq!(promoted.delivery, DELIVERY_IMMEDIATE);
        let claimed = queue.claim_immediate("session-1", "run-1");
        assert_eq!(claimed.len(), 1);
        assert_eq!(claimed[0].id, first.id);
    }

    #[test]
    fn after_run_inputs_can_be_claimed_for_next_turn() {
        let mut queue = PendingInputQueue::default();
        let first = queue.queue_input(request("run-1", "group-a", "first"));

        let claimed = queue
            .claim_after_run("session-1", "run-1")
            .expect("claim after-run input");
        assert_eq!(claimed.id, first.id);
        assert!(queue.list_session("session-1").is_empty());
    }
}
