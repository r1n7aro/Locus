use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

static SEQ: AtomicU64 = AtomicU64::new(0);

/// Sanitize a provider name into a filesystem-safe string suitable for path components.
fn sanitize_provider(provider: &str) -> String {
    provider
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect()
}

fn repo_debug_base_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("debug")
        .join("llm")
}

fn default_debug_base_dir(packaged_storage_dir: Option<PathBuf>) -> PathBuf {
    match packaged_storage_dir {
        Some(data_dir) => data_dir.join("debug").join("llm"),
        None => repo_debug_base_dir(),
    }
}

/// Resolve the absolute base directory for LLM debug captures.
///
/// Uses `LOCUS_DEBUG_DIR` if explicitly set. Otherwise:
/// - dev: `<repo_root>/debug/llm`
/// - packaged runtime: `<install_dir>/data/debug/llm`
///
/// The dev path stays outside `src-tauri/` so `tauri dev` does not pick up new captures
/// and trigger a rebuild loop.
pub fn debug_base_dir() -> PathBuf {
    match std::env::var("LOCUS_DEBUG_DIR") {
        Ok(v) if !v.is_empty() => PathBuf::from(v),
        _ => match crate::commands::packaged_runtime_storage_dir() {
            Ok(packaged_storage_dir) => default_debug_base_dir(packaged_storage_dir),
            Err(error) => {
                eprintln!(
                    "[debug] failed to resolve packaged runtime storage dir, falling back to repo debug dir: {}",
                    error
                );
                repo_debug_base_dir()
            }
        },
    }
}

/// Resolve the per-provider subfolder under the debug base. The subfolder is created
/// on demand. Callers should put all artifacts for the same provider here so the debug
/// folder stays organized by subscription endpoint.
pub fn debug_dir_for(provider: &str) -> PathBuf {
    let dir = debug_base_dir().join(sanitize_provider(provider));
    if let Err(e) = std::fs::create_dir_all(&dir) {
        eprintln!("[debug] failed to create debug dir {:?}: {}", dir, e);
    }
    dir
}

/// Persist an outgoing LLM HTTP request to the debug folder.
///
/// The destination directory is resolved by [`debug_dir_for`] and is anchored under
/// `LOCUS_DEBUG_DIR` (or the default dev / packaged debug directory) with a per-provider
/// subfolder.
/// Sensitive headers (Authorization / x-api-key) are redacted before being written.
///
/// Errors are reported to stderr but never propagated — debug logging must not break
/// the actual request flow.
pub fn save_request(provider: &str, url: &str, headers: &[(&str, &str)], body: &str) {
    let dir = debug_dir_for(provider);

    let ts = chrono::Local::now().format("%Y%m%d_%H%M%S%.3f");
    let seq = SEQ.fetch_add(1, Ordering::Relaxed);
    let filename = format!("{}_{:04}.http", ts, seq);
    let path = dir.join(filename);

    let mut content = String::new();
    content.push_str(&format!("POST {}\n", url));
    for (k, v) in headers {
        let value =
            if k.eq_ignore_ascii_case("authorization") || k.eq_ignore_ascii_case("x-api-key") {
                "***REDACTED***"
            } else {
                *v
            };
        content.push_str(&format!("{}: {}\n", k, value));
    }
    content.push('\n');
    content.push_str(body);

    if let Err(e) = std::fs::write(&path, content) {
        eprintln!("[debug] failed to write {:?}: {}", path, e);
    }
}

/// Persist a copy-pasteable reproduction of a failing request: the exact compact
/// request bytes (`<ts>_<seq>.body.json`) plus a runnable `curl` script
/// (`<ts>_<seq>.repro.sh`) that POSTs those bytes back to the same endpoint.
///
/// Intended for the gateway-truncation failure class (issue #48): because the
/// body file holds the precise bytes this client emitted, the `line 1 column N`
/// offset an upstream parser reports lines up exactly, making it trivial to
/// confirm where the request was cut.
///
/// Secrets are never written: `Authorization` / `x-api-key` are replaced with a
/// `<YOUR_API_KEY>` placeholder the user fills in before running. Returns the
/// path to the curl script, or `None` if writing failed.
pub fn save_repro_curl(
    provider: &str,
    url: &str,
    headers: &[(String, String)],
    compact_body: &str,
    note: &str,
) -> Option<PathBuf> {
    let dir = debug_dir_for(provider);
    let ts = chrono::Local::now().format("%Y%m%d_%H%M%S%.3f");
    let seq = SEQ.fetch_add(1, Ordering::Relaxed);
    let body_filename = format!("{}_{:04}.body.json", ts, seq);
    let curl_filename = format!("{}_{:04}.repro.sh", ts, seq);
    let body_path = dir.join(&body_filename);
    let curl_path = dir.join(&curl_filename);

    if let Err(e) = std::fs::write(&body_path, compact_body) {
        eprintln!("[debug] failed to write {:?}: {}", body_path, e);
        return None;
    }

    // curl.exe on Windows bypasses the PowerShell `curl` -> Invoke-WebRequest alias.
    let curl_bin = if cfg!(windows) { "curl.exe" } else { "curl" };
    let mut cmd = format!("{} -sS -X POST '{}'", curl_bin, single_quote(url));
    for (key, value) in headers {
        let value = if key.eq_ignore_ascii_case("authorization") {
            "Bearer <YOUR_API_KEY>".to_string()
        } else if key.eq_ignore_ascii_case("x-api-key") {
            "<YOUR_API_KEY>".to_string()
        } else {
            value.clone()
        };
        cmd.push_str(&format!(" -H '{}: {}'", key, single_quote(&value)));
    }
    // Keep the `@` inside the quotes so PowerShell does not treat it as a splat.
    cmd.push_str(&format!(
        " --data-binary '@{}'",
        single_quote(&body_path.to_string_lossy())
    ));

    let content = format!(
        "# Reproduce the failing request (issue #48 truncation diagnostics)\n\
         # {note}\n\
         # 1) Replace <YOUR_API_KEY> with your real API key.\n\
         # 2) Run in Git Bash / WSL / macOS / Linux, or paste the curl.exe line into PowerShell.\n\
         #    (Do not use legacy cmd.exe: it does not understand single quotes.)\n\
         # Request body (exact bytes this client sent): {body_filename}\n\
         \n\
         {cmd}\n",
        note = note,
        body_filename = body_filename,
        cmd = cmd,
    );

    if let Err(e) = std::fs::write(&curl_path, content) {
        eprintln!("[debug] failed to write {:?}: {}", curl_path, e);
        return None;
    }

    Some(curl_path)
}

/// Escape single quotes for embedding inside a single-quoted shell string.
fn single_quote(value: &str) -> String {
    value.replace('\'', "'\\''")
}

#[cfg(test)]
mod tests {
    use super::{default_debug_base_dir, repo_debug_base_dir};
    use std::path::PathBuf;

    #[test]
    fn default_debug_base_dir_uses_packaged_data_dir_when_available() {
        let packaged_data_dir = PathBuf::from(r"C:\Locus\data");
        assert_eq!(
            default_debug_base_dir(Some(packaged_data_dir)),
            PathBuf::from(r"C:\Locus\data\debug\llm")
        );
    }

    #[test]
    fn default_debug_base_dir_falls_back_to_repo_debug_dir() {
        assert_eq!(default_debug_base_dir(None), repo_debug_base_dir());
    }
}
