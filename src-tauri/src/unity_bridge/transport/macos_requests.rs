use std::collections::{HashMap, VecDeque};

use tokio::sync::oneshot;

use super::macos_impl::PipeEnvelope;

const RECENT_REQUEST_LIMIT: usize = 1_024;

struct PendingRequest {
    response: oneshot::Sender<Result<PipeEnvelope, String>>,
    acceptance: Option<oneshot::Sender<()>>,
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
            },
        );
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
}
