//! Renderer-owned interests. Shared/teleported panes register in their owning
//! renderer, not in the blank native window that displays their DOM.
use std::collections::{HashMap, HashSet};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use super::RoutedWorkspaceEvent;
use crate::workspace_service::WorkspaceRef;

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WindowEventInterests {
    #[serde(default)]
    pub checkouts: Vec<WorkspaceRef>,
    #[serde(default)]
    pub sessions: HashSet<String>,
    /// Checkout-wide stream consumers, including a launch before its durable id is known.
    #[serde(default)]
    pub stream_checkouts: Vec<WorkspaceRef>,
}

#[derive(Default)]
pub struct WindowEventInterestRegistry(Mutex<HashMap<String, WindowEventInterests>>);

/// Per-token stream content. Everything else, including `usageUpdate` (one per
/// model response, and the carrier of prompt-cache warnings for sessions no
/// window is showing), stays a broadcast.
pub fn is_high_frequency_stream_type(kind: &str) -> bool {
    matches!(
        kind,
        "textDelta" | "thinkingDelta" | "toolCallDelta" | "toolCallProgress"
    )
}

/// Mirrors the frontend's `workspaceMaterializationMatches`: a missing event
/// epoch counts as 0, and a ref without an epoch accepts epochs 0 and 1.
fn matches_checkout(scope: &WorkspaceRef, event: &RoutedWorkspaceEvent) -> bool {
    let epoch = event.envelope.materialization_epoch.unwrap_or(0);
    scope.checkout_id == event.envelope.checkout_id
        && scope
            .expected_generation
            .is_none_or(|generation| generation == event.envelope.workspace_generation)
        && match scope.expected_materialization_epoch {
            Some(expected) => epoch == expected,
            None => epoch <= 1,
        }
}

impl WindowEventInterests {
    /// Delivery filter for remote (mobile) connections, which unlike desktop
    /// windows receive nothing by default: high-frequency stream events follow
    /// the same session/stream-checkout rule as `targets`, file-change
    /// notifications never pass through interests (connections forward them
    /// for watched paths only, see `remote_access::connections`), and every
    /// other event needs a matching checkout or session.
    pub fn matches_remote(&self, event: &RoutedWorkspaceEvent) -> bool {
        if event.event_name == "workspace-file-changed" {
            return false;
        }
        let session = event
            .envelope
            .payload
            .get("sessionId")
            .and_then(|value| value.as_str());
        let session_matches = session.is_some_and(|session| self.sessions.contains(session));
        let stream_checkout_matches = self
            .stream_checkouts
            .iter()
            .any(|scope| matches_checkout(scope, event));
        let high_frequency = event.event_name == "stream-event"
            && event
                .envelope
                .payload
                .get("type")
                .and_then(|value| value.as_str())
                .is_some_and(is_high_frequency_stream_type);
        if high_frequency {
            return session_matches || stream_checkout_matches;
        }
        session_matches
            || stream_checkout_matches
            || self.checkouts.iter().any(|scope| matches_checkout(scope, event))
    }
}

impl WindowEventInterestRegistry {
    pub fn replace(&self, window: &str, interests: WindowEventInterests) {
        let mut windows = self.0.lock().unwrap_or_else(|error| error.into_inner());
        if interests.checkouts.is_empty()
            && interests.sessions.is_empty()
            && interests.stream_checkouts.is_empty()
        {
            windows.remove(window);
        } else {
            windows.insert(window.to_owned(), interests);
        }
    }

    pub fn remove(&self, window: &str) {
        self.0
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .remove(window);
    }

    /// None means a low-frequency broadcast; Some(empty) means no renderer needs
    /// this high-frequency event. Internal router subscribers are never filtered.
    pub fn targets(&self, event: &RoutedWorkspaceEvent) -> Option<Vec<String>> {
        let stream = event.event_name == "stream-event"
            && event
                .envelope
                .payload
                .get("type")
                .and_then(|value| value.as_str())
                .is_some_and(is_high_frequency_stream_type);
        if !stream && event.event_name != "workspace-file-changed" {
            return None;
        }
        let session = event
            .envelope
            .payload
            .get("sessionId")
            .and_then(|value| value.as_str());
        let windows = self.0.lock().unwrap_or_else(|error| error.into_inner());
        Some(
            windows
                .iter()
                .filter(|(_, interest)| {
                    if stream {
                        session.is_some_and(|session| interest.sessions.contains(session))
                            || interest
                                .stream_checkouts
                                .iter()
                                .any(|scope| matches_checkout(scope, event))
                    } else {
                        interest
                            .checkouts
                            .iter()
                            .any(|scope| matches_checkout(scope, event))
                    }
                })
                .map(|(window, _)| window.clone())
                .collect(),
        )
    }

    pub fn legacy_stream_targets(&self, session: &str) -> Vec<String> {
        self.0
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .iter()
            .filter(|(_, interest)| interest.sessions.contains(session))
            .map(|(window, _)| window.clone())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workspace_service::event::WorkspaceEventEnvelope;
    use crate::workspace_service::{CheckoutId, ProjectId};
    use serde_json::json;

    fn scope(checkout: &str) -> WorkspaceRef {
        WorkspaceRef::new(CheckoutId::new(checkout).unwrap(), Some(7))
            .with_materialization_epoch(Some(2))
    }

    fn event(name: &str, checkout: &str, payload: serde_json::Value) -> RoutedWorkspaceEvent {
        RoutedWorkspaceEvent {
            event_name: name.into(),
            stream_revision: 1,
            envelope: WorkspaceEventEnvelope {
                project_id: ProjectId::new("project").unwrap(),
                checkout_id: CheckoutId::new(checkout).unwrap(),
                workspace_generation: 7,
                materialization_epoch: Some(2),
                service_instance_id: None,
                service_generation: None,
                payload,
            },
        }
    }

    #[test]
    fn remote_filter_requires_explicit_interest() {
        let none = WindowEventInterests::default();
        assert!(!none.matches_remote(&event("stream-event", "a", json!({"type":"done","sessionId":"s"}))));

        let session_only = WindowEventInterests {
            sessions: ["s".to_string()].into_iter().collect(),
            ..Default::default()
        };
        assert!(session_only.matches_remote(&event(
            "stream-event",
            "a",
            json!({"type":"textDelta","sessionId":"s"})
        )));
        assert!(!session_only.matches_remote(&event(
            "stream-event",
            "a",
            json!({"type":"textDelta","sessionId":"other"})
        )));

        let checkout = WindowEventInterests { checkouts: vec![scope("a")], ..Default::default() };
        assert!(checkout.matches_remote(&event(
            "session-title-updated",
            "a",
            json!({"sessionId":"x"})
        )));
        assert!(checkout.matches_remote(&event("stream-event", "a", json!({"type":"done","sessionId":"x"}))));
        // Deltas need the session itself (or a stream checkout), not just the checkout.
        assert!(!checkout.matches_remote(&event(
            "stream-event",
            "a",
            json!({"type":"textDelta","sessionId":"x"})
        )));
        assert!(!checkout.matches_remote(&event("session-title-updated", "b", json!({}))));
        assert!(!checkout.matches_remote(&event("workspace-file-changed", "a", json!({}))));
    }

    #[test]
    fn high_frequency_allowlist_does_not_change_lifecycle_broadcasts() {
        let interests = WindowEventInterestRegistry::default();
        for kind in [
            "textDelta",
            "thinkingDelta",
            "toolCallDelta",
            "toolCallProgress",
        ] {
            assert_eq!(
                interests.targets(&event(
                    "stream-event",
                    "a",
                    json!({"type":kind,"sessionId":"s"})
                )),
                Some(vec![])
            );
        }
        for kind in [
            "runStart",
            "done",
            "error",
            "cancelled",
            "toolCallStart",
            "toolCallDone",
            "askUser",
            // Prompt-cache warnings must reach windows that are not showing the session.
            "usageUpdate",
        ] {
            assert_eq!(
                interests.targets(&event(
                    "stream-event",
                    "a",
                    json!({"type":kind,"sessionId":"s"})
                )),
                None
            );
        }
        assert_eq!(
            interests.targets(&event("plugins-changed", "a", json!({}))),
            None
        );
    }

    #[test]
    fn session_streams_only_reach_their_consumers_not_every_window_in_the_checkout() {
        let interests = WindowEventInterestRegistry::default();
        interests.replace(
            "main",
            WindowEventInterests {
                sessions: ["s".into()].into(),
                ..Default::default()
            },
        );
        interests.replace(
            "other",
            WindowEventInterests {
                checkouts: vec![scope("a")],
                sessions: ["other-session".into()].into(),
                ..Default::default()
            },
        );
        assert_eq!(
            interests.targets(&event(
                "stream-event",
                "a",
                json!({"type":"textDelta","sessionId":"s"})
            )),
            Some(vec!["main".into()])
        );
        assert_eq!(interests.legacy_stream_targets("s"), vec!["main"]);
    }

    #[test]
    fn file_events_match_checkout_generation_and_materialization() {
        let interests = WindowEventInterestRegistry::default();
        interests.replace(
            "owner",
            WindowEventInterests {
                checkouts: vec![scope("a")],
                ..Default::default()
            },
        );
        let mut change = event("workspace-file-changed", "a", json!({"path":"file.md"}));
        assert_eq!(interests.targets(&change), Some(vec!["owner".into()]));
        change.envelope.workspace_generation += 1;
        assert_eq!(interests.targets(&change), Some(vec![]));
        change.envelope.workspace_generation = 7;
        change.envelope.materialization_epoch = Some(3);
        assert_eq!(interests.targets(&change), Some(vec![]));
        assert_eq!(
            interests.targets(&event("workspace-file-changed", "b", json!({}))),
            Some(vec![])
        );
    }

    #[test]
    fn epoch_matching_agrees_with_the_frontend_rule() {
        let interests = WindowEventInterestRegistry::default();
        let mut explicit_zero = scope("a");
        explicit_zero.expected_materialization_epoch = Some(0);
        let mut unscoped = scope("a");
        unscoped.expected_materialization_epoch = None;
        interests.replace(
            "zero",
            WindowEventInterests {
                checkouts: vec![explicit_zero],
                ..Default::default()
            },
        );
        interests.replace(
            "unscoped",
            WindowEventInterests {
                checkouts: vec![unscoped],
                ..Default::default()
            },
        );
        let mut change = event("workspace-file-changed", "a", json!({"path":"file.md"}));
        // A missing epoch is epoch 0 for both an explicit 0 and an unscoped ref.
        change.envelope.materialization_epoch = None;
        let mut targets = interests.targets(&change).unwrap();
        targets.sort();
        assert_eq!(targets, vec!["unscoped".to_string(), "zero".to_string()]);
        change.envelope.materialization_epoch = Some(1);
        assert_eq!(interests.targets(&change), Some(vec!["unscoped".into()]));
        change.envelope.materialization_epoch = Some(2);
        assert_eq!(interests.targets(&change), Some(vec![]));
    }

    #[test]
    fn launch_interest_can_be_replaced_atomically_by_the_durable_session() {
        let interests = WindowEventInterestRegistry::default();
        interests.replace(
            "owner",
            WindowEventInterests {
                stream_checkouts: vec![scope("a")],
                ..Default::default()
            },
        );
        let delta = event(
            "stream-event",
            "a",
            json!({"type":"thinkingDelta","sessionId":"new"}),
        );
        assert_eq!(interests.targets(&delta), Some(vec!["owner".into()]));
        assert_eq!(
            interests.targets(&event("stream-event", "b", delta.envelope.payload.clone())),
            Some(vec![])
        );
        interests.replace(
            "owner",
            WindowEventInterests {
                sessions: ["new".into()].into(),
                ..Default::default()
            },
        );
        assert_eq!(interests.targets(&delta), Some(vec!["owner".into()]));
        assert_eq!(
            interests.targets(&event(
                "stream-event",
                "a",
                json!({"type":"textDelta","sessionId":"unrelated"})
            )),
            Some(vec![])
        );
        interests.remove("owner");
        assert_eq!(interests.targets(&delta), Some(vec![]));
        assert!(interests.legacy_stream_targets("new").is_empty());
    }

    #[test]
    fn zero_targets_does_not_drop_internal_subscribers_or_revision_order() {
        let router = crate::workspace_service::event::WorkspaceEventRouter::new();
        let mut receiver = router.subscribe();
        let delta = event(
            "stream-event",
            "a",
            json!({"type":"textDelta","sessionId":"s"}),
        );
        router.publish_routed_with(delta.clone(), || {}, |event| {
            assert_eq!(router.window_interests.targets(event), Some(vec![]));
            Ok(())
        });
        router.publish_routed_with(delta, || {}, |_| Ok(()));
        assert_eq!(receiver.try_recv().unwrap().stream_revision, 1);
        assert_eq!(receiver.try_recv().unwrap().stream_revision, 2);
    }
}
