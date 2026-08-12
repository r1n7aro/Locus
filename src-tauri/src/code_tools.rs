//! Runtime per-tool enablement for the code-analysis tool family.
//!
//! Mirrors the persisted `AppConfig::code_analysis_tools` flags into a global
//! so that hot paths (`AgentInstance::resolve_effective_tool_names`, the
//! Roslyn server's configuration handler) can read them without threading an
//! `AppHandle` through. Same pattern as `csharp_lsp::ENABLED`: commands
//! persist via `AppConfig` and mirror here.

use std::collections::HashSet;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

use futures::StreamExt;

use crate::config::CodeAnalysisToolsConfig;

static CONFIG: Mutex<Option<CodeAnalysisToolsConfig>> = Mutex::new(None);

/// Called once from app setup with the persisted flags.
pub fn initialize(value: CodeAnalysisToolsConfig) {
    if let Ok(mut guard) = CONFIG.lock() {
        *guard = Some(value);
    }
}

pub fn current() -> CodeAnalysisToolsConfig {
    CONFIG
        .lock()
        .ok()
        .and_then(|guard| *guard)
        .unwrap_or_default()
}

pub fn set(value: CodeAnalysisToolsConfig) {
    if let Ok(mut guard) = CONFIG.lock() {
        *guard = Some(value);
    }
}

/// Whether a code-analysis tool is enabled by its per-tool switch. Tools not
/// in the family are always enabled (they are governed elsewhere). Note the
/// `code_*` tools additionally require `csharp_lsp::is_enabled()`; that check
/// stays at the gating site.
pub fn tool_enabled(tool: &str) -> bool {
    let config = current();
    match tool {
        "code_symbol_search" => config.code_symbol_search,
        "code_goto_definition" => config.code_goto_definition,
        "code_find_references" => config.code_find_references,
        "code_diagnostics" => config.code_diagnostics,
        "code_hover" => config.code_hover,
        "unity_code_usages" => config.unity_code_usages,
        _ => true,
    }
}

/// Whether hot reload/recompile results should append warning-level diagnostics
/// for the C# files in the applied batch. The persisted field keeps its legacy
/// name for config compatibility. This remains independent from exposing the
/// standalone `code_diagnostics` tool to the agent.
pub fn automatic_diagnostics_enabled() -> bool {
    current().edit_write_diagnostics
}

/// Whether Microsoft.Unity.Analyzers should be injected into the Roslyn
/// language server workspace (see `csharp_lsp::analyzers`).
pub fn unity_analyzers_enabled() -> bool {
    current().unity_analyzers
}

const HOT_RELOAD_WARNING_TIMEOUT: Duration = Duration::from_secs(2);
const RECOMPILE_WARNING_TIMEOUT: Duration = Duration::from_secs(5);
const AUTOMATIC_WARNING_MAX_FILES: usize = 32;
const AUTOMATIC_WARNING_MAX_RESULTS: usize = 30;
const AUTOMATIC_WARNING_CONCURRENCY: usize = 2;

fn normalized_path_key(path: &Path) -> String {
    let normalized = dunce::simplified(path).to_string_lossy().replace('\\', "/");
    if cfg!(windows) {
        normalized.to_ascii_lowercase()
    } else {
        normalized
    }
}

fn same_workspace(left: &str, right: &str) -> bool {
    normalized_path_key(Path::new(left)) == normalized_path_key(Path::new(right))
}

fn batch_csharp_paths(workspace: &str, paths: Vec<String>) -> (Vec<String>, usize) {
    let workspace_path = Path::new(workspace);
    let mut seen = HashSet::new();
    let mut resolved = Vec::new();

    for path in paths {
        let path = PathBuf::from(path.trim());
        if path.as_os_str().is_empty() {
            continue;
        }
        let absolute = if path.is_absolute() {
            path
        } else {
            workspace_path.join(path)
        };
        if !absolute.is_file() {
            // Deleted files have no document warnings. Compile/hot-reload
            // diagnostics remain authoritative for references left behind.
            continue;
        }
        let display = absolute.to_string_lossy().to_string();
        if !crate::csharp_lsp::is_unity_managed_csharp_file(workspace, &display) {
            continue;
        }
        if seen.insert(normalized_path_key(&absolute)) {
            resolved.push(display);
        }
    }

    resolved.sort_by_key(|path| normalized_path_key(Path::new(path)));
    let omitted = resolved.len().saturating_sub(AUTOMATIC_WARNING_MAX_FILES);
    resolved.truncate(AUTOMATIC_WARNING_MAX_FILES);
    (resolved, omitted)
}

fn format_semantic_warning_feedback(
    mut diagnostics: Vec<crate::csharp_lsp::CodeDiagnostic>,
    files_queried: usize,
    files_unavailable: usize,
    files_omitted: usize,
) -> String {
    diagnostics.retain(|diagnostic| diagnostic.severity == 2);
    let mut seen = HashSet::new();
    diagnostics.retain(|diagnostic| {
        seen.insert((
            diagnostic.path.to_ascii_lowercase(),
            diagnostic.line,
            diagnostic.column,
            diagnostic.code.clone().unwrap_or_default(),
            diagnostic.message.clone(),
        ))
    });
    diagnostics.sort_by(|left, right| {
        left.path
            .cmp(&right.path)
            .then(left.line.cmp(&right.line))
            .then(left.column.cmp(&right.column))
    });

    let total = diagnostics.len();
    let shown = total.min(AUTOMATIC_WARNING_MAX_RESULTS);
    let mut output = if total == 0 {
        format!("C# semantic warnings: none in {files_queried} file(s).")
    } else {
        format!(
            "C# semantic warnings: {total} warning(s) in {files_queried} file(s){}.",
            if shown < total {
                format!(", showing first {shown}")
            } else {
                String::new()
            }
        )
    };

    let mut current_file = "";
    for diagnostic in diagnostics.iter().take(AUTOMATIC_WARNING_MAX_RESULTS) {
        if diagnostic.path != current_file {
            output.push('\n');
            output.push_str(&diagnostic.path);
            current_file = &diagnostic.path;
        }
        output.push_str(&format!(
            "\n  {}:{} warning{}: {}",
            diagnostic.line,
            diagnostic.column,
            diagnostic
                .code
                .as_deref()
                .map(|code| format!(" {code}"))
                .unwrap_or_default(),
            diagnostic.message.replace('\n', " ")
        ));
    }

    if files_unavailable > 0 {
        output.push_str(&format!(
            "\nSemantic warning analysis was unavailable for {files_unavailable} file(s)."
        ));
    }
    if files_omitted > 0 {
        output.push_str(&format!(
            "\nSemantic warning analysis omitted {files_omitted} file(s) beyond the automatic {AUTOMATIC_WARNING_MAX_FILES}-file limit."
        ));
    }
    output
}

async fn collect_semantic_warning_feedback(
    workspace: &str,
    paths: Vec<String>,
    timeout: Duration,
) -> Option<String> {
    if !automatic_diagnostics_enabled() || !crate::csharp_lsp::is_enabled() {
        return None;
    }

    let (paths, files_omitted) = batch_csharp_paths(workspace, paths);
    if paths.is_empty() {
        return None;
    }

    // Automatic feedback stays on the already-warm fast path. Explicit
    // code_diagnostics remains the entry point that may wait for Roslyn startup.
    let status = crate::csharp_lsp::status().await;
    if status.phase != "ready"
        || status
            .workspace
            .as_deref()
            .map(|active| !same_workspace(active, workspace))
            .unwrap_or(true)
    {
        return Some(format!(
            "C# semantic warnings unavailable: analysis server is {}.",
            status.phase
        ));
    }

    let files_queried = paths.len();
    let queries = futures::stream::iter(paths.into_iter().map(|path| async move {
        crate::csharp_lsp::document_diagnostics(workspace, &path).await
    }))
    .buffer_unordered(AUTOMATIC_WARNING_CONCURRENCY);

    match tokio::time::timeout(timeout, queries.collect::<Vec<_>>()).await {
        Ok(results) => {
            let files_unavailable = results.iter().filter(|result| result.is_err()).count();
            let diagnostics = results
                .into_iter()
                .filter_map(Result::ok)
                .flatten()
                .collect();
            Some(format_semantic_warning_feedback(
                diagnostics,
                files_queried,
                files_unavailable,
                files_omitted,
            ))
        }
        Err(_) => Some(format!(
            "C# semantic warnings unavailable: analysis exceeded {} ms.",
            timeout.as_millis()
        )),
    }
}

async fn run_with_semantic_warning_feedback<F>(
    workspace: &str,
    paths: Vec<String>,
    timeout: Duration,
    operation: F,
) -> Result<String, String>
where
    F: Future<Output = Result<String, String>>,
{
    let warnings = collect_semantic_warning_feedback(workspace, paths, timeout);
    let (outcome, warnings) = tokio::join!(operation, warnings);

    let append = |mut output: String| {
        if let Some(warnings) = warnings.as_deref() {
            output.push_str("\n\n");
            output.push_str(warnings);
        }
        output
    };

    match outcome {
        Ok(output) => Ok(append(output)),
        Err(error) => Err(append(error)),
    }
}

/// Run hot reload for a stable snapshot of the requested pending files and
/// append warning-level Roslyn diagnostics for that same batch.
pub async fn hot_reload_with_semantic_warnings(
    workspace: &str,
    requested_paths: Option<Vec<String>>,
) -> Result<String, String> {
    let paths = crate::unity_hotreload::coordinator::pending_paths_filtered(
        workspace,
        requested_paths.as_deref(),
    )
    .await;
    run_with_semantic_warning_feedback(
        workspace,
        paths.clone(),
        HOT_RELOAD_WARNING_TIMEOUT,
        crate::unity_hotreload::coordinator::hot_reload(workspace, Some(paths)),
    )
    .await
}

/// Run a real Unity recompile while analyzing the tracked C# batch in parallel.
/// Internal convergence recompiles continue to use `recompile_and_wait`
/// directly and therefore stay silent.
pub async fn recompile_with_semantic_warnings(workspace: &str) -> Result<String, String> {
    let paths = crate::unity_hotreload::coordinator::pending_paths(workspace).await;
    run_with_semantic_warning_feedback(
        workspace,
        paths,
        RECOMPILE_WARNING_TIMEOUT,
        crate::unity_bridge::recompile_and_wait(workspace),
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn diagnostic(
        path: &str,
        line: u32,
        severity: u8,
        code: &str,
    ) -> crate::csharp_lsp::CodeDiagnostic {
        crate::csharp_lsp::CodeDiagnostic {
            path: path.to_string(),
            line,
            column: 3,
            severity,
            code: Some(code.to_string()),
            message: format!("message {code}"),
        }
    }

    #[test]
    fn automatic_feedback_only_formats_warnings() {
        let output = format_semantic_warning_feedback(
            vec![
                diagnostic("Assets/Test.cs", 2, 1, "CS1002"),
                diagnostic("Assets/Test.cs", 4, 2, "UNT0001"),
                diagnostic("Assets/Test.cs", 6, 3, "IDE0001"),
            ],
            1,
            0,
            0,
        );

        assert!(output.contains("1 warning(s)"));
        assert!(output.contains("warning UNT0001"));
        assert!(!output.contains("CS1002"));
        assert!(!output.contains("IDE0001"));
    }

    #[test]
    fn automatic_feedback_deduplicates_warnings_and_reports_limits() {
        let warning = diagnostic("Assets/Test.cs", 4, 2, "UNT0001");
        let output = format_semantic_warning_feedback(vec![warning.clone(), warning], 32, 1, 2);

        assert!(output.contains("1 warning(s)"));
        assert!(output.contains("unavailable for 1 file(s)"));
        assert!(output.contains("omitted 2 file(s)"));
    }
}
