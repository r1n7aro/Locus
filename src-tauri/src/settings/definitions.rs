//! The settings table. Adding a setting takes one line here plus its
//! `settings.item.<key>.label` / `.desc` entries in `src/language/*.json`;
//! settings with side effects also register an apply hook.
//!
//! Keys are dot paths and double as the JSON path inside `settings.json`.

use serde_json::Value;

use super::schema::{define_settings, Kind, Scope, Sensitivity};

/// Upper bound for idle timeouts (ten years); keeps scheduler deadlines
/// representable on every platform.
pub const MAX_IDLE_TIMEOUT_SECS: i64 = 10 * 365 * 24 * 60 * 60;

fn validate_worktree_root(value: &Value) -> Result<Value, String> {
    let text = value.as_str().unwrap_or_default();
    if text.is_empty() {
        return Ok(Value::from(""));
    }
    crate::workspace_service::worktrees::location::normalize_root_setting(std::path::Path::new(text))
        .map(|path| Value::from(path.to_string_lossy().into_owned()))
}

fn validate_skill_package_namespace(value: &Value) -> Result<Value, String> {
    let text = value.as_str().unwrap_or_default();
    crate::commands::normalize_default_skill_package_namespace(text).map(Value::from)
}

fn validate_git_path(value: &Value) -> Result<Value, String> {
    crate::commands::validate_git_path_setting(value.as_str().unwrap_or_default()).map(Value::from)
}

fn string_items(value: &Value) -> impl Iterator<Item = &str> {
    value
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::trim)
        .filter(|item| !item.is_empty())
}

fn validate_remote_interfaces(value: &Value) -> Result<Value, String> {
    let mut interfaces: Vec<&str> = Vec::new();
    for name in string_items(value) {
        if !interfaces.contains(&name) {
            interfaces.push(name);
        }
    }
    if interfaces.len() > crate::remote_access::config::MAX_INTERFACES {
        return Err("too many network interfaces".to_string());
    }
    Ok(Value::from(interfaces))
}

fn validate_remote_hostnames(value: &Value) -> Result<Value, String> {
    let mut hostnames = Vec::new();
    for raw in string_items(value) {
        let host = crate::remote_access::config::normalize_hostname(raw)
            .ok_or_else(|| format!("invalid hostname '{raw}'"))?;
        if !hostnames.contains(&host) {
            hostnames.push(host);
        }
    }
    if hostnames.len() > crate::remote_access::config::MAX_HOSTNAMES {
        return Err("too many hostnames".to_string());
    }
    Ok(Value::from(hostnames))
}

fn validate_claude_idle_minutes(value: &Value) -> Result<Value, String> {
    match value.as_u64() {
        Some(5 | 15 | 30 | 60) => Ok(value.clone()),
        _ => Err("must be one of 5, 15, 30, 60".to_string()),
    }
}

/// Claude Code builtin Agent types a user may offer in native multi-agent
/// conversations. Internal types (fork, worker, workflow, ...) never qualify.
pub const CLAUDE_CONFIGURABLE_BUILTIN_AGENTS: [&str; 6] = [
    "general-purpose",
    "Explore",
    "Plan",
    "claude-code-guide",
    "statusline-setup",
    "claude",
];

fn validate_claude_builtin_agents(value: &Value) -> Result<Value, String> {
    let mut agents: Vec<&str> = Vec::new();
    for name in string_items(value) {
        if !CLAUDE_CONFIGURABLE_BUILTIN_AGENTS.contains(&name) {
            return Err(format!("unknown Claude Code builtin agent '{name}'"));
        }
        if !agents.contains(&name) {
            agents.push(name);
        }
    }
    Ok(Value::from(agents))
}

/// Locus agent ids left out of native multi-agent conversations: trimmed,
/// deduplicated. Unknown ids are kept (workspace definitions may add them).
fn validate_locus_agent_ids(value: &Value) -> Result<Value, String> {
    let mut agents: Vec<&str> = Vec::new();
    for name in string_items(value) {
        if name.len() > 128 {
            return Err(format!("agent id '{name}' is too long"));
        }
        if !agents.contains(&name) {
            agents.push(name);
        }
    }
    if agents.len() > 256 {
        return Err("too many agent ids".to_string());
    }
    Ok(Value::from(agents))
}

fn validate_ssh_target(value: &Value) -> Result<Value, String> {
    let target = value.as_str().unwrap_or_default().trim();
    if target.is_empty() {
        return Ok(Value::from(""));
    }
    crate::llm::claude_code_cli::ssh::SshConfig {
        target: target.to_string(),
        port: None,
        identity_file: None,
        known_hosts_file: None,
        cli_path: None,
    }
    .validate()
    .map(|_| Value::from(target))
}

fn one_of(value: &Value, allowed: &[u64]) -> Result<Value, String> {
    match value.as_u64() {
        Some(number) if allowed.contains(&number) => Ok(value.clone()),
        _ => Err(format!(
            "must be one of {}",
            allowed.iter().map(u64::to_string).collect::<Vec<_>>().join(", ")
        )),
    }
}

fn validate_history_page_size(value: &Value) -> Result<Value, String> {
    one_of(value, &[80, 120, 160, 240, 400])
}

fn validate_mounted_session_editors(value: &Value) -> Result<Value, String> {
    one_of(value, &[1, 2, 3, 6, 10, 0])
}

/// Directory names hidden from the Files page: single path segments,
/// trailing slashes dropped, duplicates (case-insensitive) removed.
fn validate_hidden_directories(value: &Value) -> Result<Value, String> {
    let mut names: Vec<String> = Vec::new();
    for raw in string_items(value) {
        let name = raw.trim_end_matches(['/', '\\']);
        if name.is_empty() || name == "." || name == ".." || name.contains(['/', '\\']) {
            return Err(format!("'{raw}' is not a single directory name"));
        }
        if !names.iter().any(|known| known.eq_ignore_ascii_case(name)) {
            names.push(name.to_string());
        }
    }
    Ok(Value::from(names))
}

/// Top tab bar order: page ids and workspace tree system resource ids,
/// trimmed and deduplicated. Unknown ids are kept so a newer build's entries
/// survive an older one; the frontend skips the ones it does not know.
fn validate_top_bar_nodes(value: &Value) -> Result<Value, String> {
    let mut tokens: Vec<&str> = Vec::new();
    for token in string_items(value) {
        if token.len() > 64 {
            return Err(format!("'{token}' is too long"));
        }
        if !tokens.contains(&token) {
            tokens.push(token);
        }
    }
    if tokens.len() > 32 {
        return Err("too many entries".to_string());
    }
    Ok(Value::from(tokens))
}

const SHORTCUT_MODIFIERS: [&str; 4] = ["ctrl", "meta", "alt", "shift"];

/// Keyboard shortcut such as `ctrl+shift+f` or `escape`; empty keeps the
/// platform default. Modifiers come first, the key last.
fn validate_shortcut(value: &Value) -> Result<Value, String> {
    let text = value.as_str().unwrap_or_default().trim().to_ascii_lowercase();
    if text.is_empty() {
        return Ok(Value::from(""));
    }
    let (modifiers, key) = match text.strip_suffix("++") {
        Some(rest) => (rest.split('+').collect::<Vec<_>>(), "+"),
        None => {
            let mut parts = text.split('+').collect::<Vec<_>>();
            let key = parts.pop().unwrap_or_default();
            (parts, key)
        }
    };
    let modifiers = modifiers.into_iter().filter(|part| !part.is_empty()).collect::<Vec<_>>();
    if key.is_empty() || SHORTCUT_MODIFIERS.contains(&key) {
        return Err("expected modifiers followed by a key, e.g. ctrl+shift+f".to_string());
    }
    if let Some(unknown) = modifiers.iter().find(|part| !SHORTCUT_MODIFIERS.contains(part)) {
        return Err(format!("unknown modifier '{unknown}' (use ctrl, meta, alt, shift)"));
    }
    let ordered = SHORTCUT_MODIFIERS
        .iter()
        .filter(|modifier| modifiers.contains(modifier))
        .copied()
        .chain(std::iter::once(key))
        .collect::<Vec<_>>();
    Ok(Value::from(ordered.join("+")))
}

fn validate_comment_author(value: &Value) -> Result<Value, String> {
    let text = value.as_str().unwrap_or_default();
    Ok(Value::from(text.split_whitespace().collect::<Vec<_>>().join(" ")))
}

fn validate_markdown_image_folder(value: &Value) -> Result<Value, String> {
    crate::markdown_attachments::normalize_workspace_folder(value.as_str().unwrap_or_default())
        .map(Value::from)
}

fn validate_markdown_image_rules(value: &Value) -> Result<Value, String> {
    let rules = value.as_object().ok_or_else(|| "expected an object".to_string())?;
    if rules.len() > 64 {
        return Err("at most 64 rules".to_string());
    }
    crate::markdown_attachments::normalize_image_rules(rules).map(Value::Object)
}

const TEXT_VALUE: Kind = Kind::text(512, "");
const PATH_VALUE: Kind = Kind::Path;
const BOOL_VALUE: Kind = Kind::bool(false);
const STRING_LIST_VALUE: Kind = Kind::StringList { default: &[] };
const PERMISSION_MODE_VALUE: Kind = Kind::enumeration(&["auto", "ask"], "auto");
const THEME_VALUE: Kind = Kind::enumeration(&["system", "light", "dark"], "dark");
const FONT_VALUE: Kind = Kind::text(256, "");
const SHORTCUT_VALUE: Kind = Kind::text(64, "");
const REVIEW_TARGET_VALUE: Kind = Kind::enumeration(&["inline", "window"], "window");
const VIEW_OPEN_VALUE: Kind = Kind::enumeration(&["tab", "window"], "tab");
const ASSET_REF_ACTIONS: &[&str] =
    &["unitySelect", "fileBrowser", "unityInspector", "locusInspector"];
/// Inspector variants that older builds stored separately.
const ASSET_REF_ALIASES: &[(&str, &str)] = &[
    ("locusInspectorAuto", "locusInspector"),
    ("locusInspectorEmbedded", "locusInspector"),
    ("locusInspectorWindow", "locusInspector"),
];

define_settings! {
    // ── system ──────────────────────────────────────────────────────────
    SYSTEM_DEBUG: bool = "system.debug", Kind::bool(false),
        sensitivity = Sensitivity::Sensitive;
    SYSTEM_TOOL_FAILURE_LOG: bool = "system.toolFailureLog", Kind::bool(false),
        sensitivity = Sensitivity::Sensitive;
    SYSTEM_CLOSE_BEHAVIOR: String = "system.closeBehavior",
        Kind::enumeration(&["exit", "minimizeToTray"], "exit");
    /// `auto` follows the channel of the installed build.
    UPDATES_CHANNEL: String = "updates.channel",
        Kind::enumeration(&["auto", "stable", "experimental"], "auto"), boot = true;

    // ── agent ───────────────────────────────────────────────────────────
    AGENT_SESSION_UNDO: bool = "agent.sessionUndo", Kind::bool(true);
    AGENT_NEW_SESSION_AGENT_MODE: String = "agent.newSession.agentMode",
        Kind::enumeration(&["remember", "workspace", "fixed"], "remember"), boot = true;
    AGENT_NEW_SESSION_DEFAULT_AGENT_ID: String = "agent.newSession.defaultAgentId",
        Kind::text(128, ""), boot = true;
    AGENT_TOOL_LOADING_MODE: String = "agent.tools.loadingMode",
        Kind::Enum {
            values: &["native", "metaTool", "direct"],
            aliases: &[("meta-tool", "metaTool"), ("meta_tool", "metaTool")],
            default: "native",
        };
    AGENT_LLM_RETRY_MAX_ATTEMPTS: u32 = "agent.llm.retryMaxAttempts",
        Kind::int(0, crate::llm::retry::MAX_RETRIES_LIMIT as i64,
            crate::llm::retry::DEFAULT_MAX_RETRIES as i64);
    AGENT_LLM_STRIP_INLINE_THINK_TAGS: bool = "agent.llm.stripInlineThinkTags", Kind::bool(true);
    AGENT_SUBAGENT_MAX_DEPTH: u32 = "agent.subagent.maxDepth",
        Kind::int(1, crate::config::SUBAGENT_MAX_DEPTH_LIMIT as i64,
            crate::config::DEFAULT_SUBAGENT_MAX_DEPTH as i64);
    AGENT_SUBAGENT_MAX_CONCURRENT: u32 = "agent.subagent.maxConcurrent",
        Kind::int(1, crate::config::SUBAGENT_MAX_CONCURRENT_LIMIT as i64,
            crate::config::DEFAULT_SUBAGENT_MAX_CONCURRENT as i64);

    // ── permissions ─────────────────────────────────────────────────────
    PERMISSIONS_FILE_WORKSPACE_BOUNDARY: bool = "permissions.fileWorkspaceBoundary",
        Kind::bool(false), sensitivity = Sensitivity::Sensitive;
    /// Global approval mode: `auto` runs tools without asking unless a
    /// per-tool or behavior rule asks.
    PERMISSIONS_MODE: String = "permissions.mode", PERMISSION_MODE_VALUE,
        sensitivity = Sensitivity::Sensitive;
    /// Per-tool and per-behavior approval overrides (`read`,
    /// `behavior.knowledge_governance`, ...); missing entries use each
    /// item's default.
    PERMISSIONS_TOOLS: Value = "permissions.tools", Kind::Map { value: &PERMISSION_MODE_VALUE },
        sensitivity = Sensitivity::Sensitive;

    // ── models ──────────────────────────────────────────────────────────
    MODELS_DEFAULTS_MAIN_MODEL: String = "models.defaults.mainModel", TEXT_VALUE;
    MODELS_DEFAULTS_PLAN_MODEL: String = "models.defaults.planModel", TEXT_VALUE;
    MODELS_DEFAULTS_SUBAGENT_MODELS: Value = "models.defaults.subagentModels",
        Kind::Map { value: &STRING_LIST_VALUE };
    MODELS_DEFAULTS_SUBAGENT_EFFORTS: Value = "models.defaults.subagentEfforts",
        Kind::Map { value: &TEXT_VALUE };
    MODELS_DEFAULTS_SUBAGENT_FAST_MODES: Value = "models.defaults.subagentFastModes",
        Kind::Map { value: &BOOL_VALUE };

    // ── providers ───────────────────────────────────────────────────────
    PROVIDERS_ANTHROPIC_NATIVE_LAZY_LOADING: bool = "providers.anthropic.nativeLazyLoading",
        Kind::bool(true);
    PROVIDERS_CLAUDE_CODE_ENABLED: bool = "providers.claudeCode.enabled", Kind::bool(false),
        sensitivity = Sensitivity::Sensitive, experimental = true;
    PROVIDERS_CLAUDE_CODE_PERMISSION_MODE: String = "providers.claudeCode.permissionMode",
        Kind::enumeration(&["auto", "bypassPermissions"], "auto"),
        sensitivity = Sensitivity::Sensitive, experimental = true;
    PROVIDERS_CLAUDE_CODE_IDLE_COMPACTION: bool = "providers.claudeCode.idleCompaction",
        Kind::bool(false), experimental = true;
    PROVIDERS_CLAUDE_CODE_TELEMETRY: bool = "providers.claudeCode.telemetry", Kind::bool(false),
        sensitivity = Sensitivity::Sensitive, experimental = true;
    PROVIDERS_CLAUDE_CODE_AUTO_COMPACT_WINDOW: Value = "providers.claudeCode.autoCompactWindow",
        Kind::OptionalInt { min: 100_000, max: 1_000_000 }, experimental = true;
    PROVIDERS_CLAUDE_CODE_PREWARM: bool = "providers.claudeCode.prewarm", Kind::bool(false),
        experimental = true;
    PROVIDERS_CLAUDE_CODE_POOL_CAPACITY: u32 = "providers.claudeCode.poolCapacity",
        Kind::int(1, 4, 1), experimental = true;
    PROVIDERS_CLAUDE_CODE_IDLE_MINUTES: u32 = "providers.claudeCode.idleMinutes",
        Kind::int(5, 60, 15), experimental = true,
        validate = Some(validate_claude_idle_minutes);
    /// Claude Code conversations delegate through the native Agent tool
    /// (other providers through Locus). Fixed when a conversation starts.
    PROVIDERS_CLAUDE_CODE_NATIVE_MULTI_AGENT: bool = "providers.claudeCode.nativeMultiAgent",
        Kind::bool(true), experimental = true;
    /// Claude Code builtin Agent types offered by the native Agent tool.
    PROVIDERS_CLAUDE_CODE_NATIVE_AGENTS_BUILTINS: Vec<String> =
        "providers.claudeCode.nativeAgents.builtins",
        Kind::StringList { default: &["general-purpose", "Explore", "Plan"] }, experimental = true,
        validate = Some(validate_claude_builtin_agents);
    /// Delegatable Locus agent types the native Agent tool leaves out; the
    /// default offers all of them. Fixed when a conversation starts.
    PROVIDERS_CLAUDE_CODE_NATIVE_AGENTS_EXCLUDED_LOCUS_AGENTS: Vec<String> =
        "providers.claudeCode.nativeAgents.excludedLocusAgents", STRING_LIST_VALUE,
        experimental = true, validate = Some(validate_locus_agent_ids);
    /// SSH target for a remote Claude Code CLI; empty runs it locally. A new
    /// target needs "Test and save" in Settings before it is used.
    PROVIDERS_CLAUDE_CODE_REMOTE_TARGET: String = "providers.claudeCode.remote.target",
        Kind::text(512, ""), sensitivity = Sensitivity::Sensitive, experimental = true,
        validate = Some(validate_ssh_target);
    PROVIDERS_CLAUDE_CODE_REMOTE_PORT: Value = "providers.claudeCode.remote.port",
        Kind::OptionalInt { min: 1, max: 65_535 },
        sensitivity = Sensitivity::Sensitive, experimental = true;
    PROVIDERS_CLAUDE_CODE_REMOTE_IDENTITY_FILE: String =
        "providers.claudeCode.remote.identityFile", Kind::Path,
        sensitivity = Sensitivity::Sensitive, experimental = true;
    PROVIDERS_CLAUDE_CODE_REMOTE_KNOWN_HOSTS_FILE: String =
        "providers.claudeCode.remote.knownHostsFile", Kind::Path,
        sensitivity = Sensitivity::Sensitive, experimental = true;
    PROVIDERS_CLAUDE_CODE_REMOTE_CLI_PATH: String = "providers.claudeCode.remote.cliPath",
        Kind::Path, sensitivity = Sensitivity::Sensitive, experimental = true;
    PROVIDERS_CODEX_AUTO_UPDATE_CLIENT_VERSION: bool =
        "providers.codex.autoUpdateClientVersion", Kind::bool(true);
    PROVIDERS_CODEX_TRANSPORT: String = "providers.codex.transport",
        Kind::enumeration(&["http", "websocket"], "websocket");
    PROVIDERS_CODEX_CONTEXT_WINDOW: u32 = "providers.codex.contextWindow",
        Kind::int(
            crate::commands::MIN_CODEX_CONTEXT_WINDOW as i64,
            crate::commands::MAX_CODEX_CONTEXT_WINDOW as i64,
            crate::commands::DEFAULT_CODEX_CONTEXT_WINDOW as i64,
        );
    PROVIDERS_CODEX_GENERATE_SESSION_TITLES: bool = "providers.codex.generateSessionTitles",
        Kind::bool(false);
    /// Lets a review model approve tool calls instead of asking the user.
    PROVIDERS_CODEX_AUTO_REVIEW: bool = "providers.codex.autoReview", Kind::bool(false),
        sensitivity = Sensitivity::Sensitive;
    PROVIDERS_CODEX_USE_APPLY_PATCH: bool = "providers.codex.useApplyPatch", Kind::bool(false);
    PROVIDERS_CODEX_PREFIX_CACHE_TTL_SECONDS: u32 = "providers.codex.prefixCacheTtlSeconds",
        Kind::int(0, crate::commands::MAX_CODEX_PREFIX_CACHE_TTL_SECONDS as i64,
            crate::commands::DEFAULT_CODEX_PREFIX_CACHE_TTL_SECONDS as i64);

    // ── network and connections ─────────────────────────────────────────
    NETWORK_PROXY_MODE: String = "network.proxy.mode",
        Kind::enumeration(&["auto", "manual", "disabled"], "auto"),
        sensitivity = Sensitivity::Sensitive;
    NETWORK_PROXY_MANUAL_HTTP_PROXY: String = "network.proxy.manual.httpProxy",
        Kind::text(2048, ""), sensitivity = Sensitivity::Sensitive;
    NETWORK_PROXY_MANUAL_HTTPS_PROXY: String = "network.proxy.manual.httpsProxy",
        Kind::text(2048, ""), sensitivity = Sensitivity::Sensitive;
    NETWORK_PROXY_MANUAL_ALL_PROXY: String = "network.proxy.manual.allProxy",
        Kind::text(2048, ""), sensitivity = Sensitivity::Sensitive;
    MCP_SERVER_ENABLED: bool = "mcpServer.enabled", Kind::bool(false),
        sensitivity = Sensitivity::Sensitive;
    MCP_SERVER_PORT: u32 = "mcpServer.port",
        Kind::int(1, 65_535, crate::mcp::server::config::DEFAULT_PORT as i64),
        sensitivity = Sensitivity::Sensitive;
    MCP_SERVER_DISABLED_TOOLS: Vec<String> = "mcpServer.disabledTools", STRING_LIST_VALUE,
        sensitivity = Sensitivity::Sensitive;
    MCP_SERVER_CALL_TIMEOUT_MS: u64 = "mcpServer.callTimeoutMs",
        Kind::int(
            crate::mcp::server::config::MIN_CALL_TIMEOUT_MS as i64,
            crate::mcp::server::config::MAX_CALL_TIMEOUT_MS as i64,
            crate::mcp::server::config::DEFAULT_CALL_TIMEOUT_MS as i64,
        ),
        sensitivity = Sensitivity::Sensitive;
    REMOTE_ACCESS_ENABLED: bool = "remoteAccess.enabled", Kind::bool(false),
        sensitivity = Sensitivity::Sensitive, experimental = true;
    REMOTE_ACCESS_PORT: u32 = "remoteAccess.port",
        Kind::int(1024, 65_535, crate::remote_access::config::DEFAULT_PORT as i64),
        sensitivity = Sensitivity::Sensitive, experimental = true;
    REMOTE_ACCESS_LOOPBACK: bool = "remoteAccess.loopback", Kind::bool(true),
        sensitivity = Sensitivity::Sensitive, experimental = true;
    REMOTE_ACCESS_INTERFACES: Vec<String> = "remoteAccess.interfaces", STRING_LIST_VALUE,
        sensitivity = Sensitivity::Sensitive, experimental = true,
        validate = Some(validate_remote_interfaces);
    REMOTE_ACCESS_HOSTNAMES: Vec<String> = "remoteAccess.hostnames", STRING_LIST_VALUE,
        sensitivity = Sensitivity::Sensitive, experimental = true,
        validate = Some(validate_remote_hostnames);
    REMOTE_ACCESS_REQUIRE_CONFIRMATION: bool = "remoteAccess.requireConfirmation",
        Kind::bool(true), sensitivity = Sensitivity::Sensitive, experimental = true;
    REMOTE_ACCESS_TOKEN_IDLE_DAYS: u32 = "remoteAccess.tokenIdleDays",
        Kind::int(1, 365, crate::remote_access::config::DEFAULT_TOKEN_IDLE_DAYS as i64),
        sensitivity = Sensitivity::Sensitive, experimental = true;
    REMOTE_ACCESS_APP_DOWNLOAD: bool = "remoteAccess.appDownload", Kind::bool(true),
        sensitivity = Sensitivity::Sensitive, experimental = true;

    // ── runtimes ────────────────────────────────────────────────────────
    RUNTIMES_GIT_PATH: String = "runtimes.git.path", Kind::Path,
        sensitivity = Sensitivity::Sensitive, validate = Some(validate_git_path);
    RUNTIMES_PYTHON_SELECTED_ID: String = "runtimes.python.selectedId", Kind::text(512, ""),
        sensitivity = Sensitivity::Sensitive;
    RUNTIMES_PYTHON_SELECTED_PATH: String = "runtimes.python.selectedPath", Kind::Path,
        sensitivity = Sensitivity::Sensitive;

    // ── workspace ───────────────────────────────────────────────────────
    WORKSPACE_WORKTREE_ROOT: String = "workspace.worktree.root", Kind::Path,
        sensitivity = Sensitivity::Sensitive, experimental = true,
        validate = Some(validate_worktree_root);
    WORKSPACE_RESOURCES_WORKSPACE_IDLE_TIMEOUT_SECS: u64 =
        "workspace.resources.workspaceIdleTimeoutSecs", Kind::int(1, MAX_IDLE_TIMEOUT_SECS, 600);
    WORKSPACE_RESOURCES_SERVICE_IDLE_TIMEOUT_SECS: u64 =
        "workspace.resources.serviceIdleTimeoutSecs", Kind::int(1, MAX_IDLE_TIMEOUT_SECS, 3600);
    WORKSPACE_RESOURCES_LSP_IDLE_TIMEOUT_SECS: u64 =
        "workspace.resources.lspIdleTimeoutSecs", Kind::int(1, MAX_IDLE_TIMEOUT_SECS, 600);
    WORKSPACE_WORKTREE_ENABLED: bool = "workspace.worktree.enabled", Kind::bool(false),
        experimental = true, boot = true;
    WORKSPACE_TREE_DISPLAY_MODE: String = "workspace.tree.displayMode",
        Kind::enumeration(&["single", "multi"], "single"), boot = true;
    WORKSPACE_TREE_SECTIONS_KNOWLEDGE: bool = "workspace.tree.sections.knowledge",
        Kind::bool(true), boot = true;
    WORKSPACE_TREE_SECTIONS_COLLAB: bool = "workspace.tree.sections.collab",
        Kind::bool(true), boot = true;
    WORKSPACE_TREE_SECTIONS_ASSETS: bool = "workspace.tree.sections.assets",
        Kind::bool(true), boot = true;
    WORKSPACE_TREE_SECTIONS_VIEWS: bool = "workspace.tree.sections.views",
        Kind::bool(true), boot = true;
    WORKSPACE_TREE_KNOWLEDGE_FOLDERS_PLAN: bool = "workspace.tree.knowledgeFolders.plan",
        Kind::bool(true), boot = true;
    WORKSPACE_TREE_KNOWLEDGE_FOLDERS_MEMORY: bool = "workspace.tree.knowledgeFolders.memory",
        Kind::bool(true), boot = true;
    WORKSPACE_TREE_KNOWLEDGE_FOLDERS_DESIGN: bool = "workspace.tree.knowledgeFolders.design",
        Kind::bool(true), boot = true;
    WORKSPACE_TREE_KNOWLEDGE_FOLDERS_SKILL: bool = "workspace.tree.knowledgeFolders.skill",
        Kind::bool(true), boot = true;
    WORKSPACE_TREE_KNOWLEDGE_FOLDERS_REFERENCE: bool = "workspace.tree.knowledgeFolders.reference",
        Kind::bool(true), boot = true;
    WORKSPACE_TREE_AUTO_PLACE_NEW_PLAN_DESIGN_DOCUMENTS: bool =
        "workspace.tree.autoPlaceNewPlanDesignDocuments",
        Kind::bool(true), boot = true;
    WORKSPACE_TREE_SHOW_UNREAD_INDICATORS: bool = "workspace.tree.showUnreadIndicators",
        Kind::bool(true), boot = true;
    WORKSPACE_TREE_AUTO_PROMOTE_COMPLETED_SESSIONS: bool =
        "workspace.tree.autoPromoteCompletedSessions",
        Kind::bool(true), boot = true;
    /// Single-workspace mode: system nodes shown in the top tab bar, in order
    /// with the page ids (`development`, `plugins`, `settings`); empty keeps
    /// every node in the workspace tree.
    WORKSPACE_TREE_TOP_BAR_NODES: Vec<String> = "workspace.tree.topBarNodes", STRING_LIST_VALUE,
        boot = true, validate = Some(validate_top_bar_nodes);
    WORKSPACE_EXPLORER_HIDDEN_DIRECTORIES: Vec<String> = "workspace.explorer.hiddenDirectories",
        Kind::StringList { default: &[
            ".git", ".vs", ".vscode", ".idea", "node_modules", "__pycache__", ".next", "dist",
            "build", "obj",
        ] },
        boot = true, validate = Some(validate_hidden_directories);
    WORKSPACE_EXPLORER_UNITY_HIDDEN_DIRECTORIES: Vec<String> =
        "workspace.explorer.unityHiddenDirectories",
        Kind::StringList {
            default: &["Library", "Temp", "Logs", "UserSettings", "MemoryCaptures"],
        },
        boot = true, validate = Some(validate_hidden_directories);
    WORKSPACE_SEARCH_ENABLED: bool = "workspace.search.enabled", Kind::bool(true), boot = true;
    WORKSPACE_SEARCH_INCLUDE_ARCHIVED: bool = "workspace.search.includeArchived",
        Kind::bool(true), boot = true;
    WORKSPACE_SEARCH_SOURCES_KNOWLEDGE_TITLE: bool = "workspace.search.sources.knowledgeTitle",
        Kind::bool(true), boot = true;
    WORKSPACE_SEARCH_SOURCES_KNOWLEDGE_CONTENT: bool = "workspace.search.sources.knowledgeContent",
        Kind::bool(true), boot = true;
    WORKSPACE_SEARCH_SOURCES_SESSION_TITLE: bool = "workspace.search.sources.sessionTitle",
        Kind::bool(true), boot = true;
    WORKSPACE_SEARCH_SOURCES_SESSION_CONTENT: bool = "workspace.search.sources.sessionContent",
        Kind::bool(true), boot = true;
    WORKSPACE_SEARCH_SOURCES_FILE_NAME: bool = "workspace.search.sources.fileName",
        Kind::bool(true), boot = true;
    WORKSPACE_SEARCH_SOURCES_VIEW_NAME: bool = "workspace.search.sources.viewName",
        Kind::bool(true), boot = true;
    KNOWLEDGE_DEFAULT_SKILL_PACKAGE_NAMESPACE: String = "knowledge.defaultSkillPackageNamespace",
        Kind::text(128, ""), validate = Some(validate_skill_package_namespace);

    // ── interface (frontend preferences, cached for the first frame) ────
    /// `system` follows the operating system language.
    APPEARANCE_LANGUAGE: String = "appearance.language",
        Kind::enumeration(&["system", "zh", "en"], "system"), boot = true;
    APPEARANCE_THEME_MAIN: String = "appearance.theme.main", THEME_VALUE, boot = true;
    APPEARANCE_THEME_UNITY_EMBED: String = "appearance.theme.unityEmbed", THEME_VALUE, boot = true;
    APPEARANCE_FONTS_UI: String = "appearance.fonts.ui", FONT_VALUE, boot = true;
    APPEARANCE_FONTS_PROSE: String = "appearance.fonts.prose", FONT_VALUE, boot = true;
    APPEARANCE_FONTS_MONO_INLINE: String = "appearance.fonts.monoInline", FONT_VALUE, boot = true;
    APPEARANCE_FONTS_MONO_BLOCK: String = "appearance.fonts.monoBlock", FONT_VALUE, boot = true;
    APPEARANCE_FONTS_MONO_EDITOR: String = "appearance.fonts.monoEditor", FONT_VALUE, boot = true;
    APPEARANCE_CHROME_SHOW_WELCOME_SUBTITLE: bool = "appearance.chrome.showWelcomeSubtitle",
        Kind::bool(true), boot = true;
    APPEARANCE_CHROME_SHOW_PLUGINS_TAB: bool = "appearance.chrome.showPluginsTab",
        Kind::bool(true), boot = true;
    APPEARANCE_CHROME_SHOW_AGENT_TAB: bool = "appearance.chrome.showAgentTab",
        Kind::bool(true), boot = true;
    /// Workspace windows other than the primary one also show the page tab bar.
    APPEARANCE_CHROME_WORKSPACE_WINDOW_TITLE_BAR: bool = "appearance.chrome.workspaceWindowTitleBar",
        Kind::bool(false), boot = true;
    CHAT_MESSAGES_RIGHT_ALIGN_USER: bool = "chat.messages.rightAlignUser",
        Kind::bool(true), boot = true;
    CHAT_MESSAGES_TURN_NAVIGATION_RAIL: bool = "chat.messages.turnNavigationRail",
        Kind::bool(true), boot = true;
    CHAT_MESSAGES_COMPACT_TOOL_CALLS: bool = "chat.messages.compactToolCalls",
        Kind::bool(true), boot = true;
    CHAT_MESSAGES_HIDE_THINKING_BLOCKS: bool = "chat.messages.hideThinkingBlocks",
        Kind::bool(true), boot = true;
    CHAT_COMPOSER_SHOW_AGENT_SELECTOR: bool = "chat.composer.showAgentSelector",
        Kind::bool(true), boot = true;
    CHAT_HISTORY_PAGE_SIZE: u32 = "chat.history.pageSize", Kind::int(80, 400, 120),
        boot = true, validate = Some(validate_history_page_size);
    CHAT_INPUT_SUBMIT_MODE: String = "chat.input.submitMode",
        Kind::enumeration(&["enter-send", "mod-enter-send"], "enter-send"), boot = true;
    CHAT_INPUT_RUNNING_SEND_MODE: String = "chat.input.runningSendMode",
        Kind::enumeration(&["after-run", "insert"], "insert"), boot = true;
    /// Shortcuts such as `ctrl+shift+f`; empty keeps the platform default.
    SHORTCUTS_NEW_CHAT: String = "shortcuts.newChat", SHORTCUT_VALUE,
        boot = true, validate = Some(validate_shortcut);
    SHORTCUTS_CANCEL_RUN: String = "shortcuts.cancelRun", SHORTCUT_VALUE,
        boot = true, validate = Some(validate_shortcut);
    SHORTCUTS_FIND_IN_CONTEXT: String = "shortcuts.findInContext", SHORTCUT_VALUE,
        boot = true, validate = Some(validate_shortcut);
    SHORTCUTS_GLOBAL_SEARCH: String = "shortcuts.globalSearch", SHORTCUT_VALUE,
        boot = true, validate = Some(validate_shortcut);
    /// 0 keeps every session tab mounted.
    WORKBENCH_TABS_MOUNTED_SESSION_EDITORS_PER_GROUP: u32 =
        "workbench.tabs.mountedSessionEditorsPerGroup", Kind::int(0, 10, 6),
        boot = true, validate = Some(validate_mounted_session_editors);
    WORKBENCH_PANELS_TODO_AUTO_OPEN: bool = "workbench.panels.todoAutoOpen",
        Kind::bool(true), boot = true;
    WORKBENCH_PANELS_CHANGES_AUTO_OPEN: bool = "workbench.panels.changesAutoOpen",
        Kind::bool(false), boot = true;
    WORKBENCH_PANELS_CHANGES_AUTO_CLOSE: bool = "workbench.panels.changesAutoClose",
        Kind::bool(true), boot = true;
    WORKBENCH_PANELS_FILE_CHANGE_POPOVER: bool = "workbench.panels.fileChangePopover",
        Kind::bool(true), boot = true;
    WORKBENCH_OPEN_CHAT_DIFF_REVIEW: String = "workbench.open.chatDiffReview",
        REVIEW_TARGET_VALUE, boot = true;
    WORKBENCH_OPEN_GIT_DIFF_REVIEW: String = "workbench.open.gitDiffReview",
        REVIEW_TARGET_VALUE, boot = true;
    WORKBENCH_OPEN_PLAN_APPROVAL: String = "workbench.open.planApproval",
        Kind::enumeration(&["card", "window"], "card"), boot = true;
    WORKBENCH_OPEN_MEMORY_FILE: String = "workbench.open.memoryFile",
        Kind::enumeration(&["window", "knowledge"], "knowledge"), boot = true;
    WORKBENCH_OPEN_VIEW: String = "workbench.open.view", VIEW_OPEN_VALUE, boot = true;
    /// Per-View open target, keyed by workspace and View id.
    WORKBENCH_OPEN_VIEW_OVERRIDES: Value = "workbench.open.viewOverrides",
        Kind::Map { value: &VIEW_OPEN_VALUE }, boot = true;
    WORKBENCH_OPEN_ASSET_REF_CLICK: String = "workbench.open.assetRefClick",
        Kind::Enum {
            values: ASSET_REF_ACTIONS,
            aliases: ASSET_REF_ALIASES,
            default: "locusInspector",
        },
        boot = true;
    WORKBENCH_OPEN_UNITY_EMBED_ASSET_REF_CLICK: String = "workbench.open.unityEmbedAssetRefClick",
        Kind::Enum {
            values: ASSET_REF_ACTIONS,
            aliases: ASSET_REF_ALIASES,
            default: "unityInspector",
        },
        boot = true;
    WORKBENCH_VIEWS_SHOW_IN_SESSION_PANEL: bool = "workbench.views.showInSessionPanel",
        Kind::bool(false), boot = true;
    WORKBENCH_GIT_MERGE_TREE_STATUS_ICON: bool = "workbench.git.mergeTreeStatusIcon",
        Kind::bool(true), boot = true;
    WORKBENCH_GIT_HIDE_COMMAND_SUGGESTIONS: bool = "workbench.git.hideCommandSuggestions",
        Kind::bool(false), boot = true;
    DOCUMENTS_PUNCTUATION_PAIRING: bool = "documents.punctuationPairing",
        Kind::bool(false), boot = true;
    DOCUMENTS_COMMENT_AUTHOR: String = "documents.commentAuthor", Kind::text(64, ""),
        boot = true, validate = Some(validate_comment_author);
    NOTIFICATIONS_SYSTEM_ENABLED: bool = "notifications.system.enabled",
        Kind::bool(true), boot = true;
    NOTIFICATIONS_SYSTEM_ON_CHAT_DONE: bool = "notifications.system.onChatDone",
        Kind::bool(true), boot = true;
    NOTIFICATIONS_SYSTEM_ON_SUBAGENT_DONE: bool = "notifications.system.onSubagentDone",
        Kind::bool(false), boot = true;
    NOTIFICATIONS_SYSTEM_ON_ASK_USER: bool = "notifications.system.onAskUser",
        Kind::bool(true), boot = true;
    NOTIFICATIONS_SYSTEM_ON_CHAT_ERROR: bool = "notifications.system.onChatError",
        Kind::bool(true), boot = true;
    NOTIFICATIONS_SYSTEM_ON_TOOL_CONFIRM: bool = "notifications.system.onToolConfirm",
        Kind::bool(true), boot = true;
    NOTIFICATIONS_SOUND_ENABLED: bool = "notifications.sound.enabled",
        Kind::bool(false), boot = true;
    NOTIFICATIONS_SOUND_MODE: String = "notifications.sound.mode",
        Kind::enumeration(&["soft", "bright", "urgent"], "bright"), boot = true;
    NOTIFICATIONS_SOUND_SOURCE: String = "notifications.sound.source",
        Kind::enumeration(&["builtin", "custom"], "builtin"), boot = true;
    /// Local audio file played when the source is `custom`.
    NOTIFICATIONS_SOUND_CUSTOM_FILE: String = "notifications.sound.customFile", Kind::Path,
        sensitivity = Sensitivity::Sensitive, boot = true;
    NOTIFICATIONS_SOUND_VOLUME: u32 = "notifications.sound.volume", Kind::int(0, 100, 50),
        boot = true;
    NOTIFICATIONS_SOUND_ON_CHAT_DONE: bool = "notifications.sound.onChatDone",
        Kind::bool(true), boot = true;
    NOTIFICATIONS_SOUND_ON_SUBAGENT_DONE: bool = "notifications.sound.onSubagentDone",
        Kind::bool(false), boot = true;
    NOTIFICATIONS_SOUND_ON_ASK_USER: bool = "notifications.sound.onAskUser",
        Kind::bool(true), boot = true;
    NOTIFICATIONS_SOUND_ON_CHAT_ERROR: bool = "notifications.sound.onChatError",
        Kind::bool(true), boot = true;
    NOTIFICATIONS_SOUND_ON_TOOL_CONFIRM: bool = "notifications.sound.onToolConfirm",
        Kind::bool(true), boot = true;
    NOTIFICATIONS_HINTS_CACHE_INVALIDATION_WARNINGS: bool =
        "notifications.hints.cacheInvalidationWarnings",
        Kind::bool(false), boot = true;

    // ── project (stored in <project>/Locus/config.json) ──────────────────
    PROJECT_KNOWLEDGE_ENABLED: bool = "project.knowledgeEnabled", Kind::bool(true),
        scope = Scope::Project, project_field = Some("knowledge_enabled");
    PROJECT_UNITY_TEST_TOOLS_ENABLED: bool = "project.unityTestToolsEnabled", Kind::bool(false),
        scope = Scope::Project, project_field = Some("unity_test_tools_enabled");
    /// Workspace-relative folder for images pasted into Markdown documents;
    /// empty until the user chooses one (the first paste asks).
    PROJECT_MARKDOWN_IMAGES_DIRECTORY: String = "project.markdownImages.directory", Kind::Path,
        scope = Scope::Project, project_field = Some("markdown_image_directory"),
        validate = Some(validate_markdown_image_folder);
    /// `{ document folder: image folder }`, both workspace-relative; the
    /// longest matching document folder wins over the default folder.
    PROJECT_MARKDOWN_IMAGES_RULES: Value = "project.markdownImages.rules",
        Kind::Map { value: &PATH_VALUE }, scope = Scope::Project,
        project_field = Some("markdown_image_rules"),
        validate = Some(validate_markdown_image_rules);

    // ── unity ───────────────────────────────────────────────────────────
    UNITY_ENABLED: bool = "unity.enabled", Kind::bool(true);
    UNITY_EMBED_ENABLED: bool = "unity.embed.enabled", Kind::bool(true);
    UNITY_CONNECTION_NATIVE_BRIDGE: bool = "unity.connection.nativeBridge", Kind::bool(true);
    UNITY_CONNECTION_BACKGROUND_HOOK: bool = "unity.connection.backgroundHook", Kind::bool(true);
    UNITY_CONNECTION_STATE_PROBE: bool = "unity.connection.stateProbe", Kind::bool(true);
    UNITY_EXTERNAL_EDITOR_AUTO_SELECT: bool = "unity.externalEditor.autoSelect", Kind::bool(false);
    UNITY_CODE_ANALYSIS_CSHARP_LSP: bool = "unity.codeAnalysis.csharpLsp", Kind::bool(false);
    UNITY_CODE_ANALYSIS_SYMBOL_SEARCH: bool =
        "unity.codeAnalysis.tools.codeSymbolSearch", Kind::bool(true);
    UNITY_CODE_ANALYSIS_GOTO_DEFINITION: bool =
        "unity.codeAnalysis.tools.codeGotoDefinition", Kind::bool(true);
    UNITY_CODE_ANALYSIS_FIND_REFERENCES: bool =
        "unity.codeAnalysis.tools.codeFindReferences", Kind::bool(true);
    UNITY_CODE_ANALYSIS_DIAGNOSTICS: bool =
        "unity.codeAnalysis.tools.codeDiagnostics", Kind::bool(false);
    UNITY_CODE_ANALYSIS_EDIT_WRITE_DIAGNOSTICS: bool =
        "unity.codeAnalysis.tools.editWriteDiagnostics", Kind::bool(true);
    UNITY_CODE_ANALYSIS_HOVER: bool = "unity.codeAnalysis.tools.codeHover", Kind::bool(false);
    UNITY_CODE_ANALYSIS_UNITY_CODE_USAGES: bool =
        "unity.codeAnalysis.tools.unityCodeUsages", Kind::bool(true);
    UNITY_CODE_ANALYSIS_UNITY_ANALYZERS: bool =
        "unity.codeAnalysis.tools.unityAnalyzers", Kind::bool(true);
    UNITY_COMPILE_SIDECAR: bool = "unity.compile.sidecar", Kind::bool(true);
    UNITY_COMPILE_IN_PROCESS_FALLBACK: bool = "unity.compile.inProcessFallback", Kind::bool(true);
    UNITY_COMPILE_NON_PUBLIC_ACCESS: bool = "unity.compile.nonPublicAccess", Kind::bool(true),
        sensitivity = Sensitivity::Sensitive;
    UNITY_HOT_RELOAD_ENABLED: bool = "unity.hotReload.enabled", Kind::bool(false);
    UNITY_HOT_RELOAD_INLINE_FORCE_EVALUATE: bool =
        "unity.hotReload.inlineForceEvaluate", Kind::bool(true);
    UNITY_MULTI_AGENT_EDITOR: bool = "unity.multiAgentEditor", Kind::bool(false),
        experimental = true;

    // ── blender ─────────────────────────────────────────────────────────
    BLENDER_ENABLED: bool = "blender.enabled", Kind::bool(false);
    BLENDER_RESOURCES_MAX_BACKGROUND_INSTANCES: u32 =
        "blender.resources.maxBackgroundInstances", Kind::int(1, 8, 2);
    BLENDER_RESOURCES_BACKGROUND_IDLE_TIMEOUT_SECS: u64 =
        "blender.resources.backgroundIdleTimeoutSecs", Kind::int(1, MAX_IDLE_TIMEOUT_SECS, 600);

    // ── latex ───────────────────────────────────────────────────────────
    LATEX_ENGINE: String = "latex.engine",
        Kind::enumeration(&["auto", "xelatex", "pdflatex", "lualatex"], "auto");
    LATEX_PREVIEW_SVG: bool = "latex.preview.svg", Kind::bool(true);
    LATEX_TOOLCHAIN_ALLOW_PACKAGE_INSTALL: bool =
        "latex.toolchain.allowPackageInstall", Kind::bool(false);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn keys_are_unique_and_well_formed() {
        let mut seen = HashSet::new();
        for def in DEFINITIONS {
            assert!(seen.insert(def.key), "duplicate setting key {}", def.key);
            assert!(
                def.key.split('.').all(|segment| !segment.is_empty()
                    && segment.chars().all(|ch| ch.is_ascii_alphanumeric())),
                "malformed setting key {}",
                def.key
            );
            assert!(
                def.normalize(&def.default_value()).is_ok(),
                "default of {} fails its own validation",
                def.key
            );
            assert_eq!(
                def.scope == Scope::Project,
                def.project_field.is_some(),
                "project settings need a project field: {}",
                def.key
            );
        }
    }

    #[test]
    fn no_key_is_a_prefix_of_another() {
        // A setting value cannot also be an object holding other settings.
        for outer in DEFINITIONS {
            let prefix = format!("{}.", outer.key);
            for inner in DEFINITIONS {
                assert!(
                    !inner.key.starts_with(&prefix),
                    "{} is nested under setting {}",
                    inner.key,
                    outer.key
                );
            }
        }
    }

    #[test]
    fn shortcuts_are_normalized_to_ordered_lowercase_combinations() {
        use serde_json::json;
        let def = &DEFINITIONS[SHORTCUTS_GLOBAL_SEARCH.index()];
        let normalize = |value: Value| def.normalize(&value);
        assert_eq!(normalize(json!("")).unwrap(), json!(""));
        assert_eq!(normalize(json!("Shift+Ctrl+F")).unwrap(), json!("ctrl+shift+f"));
        assert_eq!(normalize(json!("escape")).unwrap(), json!("escape"));
        assert_eq!(normalize(json!("ctrl++")).unwrap(), json!("ctrl++"));
        assert!(normalize(json!("ctrl+shift")).is_err());
        assert!(normalize(json!("hyper+f")).is_err());
    }

    #[test]
    fn excluded_locus_agents_are_trimmed_and_deduplicated() {
        use serde_json::json;
        let def = &DEFINITIONS[PROVIDERS_CLAUDE_CODE_NATIVE_AGENTS_EXCLUDED_LOCUS_AGENTS.index()];
        assert_eq!(def.default_value(), json!([]));
        assert_eq!(
            def.normalize(&json!([" unity ", "explorer", "", "unity"])).unwrap(),
            json!(["unity", "explorer"])
        );
        assert!(def.normalize(&json!(["x".repeat(129)])).is_err());
    }

    #[test]
    fn top_bar_nodes_are_trimmed_and_deduplicated_but_keep_unknown_ids() {
        use serde_json::json;
        let def = &DEFINITIONS[WORKSPACE_TREE_TOP_BAR_NODES.index()];
        assert_eq!(def.default_value(), json!([]));
        assert_eq!(
            def.normalize(&json!([" development ", "collaboration", "", "future", "collaboration"])).unwrap(),
            json!(["development", "collaboration", "future"])
        );
        assert!(def.normalize(&json!(["x".repeat(65)])).is_err());
    }

    #[test]
    fn hidden_directories_must_be_single_names() {
        use serde_json::json;
        let def = &DEFINITIONS[WORKSPACE_EXPLORER_HIDDEN_DIRECTORIES.index()];
        assert_eq!(
            def.normalize(&json!([" dist/ ", "Dist", "", "node_modules"])).unwrap(),
            json!(["dist", "node_modules"])
        );
        assert!(def.normalize(&json!(["src/generated"])).is_err());
        assert!(def.normalize(&json!([".."])).is_err());
    }

    #[test]
    fn frontend_preferences_are_cached_for_the_first_frame() {
        for def in DEFINITIONS {
            const FRONTEND_PREFIXES: [&str; 6] = [
                "appearance.",
                "chat.",
                "shortcuts.",
                "workbench.",
                "documents.",
                "notifications.",
            ];
            let frontend = FRONTEND_PREFIXES.iter().any(|prefix| def.key.starts_with(prefix));
            if frontend {
                assert!(def.boot, "{} is read by the frontend but not marked boot", def.key);
            }
        }
    }

    #[test]
    fn typed_keys_point_at_their_definition() {
        assert_eq!(DEFINITIONS[SYSTEM_DEBUG.index()].key, SYSTEM_DEBUG.name());
        assert_eq!(
            DEFINITIONS[BLENDER_RESOURCES_BACKGROUND_IDLE_TIMEOUT_SECS.index()].key,
            BLENDER_RESOURCES_BACKGROUND_IDLE_TIMEOUT_SECS.name()
        );
    }
}
