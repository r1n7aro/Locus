//! Remote compaction reuses the resolved tools and routing state of inference.
use super::*;

impl AgentInstance {
    pub(super) async fn request_codex_compaction(
        &self,
        system_prompt: &str,
        prepared: &[ChatMessage],
        request_tools: &PreparedRequestTools,
        codex_turn_state: &mut codex::TurnState,
        response_request_metadata: &HashMap<String, serde_json::Value>,
        run_id: &str,
        iteration: usize,
    ) -> Result<codex::CodexRemoteCompactOutcome, codex::CodexRemoteCompactError> {
        let api_tools = &request_tools.api_tools;
        let capture = codex::CodexCompactionCapture::default();
        match &self.backend {
            LlmBackend::OpenAiCodex {
                auth,
                transport,
                base_url,
            } => {
                let model_name = &self.effective_model;
                let actual_model = model_name.strip_prefix("openai/").unwrap_or(model_name);
                let (access_token, account_id) =
                    resolve_codex_request_auth(auth, false).await.map_err(|e| {
                        codex::CodexRemoteCompactError::new(
                            format!("OpenAI Codex token failed (please re-login): {e}"),
                            "",
                            "",
                        )
                    })?;
                match codex::compact_conversation_history_v2(
                    &access_token,
                    account_id.as_deref(),
                    *transport,
                    base_url.as_deref(),
                    actual_model,
                    system_prompt,
                    prepared,
                    codex::CodexCompactionContext {
                        tools: api_tools,
                        tool_search_description: request_tools.tool_search_description.as_deref(),
                        turn_state: &mut *codex_turn_state,
                        run_id,
                        iteration,
                        cancel_rx: Some(self.cancel_waiter()),
                        capture: capture.clone(),
                    },
                    self.effort.as_deref(),
                    self.codex_fast_mode,
                    Some(&self.session_id),
                    Some(response_request_metadata),
                    self.debug,
                )
                .await
                {
                    Ok(outcome) => Ok(outcome),
                    Err(error)
                        if !self.is_cancel_requested()
                            && is_codex_unauthorized_error(&error.message) =>
                    {
                        eprintln!(
                            "[OpenAI Codex] compact received unauthorized response, refreshing auth and retrying once"
                        );
                        let (access_token, account_id) =
                            match resolve_codex_request_auth(auth, true).await {
                                Ok(auth) => auth,
                                Err(refresh_error) => {
                                    return Err(codex::CodexRemoteCompactError::new(
                                        format!(
                                            "OpenAI Codex token refresh failed: {refresh_error}"
                                        ),
                                        error.raw_request,
                                        error.raw_response,
                                    ))
                                }
                            };
                        codex::compact_conversation_history_v2(
                            &access_token,
                            account_id.as_deref(),
                            *transport,
                            base_url.as_deref(),
                            actual_model,
                            system_prompt,
                            prepared,
                            codex::CodexCompactionContext {
                                tools: api_tools,
                                tool_search_description: request_tools
                                    .tool_search_description
                                    .as_deref(),
                                turn_state: &mut *codex_turn_state,
                                run_id,
                                iteration,
                                cancel_rx: Some(self.cancel_waiter()),
                                capture: capture.clone(),
                            },
                            self.effort.as_deref(),
                            self.codex_fast_mode,
                            Some(&self.session_id),
                            Some(response_request_metadata),
                            self.debug,
                        )
                        .await
                    }
                    Err(error) => Err(error),
                }
            }
            LlmBackend::Custom {
                api_key,
                api_model,
                endpoint,
                api_format: crate::commands::ApiFormat::OpenaiResponses,
                remote_compaction_mode: crate::commands::RemoteCompactionMode::CodexV2,
                ..
            } => {
                codex::compact_conversation_history_v2(
                    api_key,
                    None,
                    crate::commands::CodexTransportMode::Http,
                    Some(endpoint.as_str()),
                    api_model,
                    system_prompt,
                    prepared,
                    codex::CodexCompactionContext {
                        tools: api_tools,
                        tool_search_description: request_tools.tool_search_description.as_deref(),
                        turn_state: &mut *codex_turn_state,
                        run_id,
                        iteration,
                        cancel_rx: Some(self.cancel_waiter()),
                        capture,
                    },
                    self.effort.as_deref(),
                    false,
                    Some(&self.session_id),
                    Some(response_request_metadata),
                    self.debug,
                )
                .await
            }
            _ => Err(codex::CodexRemoteCompactError::new(
                "Backend does not support Codex remote compaction",
                "",
                "",
            )),
        }
    }
}

#[cfg(test)]
#[path = "compaction_tests.rs"]
mod tests;
