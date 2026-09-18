use super::*;
use codex::steering::{SteeringInput, SteeringSource};

struct ClaimedInput {
    input: PendingSessionInput,
    suffix: Option<String>,
    // None is connection-owned or awaiting an acknowledgement. The bool
    // places committed input before or after the returned provider response.
    ready: Option<bool>,
}

pub(super) struct RunSteering<'a> {
    instance: &'a AgentInstance,
    app: &'a AppHandle,
    store: &'a SessionStore,
    run_id: &'a str,
    env_prefix: Option<&'a str>,
    queue: crate::PendingInputQueueHandle,
    claimed: Mutex<Vec<ClaimedInput>>,
}

impl<'a> RunSteering<'a> {
    pub fn new(
        instance: &'a AgentInstance,
        app: &'a AppHandle,
        store: &'a SessionStore,
        run_id: &'a str,
        env_prefix: Option<&'a str>,
    ) -> Self {
        Self {
            instance,
            app,
            store,
            run_id,
            env_prefix,
            queue: app
                .state::<crate::PendingInputQueueHandle>()
                .inner()
                .clone(),
            claimed: Mutex::new(Vec::new()),
        }
    }

    pub fn persist_ready(&self, before_response: bool) -> Result<(), String> {
        let mut claimed = self
            .claimed
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        while let Some(index) = claimed
            .iter()
            .position(|item| item.ready == Some(before_response))
        {
            let item = &claimed[index];
            self.instance.persist_pending_input(
                self.app,
                self.store,
                self.run_id,
                self.env_prefix,
                item.input.clone(),
                Some(item.suffix.clone()),
            )?;
            self.queue
                .lock()
                .map_err(|e| e.to_string())?
                .finish_steering(&item.input.id);
            claimed.remove(index);
        }
        Ok(())
    }
}

impl SteeringSource for RunSteering<'_> {
    fn claim(&self) -> Result<Option<SteeringInput>, String> {
        let input = {
            let mut queue = self.queue.lock().map_err(|e| e.to_string())?;
            // Switching into Plan changes tool permissions and request settings.
            // It must use the ordinary boundary where those changes are enforced.
            let changes_mode = queue
                .list_session(&self.instance.session_id)
                .iter()
                .any(|input| {
                    input.run_id == self.run_id
                        && AgentInstance::pending_input_mode(input) == "plan"
                        && self.instance.plan_runtime_snapshot().is_none()
                });
            if changes_mode {
                return Ok(None);
            }
            queue.claim_for_steering(&self.instance.session_id, self.run_id)
        };
        let Some(input) = input else {
            return Ok(None);
        };
        let suffix = self.instance.build_user_prompt_suffix(
            self.app,
            self.store,
            input.user_intent.as_ref(),
            &input.text,
        );
        let wire = codex::steering::user_input(
            &format!("{}{}", input.text, suffix.as_deref().unwrap_or_default()),
            input.images.as_deref(),
        );
        let result = SteeringInput {
            id: input.id.clone(),
            input: wire,
        };
        emit_stream(
            self.app,
            self.run_id,
            StreamEvent::PendingInputQueued {
                session_id: self.instance.session_id.clone(),
                input: input.clone(),
            },
        );
        self.claimed
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(ClaimedInput {
                input,
                suffix,
                ready: None,
            });
        Ok(Some(result))
    }

    fn committed(&self, id: &str, before_response: bool) -> Result<(), String> {
        if let Some(item) = self
            .claimed
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .iter_mut()
            .find(|item| item.input.id == id)
        {
            item.ready = Some(before_response);
        }
        // An explicit tool continuation starts after its parent and tool results
        // are already saved. Commit before emitting any successor text deltas.
        if before_response {
            self.persist_ready(true)?;
        }
        Ok(())
    }

    fn rejected(&self, id: &str) {
        let _ = self.committed(id, false);
    }

    fn has_unresolved_input(&self) -> bool {
        !self
            .claimed
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .is_empty()
    }
}

impl Drop for RunSteering<'_> {
    fn drop(&mut self) {
        // Cancellation and unknown outcomes return drafts for user recovery. They
        // stay immediate, so the after-run scheduler cannot silently resend them.
        let inputs = std::mem::take(
            self.claimed
                .get_mut()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
        )
        .into_iter()
        .rev()
        .map(|item| item.input)
        .collect();
        if let Ok(mut queue) = self.queue.lock() {
            queue.restore_claimed(inputs);
        }
    }
}
