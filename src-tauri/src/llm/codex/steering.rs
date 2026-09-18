//! Connection-local Responses steering. Each provider response remains a separate
//! Locus round so output, tool execution, history and usage retain their ordering.
use super::*;
use serde_json::{json, Value};

#[cfg(test)]
mod tests;

#[derive(Clone, Debug)]
pub struct SteeringInput {
    pub id: String,
    pub input: Value,
}

pub trait SteeringSource: Send + Sync {
    fn claim(&self) -> Result<Option<SteeringInput>, String>;
    fn committed(&self, id: &str, before_response: bool) -> Result<(), String>;
    fn rejected(&self, id: &str);
    fn has_unresolved_input(&self) -> bool;
}

pub fn supports_steering(model: &str) -> bool {
    let model = model.strip_prefix("openai/").unwrap_or(model);
    model == "gpt-6-astra" || model.starts_with("gpt-6-astra-")
}

pub fn user_input(text: &str, images: Option<&[ImageData]>) -> Value {
    let mut content = build_user_input_content(text, images);
    // The Codex Astra lite protocol omits image detail, just like normal input.
    for part in &mut content {
        if part["type"] == "input_image" {
            part.as_object_mut().unwrap().remove("detail");
        }
    }
    json!([{"role":"user", "content":content}])
}

pub(super) struct Continuation {
    pub socket: CodexWebsocketStream,
    pub parent: LastWebsocketResponse,
    pub request: Value,
    pub input: SteeringInput,
    pub accepted_id: String,
    pub created: Option<Value>,
    pub connection_key: String,
}

impl fmt::Debug for Continuation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SteeringContinuation")
            .field("parent", &self.parent.response_id)
            .field("automatic", &self.created.is_some())
            .finish_non_exhaustive()
    }
}

#[derive(Default)]
pub(super) struct Submission {
    pub input: Option<SteeringInput>,
    pub parent_id: Option<String>,
    pub accepted_id: Option<String>,
    pub rejected: bool,
}

impl Submission {
    pub fn send(&mut self, input: SteeringInput, parent_id: &str) -> Value {
        let event = json!({"type":"response.steer", "previous_response_id":parent_id,
            "input":input.input});
        self.input = Some(input);
        self.parent_id = Some(parent_id.to_string());
        event
    }

    pub fn pending(&self) -> bool {
        self.input.is_some() && !self.rejected
    }

    pub fn event(
        &mut self,
        event: &Value,
        source: Option<&dyn SteeringSource>,
    ) -> Result<bool, String> {
        let kind = event["type"].as_str().unwrap_or_default();
        if !kind.starts_with("response.steer.") {
            return Ok(false);
        }
        let Some(input) = self.input.as_ref() else {
            return Err("Codex steering received an unexpected receipt".to_string());
        };
        if event["steer"]["previous_response_id"].as_str() != self.parent_id.as_deref() {
            return Err("Codex steering receipt targets a different response".to_string());
        }
        match kind {
            "response.steer.accepted" => {
                self.accepted_id = Some(
                    event["steer"]["id"]
                        .as_str()
                        .filter(|id| !id.is_empty())
                        .ok_or("Codex steering acceptance has no ID")?
                        .to_string(),
                );
            }
            "response.steer.failed" => {
                if self.accepted_id.is_some()
                    && event["steer"]["id"].as_str() != self.accepted_id.as_deref()
                {
                    return Err("Codex steering failure has a different ID".to_string());
                }
                self.rejected = true;
                if let Some(source) = source {
                    source.rejected(&input.id);
                }
                tracing::info!(
                    code = event["error"]["code"].as_str(),
                    "Codex steering rejected; using the next response"
                );
            }
            "response.steer.pending" => {
                if event["steer"]["id"].as_str() != self.accepted_id.as_deref()
                    || event["reason"] != "waiting_for_required_input"
                {
                    return Err(
                        "Codex steering returned an unsupported pending receipt".to_string()
                    );
                }
            }
            _ => return Err(format!("Codex steering returned an unknown event: {kind}")),
        }
        Ok(true)
    }
}
