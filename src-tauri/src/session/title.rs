use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use html2md::{Handle, StructuredPrinter, TagHandler, TagHandlerFactory};
use pulldown_cmark::{Event, Options, Parser, TagEnd};
use regex::Regex;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter};

use crate::auth::codex::CodexAuthState;
use crate::commands::CodexTransportMode;
use crate::llm::codex::{self, CodexStreamOptions, TurnState};
use crate::session::models::{ChatMessage, MessageRole};
use crate::session::store::SessionStore;

pub const DEFAULT_CODEX_TITLE_MODEL: &str = "gpt-5.6-luna";
pub const DEFAULT_CODEX_TITLE_REASONING_EFFORT: &str = "low";
pub const SESSION_TITLE_UPDATED_EVENT: &str = "session-title-updated";

const TITLE_PROMPT_CHAR_LIMIT: usize = 2_000;
const GENERATED_TITLE_CHAR_LIMIT: usize = 36;
const FALLBACK_TITLE_CHAR_LIMIT: usize = 60;
const TITLE_GENERATION_TIMEOUT: Duration = Duration::from_secs(30);

const TITLE_SYSTEM_PROMPT: &str = r#"You generate concise UI titles for software-development tasks.

Fill the structured title field from the supplied user prompt.
- Capture the question or core change requested in one line.
- Keep the title under 36 characters and under 5 words where possible.
- Use an imperative verb first for requested changes and a precise action verb for questions.
- Reuse an already short, clear title when the user supplied one.
- Write in the user's locale and preserve code terms and ticket references verbatim.
- Do not include quotes, Markdown, formatting characters, or trailing punctuation.
- Do not answer the prompt or perform the task; only fill the title field."#;

#[derive(Debug, Clone)]
pub struct SessionTitleGenerationRequest {
    pub prompt: String,
    pub model: String,
    pub reasoning_effort: Option<String>,
}

impl SessionTitleGenerationRequest {
    pub fn codex_default(prompt: String) -> Self {
        Self {
            prompt,
            model: DEFAULT_CODEX_TITLE_MODEL.to_string(),
            reasoning_effort: Some(DEFAULT_CODEX_TITLE_REASONING_EFFORT.to_string()),
        }
    }
}

struct SessionTitleGenerationOutcome {
    title: Option<String>,
    model: String,
    input_tokens: u32,
    output_tokens: u32,
    cache_read_tokens: u32,
    cache_write_tokens: u32,
    cost_usd: f64,
}

#[derive(Debug, Deserialize)]
struct SessionTitleOutput {
    title: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionTitleUpdatedEvent {
    pub session_id: String,
    pub title: String,
}

pub fn prepare_session_title_prompt(raw_prompt: &str) -> Option<String> {
    let unwrapped = unwrap_codex_delegations(raw_prompt);
    let limited = take_chars(&unwrapped, TITLE_PROMPT_CHAR_LIMIT);
    let without_writing_fences = strip_writing_block_fences(&limited);
    let plain_text = markdown_to_plain_text(&without_writing_fences);
    let normalized = normalize_whitespace(&plain_text);
    if normalized.is_empty() {
        None
    } else {
        Some(normalized)
    }
}

pub fn fallback_session_title(prepared_prompt: &str) -> Option<String> {
    let normalized = normalize_whitespace(prepared_prompt);
    if normalized.is_empty() {
        None
    } else {
        Some(truncate_with_ellipsis(
            &normalized,
            FALLBACK_TITLE_CHAR_LIMIT,
        ))
    }
}

pub fn spawn_codex_session_title_generation(
    app_handle: AppHandle,
    store: Arc<SessionStore>,
    auth: Arc<tokio::sync::Mutex<CodexAuthState>>,
    transport: CodexTransportMode,
    base_url: Option<String>,
    session_id: String,
    expected_title: String,
    request: SessionTitleGenerationRequest,
    debug: bool,
) {
    tauri::async_runtime::spawn(async move {
        let generated = generate_codex_session_title_outcome(
            auth,
            transport,
            base_url.as_deref(),
            request,
            debug,
        )
        .await;

        let outcome = match generated {
            Ok(Some(outcome)) => outcome,
            Ok(None) => return,
            Err(error) => {
                eprintln!(
                    "[SessionTitle] generation failed for session {}: {}",
                    session_id, error
                );
                return;
            }
        };

        if outcome.input_tokens > 0
            || outcome.output_tokens > 0
            || outcome.cache_read_tokens > 0
            || outcome.cache_write_tokens > 0
        {
            if let Err(error) = store.record_model_usage_event(
                &session_id,
                &outcome.model,
                "OpenAI Codex",
                "session_title",
                outcome.input_tokens as u64,
                outcome.output_tokens as u64,
                outcome.cache_read_tokens as u64,
                outcome.cache_write_tokens as u64,
                outcome.cost_usd,
            ) {
                eprintln!(
                    "[SessionTitle] failed to record usage for session {}: {}",
                    session_id, error
                );
            }
        }

        let Some(title) = outcome.title else {
            return;
        };

        match store.rename_session_if_title_matches(&session_id, &expected_title, &title) {
            Ok(true) => {
                let event = SessionTitleUpdatedEvent {
                    session_id: session_id.clone(),
                    title,
                };
                if let Err(error) = app_handle.emit(SESSION_TITLE_UPDATED_EVENT, event) {
                    eprintln!(
                        "[SessionTitle] failed to emit title update for session {}: {}",
                        session_id, error
                    );
                }
            }
            Ok(false) => {}
            Err(error) => eprintln!(
                "[SessionTitle] failed to update session {}: {}",
                session_id, error
            ),
        }
    });
}

pub async fn generate_codex_session_title(
    auth: Arc<tokio::sync::Mutex<CodexAuthState>>,
    transport: CodexTransportMode,
    base_url: Option<&str>,
    request: SessionTitleGenerationRequest,
    debug: bool,
) -> Result<Option<String>, String> {
    Ok(
        generate_codex_session_title_outcome(auth, transport, base_url, request, debug)
            .await?
            .and_then(|outcome| outcome.title),
    )
}

async fn generate_codex_session_title_outcome(
    auth: Arc<tokio::sync::Mutex<CodexAuthState>>,
    transport: CodexTransportMode,
    base_url: Option<&str>,
    request: SessionTitleGenerationRequest,
    debug: bool,
) -> Result<Option<SessionTitleGenerationOutcome>, String> {
    if request.prompt.trim().is_empty() || request.model.trim().is_empty() {
        return Ok(None);
    }

    let status = auth.lock().await.status();
    if !status.authenticated || status.validation_failed {
        return Ok(None);
    }

    match tokio::time::timeout(
        TITLE_GENERATION_TIMEOUT,
        generate_codex_session_title_inner(auth, transport, base_url, &request, debug),
    )
    .await
    {
        Ok(result) => result,
        Err(_) => Err("Timed out waiting for structured title result".to_string()),
    }
}

async fn generate_codex_session_title_inner(
    auth: Arc<tokio::sync::Mutex<CodexAuthState>>,
    transport: CodexTransportMode,
    base_url: Option<&str>,
    request: &SessionTitleGenerationRequest,
    debug: bool,
) -> Result<Option<SessionTitleGenerationOutcome>, String> {
    let model = request
        .model
        .trim()
        .strip_prefix("openai/")
        .unwrap_or(request.model.trim());
    let first_auth = resolve_codex_title_auth(&auth, false).await?;
    let first = request_codex_session_title(
        &first_auth.0,
        first_auth.1.as_deref(),
        transport,
        base_url,
        model,
        request.reasoning_effort.as_deref(),
        &request.prompt,
        debug,
    )
    .await;

    let response = match first {
        Err(error) if is_codex_unauthorized_error(&error) => {
            let refreshed_auth = resolve_codex_title_auth(&auth, true).await?;
            request_codex_session_title(
                &refreshed_auth.0,
                refreshed_auth.1.as_deref(),
                transport,
                base_url,
                model,
                request.reasoning_effort.as_deref(),
                &request.prompt,
                debug,
            )
            .await?
        }
        result => result?,
    };

    let parsed: SessionTitleOutput = serde_json::from_str(response.text.trim())
        .map_err(|error| format!("Invalid structured title response: {}", error))?;
    Ok(Some(SessionTitleGenerationOutcome {
        title: normalize_generated_title(&parsed.title),
        model: request.model.clone(),
        input_tokens: response.input_tokens,
        output_tokens: response.output_tokens,
        cache_read_tokens: response.cache_read_tokens,
        cache_write_tokens: response.cache_write_tokens,
        cost_usd: response.cost_usd,
    }))
}

async fn resolve_codex_title_auth(
    auth: &Arc<tokio::sync::Mutex<CodexAuthState>>,
    force_refresh: bool,
) -> Result<(String, Option<String>), String> {
    let mut guard = auth.lock().await;
    if force_refresh {
        guard.retry_validation().await?;
    }
    let access_token = guard.access_token().await?;
    Ok((access_token, guard.account_id()))
}

#[allow(clippy::too_many_arguments)]
async fn request_codex_session_title(
    access_token: &str,
    account_id: Option<&str>,
    transport: CodexTransportMode,
    base_url: Option<&str>,
    model: &str,
    reasoning_effort: Option<&str>,
    prompt: &str,
    debug: bool,
) -> Result<crate::llm::openrouter::LlmResponse, String> {
    let history = vec![ChatMessage {
        id: uuid::Uuid::new_v4().to_string(),
        role: MessageRole::User,
        content: format!("User prompt:\n{}", prompt),
        created_at: chrono::Utc::now().timestamp(),
        prompt_prefix: None,
        prompt_suffix: None,
        response_id: None,
        content_order: None,
        thinking_order: None,
        tool_calls: None,
        tool_call_id: None,
        images: None,
        asset_refs: None,
        thinking_content: None,
        thinking_duration: None,
        thinking_signature: None,
        knowledge_proposal: None,
        render_parts: None,
    }];
    let schema = serde_json::json!({
        "type": "object",
        "properties": {
            "title": {
                "type": "string",
                "minLength": 1,
                "maxLength": GENERATED_TITLE_CHAR_LIMIT
            }
        },
        "required": ["title"],
        "additionalProperties": false
    });
    let mut turn_state = TurnState::default();

    codex::stream_chat_with_options(
        access_token,
        account_id,
        transport,
        base_url,
        model,
        TITLE_SYSTEM_PROMPT,
        &history,
        &[],
        None,
        reasoning_effort,
        debug,
        None,
        None,
        &mut turn_state,
        CodexStreamOptions::compact().with_output_schema("session_title", schema),
        &|_| {},
        &|_| {},
        &|_, _| {},
    )
    .await
}

fn is_codex_unauthorized_error(error: &str) -> bool {
    let lower = error.to_ascii_lowercase();
    lower.contains("401 unauthorized")
        || lower.contains("http error: 401")
        || lower.contains("http 401")
        || lower.contains("api error (401")
}

fn normalize_generated_title(value: &str) -> Option<String> {
    let first_line = value.lines().find(|line| !line.trim().is_empty())?.trim();
    let plain = markdown_to_plain_text(first_line);
    let first_line = plain.trim();
    let without_prefix = strip_ascii_case_prefix(first_line, "title:")
        .unwrap_or(first_line)
        .trim();
    let unquoted = without_prefix
        .trim_matches(|character| {
            matches!(
                character,
                '`' | '"' | '\'' | '\u{201c}' | '\u{201d}' | '\u{2018}' | '\u{2019}'
            )
        })
        .trim();
    let normalized = normalize_whitespace(unquoted)
        .trim_end_matches(['.', '?', '!', '\u{3002}', '\u{ff1f}', '\u{ff01}'])
        .trim()
        .to_string();
    if normalized.is_empty() {
        None
    } else {
        Some(truncate_with_ellipsis(
            &normalized,
            GENERATED_TITLE_CHAR_LIMIT,
        ))
    }
}

fn strip_ascii_case_prefix<'a>(value: &'a str, prefix: &str) -> Option<&'a str> {
    value
        .get(..prefix.len())
        .filter(|candidate| candidate.eq_ignore_ascii_case(prefix))
        .map(|_| &value[prefix.len()..])
}

fn unwrap_codex_delegations(raw_prompt: &str) -> String {
    let mut prompt = raw_prompt.trim().to_string();
    const CLOSING_TAG: &str = "</codex_delegation>";
    loop {
        let Some(closing_start) = prompt.find(CLOSING_TAG) else {
            break;
        };
        let closing_end = closing_start + CLOSING_TAG.len();
        let wrapped = prompt[..closing_end].trim();
        let Some(input) = parse_codex_delegation(wrapped) else {
            break;
        };
        prompt = format!("{}{}", input, &prompt[closing_end..]);
    }
    prompt
}

fn parse_codex_delegation(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if !trimmed.starts_with("<codex_delegation>") || !trimmed.ends_with("</codex_delegation>") {
        return None;
    }

    let source = capture_xml_tag(trimmed, "source_thread_id")?;
    if source.trim().is_empty() {
        return None;
    }
    capture_xml_tag(trimmed, "input").map(decode_basic_xml_entities)
}

fn capture_xml_tag(value: &str, tag: &str) -> Option<String> {
    let pattern = format!(r"(?is)<{tag}>\s*(.*?)\s*</{tag}>");
    Regex::new(&pattern)
        .ok()?
        .captures(value)?
        .get(1)
        .map(|capture| capture.as_str().trim().to_string())
}

fn decode_basic_xml_entities(value: String) -> String {
    value
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&")
}

fn strip_writing_block_fences(value: &str) -> String {
    if !value.contains(":::writing") {
        return value.to_string();
    }

    let writing_open = Regex::new(r"^ {0,3}:::writing(?:\{.*)?\s*$").expect("writing regex");
    let writing_close = Regex::new(r"^ {0,3}:::\s*$").expect("writing close regex");
    let mut output = Vec::new();
    let mut inside_writing = false;
    let mut code_fence: Option<(char, usize)> = None;

    for line in value.lines() {
        let was_in_code = code_fence.is_some();
        update_code_fence(&mut code_fence, line);
        if was_in_code || code_fence.is_some() {
            output.push(line);
            continue;
        }
        if writing_open.is_match(line) {
            inside_writing = true;
            continue;
        }
        if inside_writing && writing_close.is_match(line) {
            inside_writing = false;
            continue;
        }
        output.push(line);
    }

    output.join("\n")
}

fn update_code_fence(current: &mut Option<(char, usize)>, line: &str) {
    let trimmed = line.trim_start_matches(' ');
    if line.len().saturating_sub(trimmed.len()) > 3 {
        return;
    }
    let Some(marker) = trimmed
        .chars()
        .next()
        .filter(|value| *value == '`' || *value == '~')
    else {
        return;
    };
    let length = trimmed.chars().take_while(|value| *value == marker).count();
    if length < 3 {
        return;
    }
    let suffix = &trimmed[marker.len_utf8() * length..];

    match current {
        Some((open_marker, open_length))
            if *open_marker == marker && length >= *open_length && suffix.trim().is_empty() =>
        {
            *current = None;
        }
        None if marker != '`' || !suffix.contains('`') => {
            *current = Some((marker, length));
        }
        _ => {}
    }
}

fn markdown_to_plain_text(value: &str) -> String {
    markdown_to_plain_text_inner(&strip_nonvisible_html(value), 0)
}

fn markdown_to_plain_text_inner(value: &str, depth: usize) -> String {
    let options = Options::ENABLE_TABLES
        | Options::ENABLE_FOOTNOTES
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_TASKLISTS;
    let mut output = String::new();
    for event in Parser::new_ext(value, options) {
        match event {
            Event::Text(text)
            | Event::Code(text)
            | Event::InlineMath(text)
            | Event::DisplayMath(text)
            | Event::FootnoteReference(text) => output.push_str(&text),
            Event::Html(html) | Event::InlineHtml(html) => {
                if depth < 2 {
                    let markdown = html_fragment_to_markdown(&html);
                    output.push_str(&markdown_to_plain_text_inner(&markdown, depth + 1));
                }
            }
            Event::SoftBreak | Event::HardBreak | Event::Rule => output.push(' '),
            Event::End(tag) if block_tag_needs_separator(tag) => output.push(' '),
            Event::Start(_) | Event::End(_) | Event::TaskListMarker(_) => {}
        }
    }
    remove_spaces_before_punctuation(&normalize_whitespace(&output))
}

fn block_tag_needs_separator(tag: TagEnd) -> bool {
    matches!(
        tag,
        TagEnd::Paragraph
            | TagEnd::Heading(_)
            | TagEnd::BlockQuote(_)
            | TagEnd::CodeBlock
            | TagEnd::HtmlBlock
            | TagEnd::List(_)
            | TagEnd::Item
            | TagEnd::FootnoteDefinition
            | TagEnd::DefinitionList
            | TagEnd::DefinitionListTitle
            | TagEnd::DefinitionListDefinition
            | TagEnd::Table
            | TagEnd::TableHead
            | TagEnd::TableRow
            | TagEnd::TableCell
            | TagEnd::MetadataBlock(_)
    )
}

fn strip_nonvisible_html(value: &str) -> String {
    let mut output = value.to_string();
    for tag in ["script", "style", "head", "template", "noscript"] {
        let pattern = format!(r"(?is)<{tag}\b[^>]*>.*?</{tag}\s*>");
        if let Ok(regex) = Regex::new(&pattern) {
            output = regex.replace_all(&output, " ").into_owned();
        }
    }
    Regex::new(r"(?s)<!--.*?-->")
        .expect("html comment regex")
        .replace_all(&output, " ")
        .into_owned()
}

fn html_fragment_to_markdown(fragment: &str) -> String {
    let mut handlers: HashMap<String, Box<dyn TagHandlerFactory>> = HashMap::new();
    for tag in ["script", "style", "head", "template", "noscript", "iframe"] {
        handlers.insert(tag.to_string(), Box::new(DropHtmlTagFactory));
    }
    for tag in ["details", "summary", "sub", "sup"] {
        handlers.insert(tag.to_string(), Box::new(PlainHtmlTagFactory));
    }
    html2md::parse_html_custom(fragment, &handlers)
}

struct DropHtmlTagFactory;

impl TagHandlerFactory for DropHtmlTagFactory {
    fn instantiate(&self) -> Box<dyn TagHandler> {
        Box::new(DropHtmlTagHandler)
    }
}

struct DropHtmlTagHandler;

impl TagHandler for DropHtmlTagHandler {
    fn handle(&mut self, _tag: &Handle, _printer: &mut StructuredPrinter) {}

    fn after_handle(&mut self, _printer: &mut StructuredPrinter) {}

    fn skip_descendants(&self) -> bool {
        true
    }
}

struct PlainHtmlTagFactory;

impl TagHandlerFactory for PlainHtmlTagFactory {
    fn instantiate(&self) -> Box<dyn TagHandler> {
        Box::new(html2md::dummy::DummyHandler)
    }
}

fn normalize_whitespace(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn remove_spaces_before_punctuation(value: &str) -> String {
    Regex::new(r"\s+([,.;:!?，。；：！？])")
        .expect("punctuation spacing regex")
        .replace_all(value, "$1")
        .into_owned()
}

fn take_chars(value: &str, limit: usize) -> String {
    value.chars().take(limit).collect()
}

fn truncate_with_ellipsis(value: &str, limit: usize) -> String {
    if value.chars().count() <= limit {
        return value.to_string();
    }
    if limit == 0 {
        return String::new();
    }
    let shortened = value
        .chars()
        .take(limit.saturating_sub(1))
        .collect::<String>();
    format!("{}…", shortened.trim_end())
}

#[cfg(test)]
mod tests {
    use super::{
        fallback_session_title, is_codex_unauthorized_error, normalize_generated_title,
        prepare_session_title_prompt, strip_writing_block_fences,
    };

    #[test]
    fn prepares_plain_title_input_from_markdown_and_html() {
        let prompt = r#"# Fix **login**

<div>Handle <code>OAuthCallback</code></div>
<script>ignore_me()</script>
<!-- hidden -->

[issue](https://example.com/ABC-123)"#;

        assert_eq!(
            prepare_session_title_prompt(prompt).as_deref(),
            Some("Fix login Handle OAuthCallback issue")
        );
    }

    #[test]
    fn preserves_inline_text_across_markdown_formatting() {
        assert_eq!(
            prepare_session_title_prompt("Fix OAuth**Callback** and `Player`Controller").as_deref(),
            Some("Fix OAuthCallback and PlayerController")
        );
    }

    #[test]
    fn unwraps_codex_delegation_and_decodes_input() {
        let prompt = r#"<codex_delegation>
  <source_thread_id>thread-1</source_thread_id>
  <input>Fix &lt;PlayerController&gt; &amp; tests</input>
</codex_delegation>"#;

        assert_eq!(
            prepare_session_title_prompt(prompt).as_deref(),
            Some("Fix & tests")
        );
    }

    #[test]
    fn removes_writing_fences_but_preserves_fences_inside_code() {
        let prompt = ":::writing{variant=\"standard\" id=\"12345\"}\nTitle body\n:::\n\n```text\n:::writing\n::: \n```";
        let stripped = strip_writing_block_fences(prompt);

        assert!(stripped.starts_with("Title body"));
        assert!(stripped.contains("```text\n:::writing\n::: \n```"));
    }

    #[test]
    fn fallback_title_is_plain_and_limited() {
        let input = "修复玩家控制器登录回调并补充编辑器测试，同时更新相关文档、错误提示与兼容路径，覆盖旧项目升级、新项目初始化、离线恢复与跨平台构建行为";
        let title = fallback_session_title(input).expect("fallback title");

        assert_eq!(title.chars().count(), 60);
        assert!(title.ends_with('…'));
    }

    #[test]
    fn normalizes_structured_title_defensively() {
        assert_eq!(
            normalize_generated_title("**Title: Fix OAuth callback.**").as_deref(),
            Some("Fix OAuth callback")
        );
    }

    #[test]
    fn recognizes_http_and_websocket_unauthorized_errors() {
        assert!(is_codex_unauthorized_error(
            "OpenAI Codex API error (401 Unauthorized): expired"
        ));
        assert!(is_codex_unauthorized_error(
            "OpenAI Codex websocket error (HTTP 401): expired"
        ));
    }
}
