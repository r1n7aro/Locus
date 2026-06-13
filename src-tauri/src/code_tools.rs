//! Runtime per-tool enablement for the code-analysis tool family.
//!
//! Mirrors the persisted `AppConfig::code_analysis_tools` flags into a global
//! so that hot paths (`AgentInstance::resolve_effective_tool_names`, the
//! Roslyn server's configuration handler) can read them without threading an
//! `AppHandle` through. Same pattern as `csharp_lsp::ENABLED`: commands
//! persist via `AppConfig` and mirror here.

use std::sync::Mutex;

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

/// Whether successful edit/write calls should append file-scope diagnostics for
/// touched C# files. This is intentionally independent from exposing the
/// standalone `code_diagnostics` tool to the agent.
pub fn edit_write_diagnostics_enabled() -> bool {
    current().edit_write_diagnostics
}

/// Whether Microsoft.Unity.Analyzers should be injected into the Roslyn
/// language server workspace (see `csharp_lsp::analyzers`).
pub fn unity_analyzers_enabled() -> bool {
    current().unity_analyzers
}
