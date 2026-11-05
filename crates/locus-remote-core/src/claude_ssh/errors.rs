use std::sync::OnceLock;
const DISCONNECTED: &str = "Claude transport disconnected: ";
pub fn disconnected(reason: impl AsRef<str>) -> String {
    format!("{DISCONNECTED}{}", reason.as_ref())
}

pub fn is_disconnected(error: &str) -> bool {
    error.starts_with(DISCONNECTED)
}

/// Adding a stage must not hide the transport tag from the bounded retry loop.
pub fn with_context(context: &str, error: String) -> String {
    match error.strip_prefix(DISCONNECTED) {
        Some(reason) => disconnected(format!("{context}: {reason}")),
        None => format!("{context}: {error}"),
    }
}

// OpenSSH has no structured startup verdict. Classify its bounded diagnostic
// only at the SSH startup boundary, never arbitrary model/tool error text.
pub fn startup_rejected(error: &str) -> bool {
    let lower = error.to_ascii_lowercase();
    [
        "permission denied",
        "host key verification failed",
        "remote host identification has changed",
        "invalid",
        "not found",
        "keychain",
        "authentication",
        "not logged in",
        "plugin validation failed",
    ]
    .iter()
    .any(|s| lower.contains(s))
}

pub fn ssh_startup_error(error: String) -> String {
    if startup_rejected(&error) {
        return error
            .strip_prefix(DISCONNECTED)
            .unwrap_or(&error)
            .to_string();
    }
    if is_disconnected(&error) {
        return error;
    }
    let lower = error.to_ascii_lowercase();
    if [
        "connection timed out",
        "connection refused",
        "connection reset",
        "connection closed",
        "broken pipe",
        "network is unreachable",
        "no route to host",
        "operation timed out",
    ]
    .iter()
    .any(|s| lower.contains(s))
    {
        disconnected(error)
    } else {
        error
    }
}

pub fn plain_text(text: &str) -> String {
    static ESCAPES: OnceLock<regex::Regex> = OnceLock::new();
    let escapes = ESCAPES.get_or_init(|| regex::Regex::new(
        r"\x1b(?:\[[0-?]*[ -/]*[@-~]|\][^\x07\x1b]*(?:\x07|\x1b\\)|[@-_])"
    ).expect("terminal escape pattern"));
    let plain = escapes.replace_all(text, "");
    let plain: String = plain.chars().filter(|c| !c.is_control() || matches!(c, '\n' | '\r' | '\t')).collect();
    let plain = plain.replace('\r', "\n");
    let plain = plain.lines().map(str::trim).filter(|line| !line.is_empty()).collect::<Vec<_>>().join("\n");
    plain.chars().rev().take(6000).collect::<Vec<_>>().into_iter().rev().collect()
}

