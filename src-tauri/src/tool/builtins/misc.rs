use std::sync::Arc;

use super::{make_exec, ToolDef, ToolExecuteFn, ToolResult};

// ─── webfetch ────────────────────────────────────────────────────────────────

pub(super) fn webfetch() -> ToolDef {
    let prompt = crate::prompt::parse_tool_prompt(crate::prompt::tools::WEBFETCH);
    ToolDef {
        name: "webfetch".to_string(),
        description: prompt.description,
        parameters: prompt.parameters,
        execute: make_exec(|args, _ctx| {
            Box::pin(async move {
                let url = args.get("url").and_then(|v| v.as_str()).unwrap_or("");
                let format = args
                    .get("format")
                    .and_then(|v| v.as_str())
                    .unwrap_or("markdown");
                let timeout_secs = args
                    .get("timeout")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(30)
                    .min(120);

                if !url.starts_with("http://") && !url.starts_with("https://") {
                    return ToolResult {
                        output: "Error: URL must start with http:// or https://".to_string(),
                        is_error: true,
                    };
                }

                let client = match crate::network::default_reqwest_client() {
                    Ok(client) => client,
                    Err(e) => {
                        return ToolResult {
                            output: format!("Error creating HTTP client: {}", e),
                            is_error: true,
                        }
                    }
                };
                let response = match client
                    .get(url)
                    .header("User-Agent", "Mozilla/5.0 (compatible; bot/1.0)")
                    .timeout(std::time::Duration::from_secs(timeout_secs))
                    .send()
                    .await
                {
                    Ok(resp) => resp,
                    Err(e) => {
                        return ToolResult {
                            output: format!("Error fetching URL: {}", e),
                            is_error: true,
                        };
                    }
                };

                let status = response.status();
                if !status.is_success() {
                    return ToolResult {
                        output: format!(
                            "HTTP {}: {}",
                            status.as_u16(),
                            status.canonical_reason().unwrap_or("Unknown")
                        ),
                        is_error: true,
                    };
                }

                let content_type = response
                    .headers()
                    .get("content-type")
                    .and_then(|v| v.to_str().ok())
                    .unwrap_or("")
                    .to_string();

                let body = match response.text().await {
                    Ok(text) => text,
                    Err(e) => {
                        return ToolResult {
                            output: format!("Error reading response body: {}", e),
                            is_error: true,
                        };
                    }
                };

                let mut content = if format == "markdown" && content_type.contains("text/html") {
                    html_to_markdown(&body)
                } else if format == "text" && content_type.contains("text/html") {
                    strip_html_tags(&body)
                } else {
                    body
                };

                const MAX_CHARS: usize = 50_000;
                let truncated = content.len() > MAX_CHARS;
                if truncated {
                    content = truncate_utf8_prefix(&content, MAX_CHARS).to_string();
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

fn html_to_markdown(html: &str) -> String {
    use regex::Regex;

    let mut s = html.to_string();

    let re_script = Regex::new(r"(?is)<script[\s\S]*?</script>").unwrap();
    s = re_script.replace_all(&s, "").to_string();
    let re_style = Regex::new(r"(?is)<style[\s\S]*?</style>").unwrap();
    s = re_style.replace_all(&s, "").to_string();

    let re_h = Regex::new(r"(?is)<h([1-6])[^>]*>([\s\S]*?)</h\1>").unwrap();
    s = re_h
        .replace_all(&s, |caps: &regex::Captures| {
            let level: usize = caps[1].parse().unwrap_or(1);
            let text = strip_html_tags(&caps[2]);
            format!("{} {}\n\n", "#".repeat(level), text)
        })
        .to_string();

    let re_a = Regex::new(r#"(?is)<a[^>]*href="([^"]*)"[^>]*>([\s\S]*?)</a>"#).unwrap();
    s = re_a
        .replace_all(&s, |caps: &regex::Captures| {
            let href = &caps[1];
            let text = strip_html_tags(&caps[2]);
            format!("[{}]({})", text, href)
        })
        .to_string();

    let re_strong = Regex::new(r"(?is)<strong[^>]*>([\s\S]*?)</strong>").unwrap();
    s = re_strong
        .replace_all(&s, |caps: &regex::Captures| {
            format!("**{}**", strip_html_tags(&caps[1]))
        })
        .to_string();

    let re_em = Regex::new(r"(?is)<em[^>]*>([\s\S]*?)</em>").unwrap();
    s = re_em
        .replace_all(&s, |caps: &regex::Captures| {
            format!("_{}_", strip_html_tags(&caps[1]))
        })
        .to_string();

    let re_code = Regex::new(r"(?is)<code[^>]*>([\s\S]*?)</code>").unwrap();
    s = re_code
        .replace_all(&s, |caps: &regex::Captures| {
            format!("`{}`", strip_html_tags(&caps[1]))
        })
        .to_string();

    let re_pre = Regex::new(r"(?is)<pre[^>]*>([\s\S]*?)</pre>").unwrap();
    s = re_pre
        .replace_all(&s, |caps: &regex::Captures| {
            format!("```\n{}\n```\n", strip_html_tags(&caps[1]))
        })
        .to_string();

    let re_li = Regex::new(r"(?is)<li[^>]*>([\s\S]*?)</li>").unwrap();
    s = re_li
        .replace_all(&s, |caps: &regex::Captures| {
            format!("- {}\n", strip_html_tags(&caps[1]).trim())
        })
        .to_string();

    let re_br = Regex::new(r"(?i)<br\s*/?>").unwrap();
    s = re_br.replace_all(&s, "\n").to_string();

    let re_p = Regex::new(r"(?is)<p[^>]*>([\s\S]*?)</p>").unwrap();
    s = re_p
        .replace_all(&s, |caps: &regex::Captures| {
            format!("{}\n\n", strip_html_tags(&caps[1]).trim())
        })
        .to_string();

    let re_tags = Regex::new(r"<[^>]+>").unwrap();
    s = re_tags.replace_all(&s, "").to_string();

    s = decode_html_entities(&s);

    let re_newlines = Regex::new(r"\n{3,}").unwrap();
    s = re_newlines.replace_all(&s, "\n\n").to_string();

    s.trim().to_string()
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

// ─── canvas ──────────────────────────────────────────────────────────────────

pub(super) fn canvas() -> ToolDef {
    let prompt = crate::prompt::parse_tool_prompt(crate::prompt::tools::CANVAS);
    ToolDef {
        name: "canvas".to_string(),
        description: prompt.description,
        parameters: prompt.parameters,
        execute: make_exec(|args, _ctx| {
            Box::pin(async move {
                let spec = match args.get("spec") {
                    Some(s) => s.clone(),
                    None => {
                        return ToolResult {
                            output: "Missing required parameter: spec".to_string(),
                            is_error: true,
                        }
                    }
                };

                let node_count = spec
                    .get("nodes")
                    .and_then(|n| n.as_array())
                    .map(|a| a.len())
                    .unwrap_or(0);
                let edge_count = spec
                    .get("edges")
                    .and_then(|e| e.as_array())
                    .map(|a| a.len())
                    .unwrap_or(0);
                let title = spec
                    .get("title")
                    .and_then(|t| t.as_str())
                    .unwrap_or("Canvas");

                ToolResult {
                    output: format!(
                        "Canvas \"{}\" created ({} nodes, {} edges). The canvas spec is available in the tool arguments.",
                        title, node_count, edge_count
                    ),
                    is_error: false,
                }
            })
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::truncate_utf8_prefix;

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
}
