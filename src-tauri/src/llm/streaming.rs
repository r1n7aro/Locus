use aisdk::core::messages::MessageBuilder;
use aisdk::core::{DynamicModel, LanguageModelRequest, LanguageModelStreamChunkType};
use aisdk::providers::Openrouter;
use futures::StreamExt;

use crate::session::models::{ChatMessage, MessageRole};

#[allow(dead_code)]
pub async fn stream_llm<F>(
    api_key: &str,
    model: &str,
    system_prompt: &str,
    history: &[ChatMessage],
    on_delta: F,
) -> Result<String, String>
where
    F: Fn(String) + Send + 'static,
{
    let openrouter = Openrouter::<DynamicModel>::builder()
        .model_name(model)
        .api_key(api_key)
        .build()
        .map_err(|e| format!("OpenRouter config error: {}", e))?;

    let messages = build_messages(system_prompt, history)?;

    let mut response = LanguageModelRequest::builder()
        .model(openrouter)
        .messages(messages)
        .build()
        .stream_text()
        .await
        .map_err(|e| format!("Stream request failed: {}", e))?;

    let mut full_text = String::new();

    while let Some(chunk) = response.stream.next().await {
        match chunk {
            LanguageModelStreamChunkType::Text(delta) => {
                full_text.push_str(&delta);
                on_delta(delta);
            }
            LanguageModelStreamChunkType::End(_) => {
                break;
            }
            LanguageModelStreamChunkType::Failed(err) => {
                return Err(format!("LLM generation failed: {}", err));
            }
            LanguageModelStreamChunkType::Incomplete(msg) => {
                return Err(format!("LLM response incomplete: {}", msg));
            }
            _ => {}
        }
    }

    Ok(full_text)
}

#[allow(dead_code)]
fn build_messages(
    system_prompt: &str,
    history: &[ChatMessage],
) -> Result<aisdk::core::messages::Messages, String> {
    if history.is_empty() {
        return Err("No messages to send".to_string());
    }

    let first = &history[0];
    if first.role != MessageRole::User {
        return Err("First message must be from user".to_string());
    }

    let mut builder = MessageBuilder::default()
        .system(system_prompt)
        .user(first.content.clone());

    for msg in &history[1..] {
        match msg.role {
            MessageRole::Assistant => {
                builder = builder.assistant(msg.content.clone());
            }
            MessageRole::User => {
                builder = builder.user(msg.content.clone());
            }
            MessageRole::Tool => {
                continue;
            }
        }
    }

    Ok(builder.build())
}
