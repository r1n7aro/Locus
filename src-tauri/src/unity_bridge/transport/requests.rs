use std::collections::{HashMap, HashSet, VecDeque};
use std::time::{Duration, Instant};

use tokio::sync::oneshot;

use super::windows_impl::PipeEnvelope;

const RECENT_REQUEST_LIMIT: usize = 1_024;

struct PendingRequest {
    response: oneshot::Sender<Result<PipeEnvelope, String>>,
    acceptance: Option<oneshot::Sender<()>>,
    sent_at: Instant,
}

/// ACK subscriptions are optional, but every request is tracked. Keep a bounded
/// history as well: a managed response can overtake the broker ACK, and local
/// cancellation or timeout does not retract a frame already sent to Unity.
#[derive(Default)]
pub(super) struct PendingRequests {
    pending: HashMap<String, PendingRequest>,
    recent: VecDeque<String>,
}

impl PendingRequests {
    pub(super) fn insert(
        &mut self,
        id: String,
        response: oneshot::Sender<Result<PipeEnvelope, String>>,
        acceptance: Option<oneshot::Sender<()>>,
    ) {
        self.pending.insert(
            id,
            PendingRequest {
                response,
                acceptance,
                sent_at: Instant::now(),
            },
        );
    }

    /// Requests that have waited at least `age` for their reply.
    pub(super) fn waiting_longer_than(&self, age: Duration) -> Vec<String> {
        let mut ids = self
            .pending
            .iter()
            .filter(|(_, request)| request.sent_at.elapsed() >= age)
            .map(|(id, _)| id.clone())
            .collect::<Vec<_>>();
        ids.sort();
        ids
    }

    pub(super) fn contains(&self, id: &str) -> bool {
        self.pending.contains_key(id)
    }

    /// Fail one request whose reply is known not to come.
    pub(super) fn fail(&mut self, id: &str, reason: &str) -> bool {
        let Some(request) = self.retire(id) else {
            return false;
        };
        drop(request.acceptance);
        let _ = request.response.send(Err(reason.to_string()));
        true
    }

    /// Return whether this ACK belongs to a known request, regardless of whether
    /// its caller subscribed to acceptance or already received an earlier ACK.
    pub(super) fn accept(&mut self, id: &str) -> bool {
        if let Some(request) = self.pending.get_mut(id) {
            if let Some(acceptance) = request.acceptance.take() {
                let _ = acceptance.send(());
            }
            return true;
        }
        self.recent.iter().any(|recent| recent == id)
    }

    fn retire(&mut self, id: &str) -> Option<PendingRequest> {
        let request = self.pending.remove(id)?;
        self.remember(id.to_string());
        Some(request)
    }

    fn remember(&mut self, id: String) {
        if self.recent.len() == RECENT_REQUEST_LIMIT {
            self.recent.pop_front();
        }
        self.recent.push_back(id);
    }

    pub(super) fn remove(&mut self, id: &str) {
        self.retire(id);
    }

    pub(super) fn resolve(&mut self, id: &str, response: Result<PipeEnvelope, String>) -> bool {
        let Some(request) = self.retire(id) else {
            return false;
        };
        // A final response remains authoritative even when it overtakes the
        // ACK. Do not synthesize acceptance for broker-side rejections.
        drop(request.acceptance);
        let _ = request.response.send(response);
        true
    }

    pub(super) fn fail_all(&mut self, reason: &str) {
        for (id, request) in std::mem::take(&mut self.pending) {
            self.remember(id);
            drop(request.acceptance);
            let _ = request.response.send(Err(reason.to_string()));
        }
    }
}

/// Reply-loss detection from the broker's `request_state` answers. The broker
/// writes a connection's replies in order, so when its answer reports a
/// request `none` (no longer queued or running) and the request is still
/// pending after every earlier frame was read, its reply was lost. A request
/// is only declared lost on the second such answer in a row, so a request
/// moving between the broker's queue and its in-flight table, or completing
/// at the moment of the query, is never failed.
#[derive(Default)]
pub(super) struct LostReplies {
    suspects: HashSet<String>,
}

impl LostReplies {
    /// `still_pending`: the queried ids still waiting after the answer was
    /// read. Returns the ids whose reply was lost.
    pub(super) fn observe(
        &mut self,
        still_pending: &[String],
        states: &serde_json::Map<String, serde_json::Value>,
    ) -> Vec<String> {
        let mut lost = Vec::new();
        let mut suspects = HashSet::new();
        for id in still_pending {
            if states.get(id).and_then(serde_json::Value::as_str) != Some("none") {
                continue;
            }
            if self.suspects.contains(id) {
                lost.push(id.clone());
            } else {
                suspects.insert(id.clone());
            }
        }
        self.suspects = suspects;
        lost
    }

    pub(super) fn clear(&mut self) {
        self.suspects.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn response(id: &str) -> PipeEnvelope {
        serde_json::from_value(serde_json::json!({
            "reply_to": id,
            "type": "response",
            "ok": true,
            "message": id,
        }))
        .expect("response envelope")
    }

    #[test]
    fn ordinary_poll_ack_is_known_and_preserves_response() {
        let mut requests = PendingRequests::default();
        let (tx, mut rx) = oneshot::channel();
        requests.insert("poll".to_string(), tx, None);

        assert!(requests.accept("poll"));
        assert!(matches!(
            rx.try_recv(),
            Err(oneshot::error::TryRecvError::Empty)
        ));
        assert!(requests.resolve("poll", Ok(response("poll"))));
        assert_eq!(
            rx.try_recv().unwrap().unwrap().message.as_deref(),
            Some("poll")
        );
    }

    #[test]
    fn subscribed_and_duplicate_acks_preserve_final_response() {
        let mut requests = PendingRequests::default();
        let (tx, mut rx) = oneshot::channel();
        let (acceptance, mut accepted) = oneshot::channel();
        requests.insert("execute".to_string(), tx, Some(acceptance));

        assert!(requests.accept("execute"));
        assert_eq!(accepted.try_recv(), Ok(()));
        assert!(requests.accept("execute"));
        assert!(matches!(
            rx.try_recv(),
            Err(oneshot::error::TryRecvError::Empty)
        ));
        assert!(requests.resolve("execute", Ok(response("execute"))));
        assert!(rx.try_recv().unwrap().unwrap().ok.unwrap());
        assert!(requests.accept("execute"));
    }

    #[test]
    fn response_before_ack_is_delivered_and_late_ack_is_known() {
        let mut requests = PendingRequests::default();
        let (tx, mut rx) = oneshot::channel();
        let (acceptance, mut accepted) = oneshot::channel();
        requests.insert("fast".to_string(), tx, Some(acceptance));

        assert!(requests.resolve("fast", Ok(response("fast"))));
        assert_eq!(
            rx.try_recv().unwrap().unwrap().message.as_deref(),
            Some("fast")
        );
        assert!(matches!(
            accepted.try_recv(),
            Err(oneshot::error::TryRecvError::Closed)
        ));
        assert!(requests.accept("fast"));
    }

    #[test]
    fn cancelled_or_timed_out_request_accepts_late_ack_without_affecting_other_requests() {
        let mut requests = PendingRequests::default();
        let (tx, mut rx) = oneshot::channel();
        let (acceptance, mut accepted) = oneshot::channel();
        requests.insert("cancelled".to_string(), tx, Some(acceptance));
        let (other_tx, mut other_rx) = oneshot::channel();
        requests.insert("other".to_string(), other_tx, None);

        requests.remove("cancelled");
        requests.remove("cancelled");
        assert!(matches!(
            rx.try_recv(),
            Err(oneshot::error::TryRecvError::Closed)
        ));
        assert!(matches!(
            accepted.try_recv(),
            Err(oneshot::error::TryRecvError::Closed)
        ));
        assert!(requests.accept("cancelled"));
        assert!(requests.resolve("other", Ok(response("other"))));
        assert_eq!(
            other_rx.try_recv().unwrap().unwrap().message.as_deref(),
            Some("other")
        );
    }

    #[test]
    fn disconnect_fails_responses_and_closes_acceptance_subscriptions() {
        let mut requests = PendingRequests::default();
        let (tx, mut rx) = oneshot::channel();
        let (acceptance, mut accepted) = oneshot::channel();
        requests.insert("disconnected".to_string(), tx, Some(acceptance));

        requests.fail_all("pipe disconnected");
        assert_eq!(rx.try_recv().unwrap().unwrap_err(), "pipe disconnected");
        assert!(matches!(
            accepted.try_recv(),
            Err(oneshot::error::TryRecvError::Closed)
        ));
        assert!(requests.accept("disconnected"));
        assert!(requests.pending.is_empty());
    }

    #[test]
    fn unknown_ack_does_not_resolve_another_request_or_extend_history() {
        let mut requests = PendingRequests::default();
        let (tx, mut rx) = oneshot::channel();
        let (acceptance, mut accepted) = oneshot::channel();
        requests.insert("known".to_string(), tx, Some(acceptance));

        assert!(!requests.accept("unknown"));
        assert!(!requests.resolve("unknown", Ok(response("unknown"))));
        assert!(requests.recent.is_empty());
        assert!(matches!(
            rx.try_recv(),
            Err(oneshot::error::TryRecvError::Empty)
        ));
        assert!(matches!(
            accepted.try_recv(),
            Err(oneshot::error::TryRecvError::Empty)
        ));
        assert!(requests.accept("known"));
        assert_eq!(accepted.try_recv(), Ok(()));
    }

    #[test]
    fn recent_request_history_is_bounded_and_duplicate_cleanup_does_not_evict_entries() {
        let mut requests = PendingRequests::default();
        for index in 0..=RECENT_REQUEST_LIMIT {
            let id = format!("req-{index}");
            let (tx, _rx) = oneshot::channel();
            requests.insert(id.clone(), tx, None);
            requests.remove(&id);
            requests.remove(&id);
        }

        assert!(requests.pending.is_empty());
        assert_eq!(requests.recent.len(), RECENT_REQUEST_LIMIT);
        assert!(!requests.accept("req-0"));
        assert!(requests.accept("req-1"));
        assert!(requests.accept(&format!("req-{RECENT_REQUEST_LIMIT}")));
    }

    #[test]
    fn a_failed_request_gets_the_reason_and_leaves_the_others_waiting() {
        let mut requests = PendingRequests::default();
        let (lost_tx, mut lost_rx) = oneshot::channel();
        let (kept_tx, mut kept_rx) = oneshot::channel();
        requests.insert("lost".to_string(), lost_tx, None);
        requests.insert("kept".to_string(), kept_tx, None);
        assert_eq!(
            requests.waiting_longer_than(Duration::ZERO),
            ["kept", "lost"]
        );
        assert!(requests
            .waiting_longer_than(Duration::from_secs(3600))
            .is_empty());

        assert!(requests.fail("lost", "Unity response lost"));
        assert_eq!(
            lost_rx.try_recv().unwrap().unwrap_err(),
            "Unity response lost"
        );
        assert!(!requests.fail("lost", "again"));
        assert!(requests.contains("kept"));
        assert!(matches!(
            kept_rx.try_recv(),
            Err(oneshot::error::TryRecvError::Empty)
        ));
    }

    fn states(value: serde_json::Value) -> serde_json::Map<String, serde_json::Value> {
        value.as_object().unwrap().clone()
    }

    #[test]
    fn a_reply_is_lost_only_after_two_answers_in_a_row_say_none() {
        let mut lost = LostReplies::default();
        let pending = ["a".to_string(), "b".to_string(), "c".to_string()];
        let first = states(serde_json::json!({"a": "none", "b": "inflight", "c": "none"}));
        assert!(lost.observe(&pending, &first).is_empty());

        // `c` was answered meanwhile; `b` finished just now: only `a` is lost.
        let second = states(serde_json::json!({"a": "none", "b": "none"}));
        assert_eq!(lost.observe(&pending[..2], &second), ["a"]);
        // `b` was reported `none` once; a queued answer resets it.
        let third = states(serde_json::json!({"b": "queued"}));
        assert!(lost.observe(&pending[1..2], &third).is_empty());
        assert!(lost.observe(&pending[1..2], &second).is_empty());
        lost.clear();
        assert!(lost.observe(&pending[1..2], &second).is_empty());
    }
}
