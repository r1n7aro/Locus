use std::sync::Arc;

use super::{make_exec, ToolDef, ToolExecuteFn, ToolResult};

// ─── web_fetch ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WebFetchFormat {
    Markdown,
    Html,
    Text,
}

struct FetchedPage {
    body: String,
    content_type: String,
}

pub(super) fn web_fetch() -> ToolDef {
    let prompt = crate::prompt::parse_tool_prompt(crate::prompt::tools::WEB_FETCH);
    ToolDef {
        name: "web_fetch".to_string(),
        description: prompt.description,
        parameters: prompt.parameters,
        execute: make_exec(|args, _ctx| {
            Box::pin(async move {
                let url = args.get("url").and_then(|v| v.as_str()).unwrap_or("");
                let format = match parse_web_fetch_format(args.get("format")) {
                    Ok(format) => format,
                    Err(message) => {
                        return ToolResult {
                            output: message,
                            is_error: true,
                        };
                    }
                };
                let timeout_secs = args
                    .get("timeout")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(30)
                    .min(120);
                let max_chars = args
                    .get("maxChars")
                    .or_else(|| args.get("max_chars"))
                    .and_then(|v| v.as_u64())
                    .unwrap_or(50_000)
                    .clamp(1, 200_000) as usize;

                let parsed_url = match url::Url::parse(url) {
                    Ok(parsed) if matches!(parsed.scheme(), "http" | "https") => parsed,
                    _ => {
                        return ToolResult {
                            output: "Error: URL must start with http:// or https://".to_string(),
                            is_error: true,
                        }
                    }
                };

                let client = match crate::network::reqwest_client(
                    crate::network::ReqwestClientOptions::new()
                        .timeout(std::time::Duration::from_secs(timeout_secs))
                        .user_agent(web_fetch_user_agent())
                        .gzip(true)
                        .deflate(true),
                ) {
                    Ok(client) => client,
                    Err(e) => {
                        return ToolResult {
                            output: format!("Error creating HTTP client: {}", e),
                            is_error: true,
                        }
                    }
                };

                let page = match fetch_web_page(&client, &parsed_url, format, timeout_secs).await {
                    Ok(page) => page,
                    Err(e) => {
                        return ToolResult {
                            output: e,
                            is_error: true,
                        };
                    }
                };

                let mut content = render_web_fetch_content(&page.body, &page.content_type, format);

                let truncated = content.len() > max_chars;
                if truncated {
                    content = truncate_utf8_prefix(&content, max_chars).to_string();
                    content.push_str("\n\n(Content truncated)");
                }

                ToolResult {
                    output: content,
                    is_error: false,
                }
            })
        }),
    }
}

fn parse_web_fetch_format(value: Option<&serde_json::Value>) -> Result<WebFetchFormat, String> {
    match value.and_then(|v| v.as_str()).unwrap_or("markdown") {
        "markdown" => Ok(WebFetchFormat::Markdown),
        "html" => Ok(WebFetchFormat::Html),
        "text" => Ok(WebFetchFormat::Text),
        other => Err(format!(
            "Error: unsupported format '{}'. Use markdown, html, or text.",
            other
        )),
    }
}

async fn fetch_web_page(
    client: &reqwest::Client,
    parsed_url: &url::Url,
    format: WebFetchFormat,
    timeout_secs: u64,
) -> Result<FetchedPage, String> {
    fetch_single_url(
        client,
        parsed_url.as_str(),
        accept_header_for_format(format),
        timeout_secs,
    )
    .await
}

async fn fetch_single_url(
    client: &reqwest::Client,
    url: &str,
    accept: &'static str,
    timeout_secs: u64,
) -> Result<FetchedPage, String> {
    let response = client
        .get(url)
        .header(reqwest::header::ACCEPT, accept)
        .header(reqwest::header::ACCEPT_LANGUAGE, "en-US,en;q=0.9,*;q=0.5")
        .timeout(std::time::Duration::from_secs(timeout_secs))
        .send()
        .await
        .map_err(|e| format!("Error fetching URL: {}", e))?;

    let status = response.status();
    if !status.is_success() {
        return Err(format!(
            "HTTP {}: {}",
            status.as_u16(),
            status.canonical_reason().unwrap_or("Unknown")
        ));
    }

    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();

    let body = response
        .text()
        .await
        .map_err(|e| format!("Error reading response body: {}", e))?;

    Ok(FetchedPage { body, content_type })
}

fn accept_header_for_format(format: WebFetchFormat) -> &'static str {
    match format {
        WebFetchFormat::Markdown => markdown_accept_header(),
        WebFetchFormat::Html => "text/html,application/xhtml+xml;q=0.9,*/*;q=0.5",
        WebFetchFormat::Text => "text/plain,text/html;q=0.8,*/*;q=0.5",
    }
}

fn markdown_accept_header() -> &'static str {
    "text/markdown,text/x-markdown,text/plain;q=0.9,text/html;q=0.8,application/xhtml+xml;q=0.7,*/*;q=0.5"
}

fn web_fetch_user_agent() -> &'static str {
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36 LocusWebFetch/1.0"
}

fn render_web_fetch_content(body: &str, content_type: &str, format: WebFetchFormat) -> String {
    match format {
        WebFetchFormat::Html => body.to_string(),
        WebFetchFormat::Text => {
            if is_html_content_type(content_type) || looks_like_html(body) {
                strip_html_tags(body)
            } else {
                body.to_string()
            }
        }
        WebFetchFormat::Markdown => {
            if is_html_content_type(content_type) || looks_like_html(body) {
                html_to_markdown(body)
            } else {
                body.to_string()
            }
        }
    }
}

fn html_to_markdown(html: &str) -> String {
    use regex::Regex;

    let mut s = html.to_string();

    let re_script = Regex::new(r"(?is)<script[\s\S]*?</script>").unwrap();
    s = re_script.replace_all(&s, "").to_string();
    let re_style = Regex::new(r"(?is)<style[\s\S]*?</style>").unwrap();
    s = re_style.replace_all(&s, "").to_string();

    s = html2md::parse_html(&s);
    let re_newlines = Regex::new(r"\n{3,}").unwrap();
    s = re_newlines.replace_all(&s, "\n\n").to_string();

    s.trim().to_string()
}

fn is_html_content_type(content_type: &str) -> bool {
    content_type.to_ascii_lowercase().contains("html")
}

fn looks_like_html(body: &str) -> bool {
    let trimmed = body.trim_start().to_ascii_lowercase();
    trimmed.starts_with("<!doctype html")
        || trimmed.starts_with("<html")
        || trimmed.contains("<body")
        || trimmed.contains("<head")
}

fn strip_html_tags(html: &str) -> String {
    use regex::Regex;

    let mut s = html.to_string();
    let re_script = Regex::new(r"(?is)<script[\s\S]*?</script>").unwrap();
    s = re_script.replace_all(&s, "").to_string();
    let re_style = Regex::new(r"(?is)<style[\s\S]*?</style>").unwrap();
    s = re_style.replace_all(&s, "").to_string();
    let re_tags = Regex::new(r"<[^>]+>").unwrap();
    s = re_tags.replace_all(&s, "").to_string();
    s = decode_html_entities(&s);
    s.trim().to_string()
}

fn decode_html_entities(s: &str) -> String {
    s.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&nbsp;", " ")
}

pub(super) fn truncate_utf8_prefix(s: &str, max_bytes: usize) -> &str {
    if s.len() <= max_bytes {
        s
    } else {
        &s[..s.floor_char_boundary(max_bytes)]
    }
}

// ─── todowrite ──────────────────────────────────────────────────────────────

pub(super) fn todowrite() -> ToolDef {
    let execute: ToolExecuteFn = Arc::new(|_args, _ctx| {
        Box::pin(async {
            ToolResult {
                output: "Error: todowrite tool should be intercepted by agent loop".to_string(),
                is_error: true,
            }
        })
    });

    let prompt = crate::prompt::parse_tool_prompt(crate::prompt::tools::TODOWRITE);
    ToolDef {
        name: "todowrite".to_string(),
        description: prompt.description,
        parameters: prompt.parameters,
        execute,
    }
}

// ─── graph_view ─────────────────────────────────────────────────────────────

pub(super) fn graph_view() -> ToolDef {
    let execute: ToolExecuteFn = Arc::new(|_args, _ctx| {
        Box::pin(async {
            ToolResult {
                output: "Error: graph_view tool should be intercepted by agent loop".to_string(),
                is_error: true,
            }
        })
    });

    let prompt = crate::prompt::parse_tool_prompt(crate::prompt::tools::GRAPH_VIEW);
    ToolDef {
        name: "graph_view".to_string(),
        description: prompt.description,
        parameters: prompt.parameters,
        execute,
    }
}

// ─── ask ─────────────────────────────────────────────────────────────────────

pub(super) fn ask() -> ToolDef {
    let execute: ToolExecuteFn = Arc::new(|_args, _ctx| {
        Box::pin(async {
            ToolResult {
                output: "Error: ask tool should be intercepted by agent loop".to_string(),
                is_error: true,
            }
        })
    });

    let prompt = crate::prompt::parse_tool_prompt(crate::prompt::tools::ASK);
    ToolDef {
        name: "ask_user_question".to_string(),
        description: prompt.description,
        parameters: prompt.parameters,
        execute,
    }
}

#[cfg(test)]
mod tests {
    use super::{html_to_markdown, render_web_fetch_content, truncate_utf8_prefix, WebFetchFormat};

    #[test]
    fn truncate_utf8_prefix_handles_cjk_boundary() {
        let text = "abc中文def";
        assert_eq!(truncate_utf8_prefix(text, 4), "abc");
        assert_eq!(truncate_utf8_prefix(text, 6), "abc中");
    }

    #[test]
    fn truncate_utf8_prefix_handles_emoji_boundary() {
        let text = "ab😀cd";
        assert_eq!(truncate_utf8_prefix(text, 3), "ab");
        assert_eq!(truncate_utf8_prefix(text, 6), "ab😀");
    }

    #[test]
    fn web_fetch_html_to_markdown_keeps_links() {
        let markdown =
            html_to_markdown(r#"<main><h1>Title</h1><p>See <a href="/docs">docs</a>.</p></main>"#);
        assert!(markdown.contains("Title"));
        assert!(markdown.contains("[docs](/docs)"));
    }

    #[test]
    fn web_fetch_preserves_markdown_response() {
        let markdown = "# Title\n\nBody";
        assert_eq!(
            render_web_fetch_content(
                markdown,
                "text/markdown; charset=utf-8",
                WebFetchFormat::Markdown
            ),
            markdown
        );
    }
}
