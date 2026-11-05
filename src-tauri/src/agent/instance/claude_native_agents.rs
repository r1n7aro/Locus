//! Claude Code native multi-agent route: one `Agent` entry whose subagent
//! types route by the target's resolved backend. Claude Code targets on the
//! conversation's own connection run as native Claude subagents; every other
//! target runs as a Locus subagent. The route (tool catalog) and the published
//! directory (session tool contract) are pinned per conversation, so later
//! setting changes never rewrite an existing tool prefix. Calls are checked
//! against the current settings and fail with the current choices; a target
//! is never silently replaced by another backend or by `inherit`.
use super::*;
use crate::session::store::SessionToolContract;

#[cfg(test)]
#[path = "claude_native_agents_tests.rs"]
mod tests;

pub(super) const DIRECTORY_CONTRACT: &str = "claude_native_agents";
const DIRECTORY_VERSION: u32 = 1;
/// Catalog-only prompt of a Locus-routed type. Never executed by Claude.
const LOCUS_ROUTED_PROMPT: &str = "Locus runs this agent type on another provider. Claude Code never executes it.";
/// Ends every native definition's prompt: who executes its tools.
const NATIVE_TOOLS_NOTE: &str = "Your tools run through Locus on this machine, with the delegating agent's permissions. Work inside the working directory.";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum DelegationBackend {
    ClaudeNative,
    Locus,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DelegationEntry {
    /// Definition name the Mod registers; Claude exposes it as `locus:<name>`.
    pub name: String,
    pub backend: DelegationBackend,
    /// Locus agent type and the full model id this entry is bound to.
    pub agent_type: String,
    pub model: String,
    pub description: String,
    /// Native definition (ClaudeNative only).
    #[serde(default)]
    pub prompt: String,
    #[serde(default)]
    pub tools: Vec<String>,
    #[serde(default)]
    pub native_model: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DelegationDirectory {
    pub version: u32,
    /// Claude Code builtin types offered through `agent.offer`.
    pub builtins: Vec<String>,
    pub entries: Vec<DelegationEntry>,
}

/// Where every Claude subagent's tools run: Locus executes them on this
/// machine, never on the Claude host. Builtin subagents receive it in place
/// of Claude's own environment attachment, with the same path and working
/// directory rules as the main agent.
pub(super) fn claude_subagent_environment(working_dir: &str) -> String {
    format!(
        "{}\n\n# Environment\nOS: {}\nShell: {}\nWorking directory: {working_dir}\nYour tools run through Locus on this machine, with the delegating agent's permissions. Work inside the working directory.",
        crate::prompt::claude_code_cli::TOOL_ENVIRONMENT.trim(),
        std::env::consts::OS,
        crate::tool::builtins::shell_display_name(),
    )
}

impl DelegationDirectory {
    /// `/options` "agents": definitions to register, builtins to offer and
    /// the subagents' tool environment.
    pub(crate) fn mod_payload(&self, environment: &str) -> serde_json::Value {
        let definitions = self
            .entries
            .iter()
            .map(|entry| match entry.backend {
                DelegationBackend::ClaudeNative => serde_json::json!({
                    "name": entry.name, "backend": "native", "description": entry.description,
                    "prompt": entry.prompt, "tools": entry.tools, "model": entry.native_model,
                }),
                // Catalog-only: the Mod forwards these calls to Locus before
                // native dispatch, and agent.spawn refuses them as a backstop.
                DelegationBackend::Locus => serde_json::json!({
                    "name": entry.name, "backend": "locus", "description": entry.description,
                    "prompt": LOCUS_ROUTED_PROMPT, "tools": entry.tools, "model": "inherit",
                }),
            })
            .collect::<Vec<_>>();
        serde_json::json!({"version": self.version, "builtins": self.builtins, "definitions": definitions, "environment": environment})
    }

    /// Entry for an `Agent.subagent_type` (`locus:<name>` or `<name>`).
    pub(crate) fn entry_for(&self, subagent_type: &str) -> Option<&DelegationEntry> {
        let name = subagent_type
            .strip_prefix("locus:")
            .unwrap_or(subagent_type);
        self.entries.iter().find(|entry| entry.name == name)
    }
}

/// Connection a model runs on: a Locus Server binding id, or `None` for the
/// configured local/SSH Claude Code connection.
fn claude_connection_key(model: &str) -> Option<&str> {
    model
        .strip_prefix("locus-server/")
        .and_then(|rest| rest.split_once('/'))
        .map(|(id, _)| id)
}

fn native_model_id(model: &str) -> String {
    #[cfg(feature = "server-client")]
    let model = crate::server_client::provider_model_id(model);
    crate::llm::claude_code_cli::normalize_claude_model(model)
}

fn sanitize_name(value: &str) -> String {
    let mut name = String::new();
    for ch in value.chars() {
        if ch.is_ascii_alphanumeric() {
            name.push(ch.to_ascii_lowercase());
        } else if !name.ends_with('-') {
            name.push('-');
        }
    }
    name.trim_matches('-').to_string()
}

/// Native tool names for a Locus agent definition, limited to what the
/// parent may execute (subagent calls run with the parent's authority).
/// Delegation and planning tools are never offered to native subagents.
pub(super) fn native_agent_tools(
    locus_tools: &[String],
    allowed: &HashSet<String>,
    native_tool_search: bool,
) -> Vec<String> {
    let mut tools = Vec::new();
    for name in locus_tools {
        // Messages go to the main agent only, through the host's mailbox; the
        // parent's catalog never lists send_message in a native conversation.
        if name == "send_message" {
            if !tools.iter().any(|tool| tool == "SendMessage") {
                tools.push("SendMessage".to_string());
            }
            continue;
        }
        if !allowed.contains(name) {
            continue;
        }
        let native = match name.as_str() {
            "subagent" | "explore" | "task" | "todowrite" | "tool_load"
            | "tool_call" | "exit_plan_mode" => continue,
            "read" => "Read".to_string(),
            "write" => "Write".to_string(),
            "edit" => "Edit".to_string(),
            "bash" => "Bash".to_string(),
            "grep" => "Grep".to_string(),
            "list" => "Glob".to_string(),
            "ask_user_question" => "AskUserQuestion".to_string(),
            "web_fetch" => "WebFetch".to_string(),
            other => format!("mcp__locus__{other}"),
        };
        if !tools.contains(&native) {
            tools.push(native);
        }
    }
    // Deferred Locus tools are reached through ToolSearch.
    if native_tool_search && tools.iter().any(|tool| tool.starts_with("mcp__locus__")) {
        tools.push("ToolSearch".to_string());
    }
    tools
}

/// Locus tool names a native subagent must never execute, whatever its
/// definition says: no delegation, planning control or meta-tool dispatch.
pub(super) fn native_agent_tool_refusal(tool_name: &str) -> Option<String> {
    matches!(
        tool_name,
        "subagent" | "send_message" | "explore" | "task" | "todowrite" | "tool_load" | "tool_call"
            | "exit_plan_mode"
    )
    .then(|| format!("Tool '{tool_name}' is not available to Claude Code subagents."))
}

impl AgentInstance {
    pub(super) fn claude_delegation(&self) -> Option<Arc<DelegationDirectory>> {
        self.claude_delegation
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    /// Resolves this run's delegation route. Call once per run after the lazy
    /// tool renderer is refreshed and before tools or prompts are assembled.
    pub(super) async fn prepare_claude_delegation(
        &self,
        app: &AppHandle,
        store: &SessionStore,
    ) -> Result<(), String> {
        let directory = self.resolve_claude_delegation(app, store).await?;
        *self
            .claude_delegation
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = directory.map(Arc::new);
        Ok(())
    }

    async fn resolve_claude_delegation(
        &self,
        app: &AppHandle,
        store: &SessionStore,
    ) -> Result<Option<DelegationDirectory>, String> {
        // Only a top-level Claude Code conversation owns a native route.
        if !matches!(self.backend, LlmBackend::ClaudeCodeCli)
            || self.parent_tool_call.is_some()
            || self.subagent_depth > 0
            || self.session_id.starts_with("__")
        {
            return Ok(None);
        }
        let resuming =
            crate::llm::claude_code_cli::recovery::native_session_id(store, &self.session_id)?
                .is_some();
        let native_tool_search =
            self.cached_lazy_tool_renderer() == LazyToolRenderer::ClaudeCliNative;
        let unified = crate::llm::claude_code_cli::tool_catalog::pin_multi_agent_route(
            &self.session_id,
            resuming,
            native_tool_search,
            || self.claude_native_route_wanted(),
        )?;
        if !unified || !self.multi_agent_enabled {
            return Ok(None);
        }
        self.pinned_claude_delegation_directory(app, store, native_tool_search)
            .await
            .map(Some)
    }

    /// Setting and version gate for a new conversation's route.
    fn claude_native_route_wanted(&self) -> bool {
        let Ok(settings) = crate::llm::claude_code_cli::ssh::load_settings() else {
            return false;
        };
        if !settings.native_multi_agent {
            return false;
        }
        // A Locus Server runs and validates its own Claude Code.
        if claude_connection_key(&self.effective_model).is_some() {
            return true;
        }
        let version = crate::llm::claude_code_cli::native_agents_cli_version(&settings);
        let supported = version.as_deref().is_some_and(|version| {
            crate::llm::claude_code_cli::version_at_least(
                version,
                crate::llm::claude_code_cli::NATIVE_AGENTS_MIN_VERSION,
            )
        });
        if !supported {
            eprintln!(
                "[Agent {}] Claude Code {} does not support native agents; session {} uses Locus subagents",
                self.id,
                version.as_deref().unwrap_or("(unknown version)"),
                self.session_id
            );
        }
        supported
    }

    async fn pinned_claude_delegation_directory(
        &self,
        app: &AppHandle,
        store: &SessionStore,
        native_tool_search: bool,
    ) -> Result<DelegationDirectory, String> {
        if let Some(contract) = store.session_tool_contract(&self.session_id, DIRECTORY_CONTRACT)? {
            if let Ok(directory) = serde_json::from_value::<DelegationDirectory>(contract.parameters) {
                if directory.version == DIRECTORY_VERSION {
                    return Ok(directory);
                }
            }
            return Err("The saved Claude agent directory is unreadable. Start a new conversation; no agent definition was replaced.".into());
        }
        let selection = crate::llm::claude_code_cli::ssh::load_settings()
            .map(|settings| settings.native_agents)
            .unwrap_or_default();
        let builtins = selection
            .builtins
            .into_iter()
            .filter(|name| {
                crate::settings::definitions::CLAUDE_CONFIGURABLE_BUILTIN_AGENTS
                    .contains(&name.as_str())
            })
            .collect();
        let directory = self
            .build_claude_delegation_directory(
                native_tool_search,
                builtins,
                &selection.excluded_locus_agents,
                |model| async move {
                    crate::commands::resolve_model_backend_for_app(app, &model)
                        .await
                        .map_err(|error| error.to_string())
                },
            )
            .await;
        let contract = SessionToolContract {
            description: format!("claude native agents v{DIRECTORY_VERSION}"),
            parameters: serde_json::to_value(&directory).map_err(|e| e.to_string())?,
        };
        let pinned = store.pin_session_tool_contract(&self.session_id, DIRECTORY_CONTRACT, &contract)?;
        serde_json::from_value(pinned.parameters)
            .map_err(|error| format!("Cannot read the pinned Claude agent directory: {error}"))
    }

    /// A native subagent's system prompt: its definition rendered in this
    /// workspace as Locus renders a subagent (base prompt, enabled rules,
    /// knowledge context, the bridged tool rules and environment, in the
    /// order of the main Claude Code prompt). It is pinned with the
    /// directory, so later rule edits reach new conversations.
    async fn native_subagent_prompt(&self, def: &AgentDef) -> String {
        let parts = self.build_system_prompt_parts_for(def, true).await;
        let tool_environment = crate::prompt::claude_code_cli::TOOL_ENVIRONMENT.trim();
        [parts.base_prompt.trim(), parts.rules_prompt.trim(), parts.knowledge_prompt.trim(), tool_environment, parts.env_prompt.trim(), NATIVE_TOOLS_NOTE]
            .into_iter()
            .filter(|part| !part.is_empty())
            .collect::<Vec<_>>()
            .join("\n\n")
    }

    /// One entry per delegatable agent type and allowed model, except the
    /// types the user left out of native conversations. The resolver is
    /// injected so tests do not need account state.
    pub(super) async fn build_claude_delegation_directory<F, Fut>(
        &self,
        native_tool_search: bool,
        builtins: Vec<String>,
        excluded_locus_agents: &[String],
        resolve_backend: F,
    ) -> DelegationDirectory
    where
        F: Fn(String) -> Fut,
        Fut: std::future::Future<Output = Result<LlmBackend, String>>,
    {
        let allowed = self.allowed_tool_set().await;
        let parent_connection = claude_connection_key(&self.effective_model);
        let mut entries: Vec<DelegationEntry> = Vec::new();
        for (agent_type, description) in self
            .registry
            .list_subagent_descriptions_for_project_type(self.workspace_project_type())
        {
            if excluded_locus_agents.contains(&agent_type) {
                continue;
            }
            let Some(def) = self.registry.get(&agent_type).cloned() else {
                continue;
            };
            let models = self.subagent_model_choices(&agent_type);
            let labels = Self::subagent_model_labels(&models);
            // Rendered once per type, only if one of its models runs natively.
            let mut native_prompt: Option<String> = None;
            for (model, label) in models.iter().zip(&labels) {
                let backend = match resolve_backend(model.clone()).await {
                    Ok(LlmBackend::ClaudeCodeCli)
                        if claude_connection_key(model) == parent_connection =>
                    {
                        DelegationBackend::ClaudeNative
                    }
                    Ok(_) => DelegationBackend::Locus,
                    // Unavailable now: calls report the current choices.
                    Err(error) => {
                        eprintln!(
                            "[Agent {}] Claude agent directory skips {agent_type} on {model}: {error}",
                            self.id
                        );
                        continue;
                    }
                };
                let base = sanitize_name(&agent_type);
                let mut name = if models.len() > 1 {
                    format!("{base}-{}", sanitize_name(label))
                } else {
                    base
                };
                if name.is_empty() || entries.iter().any(|entry| entry.name == name) {
                    name = format!("{}-{}", if name.is_empty() { "agent" } else { &name }, entries.len() + 1);
                }
                let summary = description.trim();
                // A Locus type's tools are shown in the catalog only; Locus
                // runs it with its own definition.
                // The directory exists only with Multi-Agent, where every
                // Locus subagent gets send_message (resolve_configured_tool_names);
                // a native definition keeps that channel to the main agent.
                let mut locus_tools = def.tools.clone();
                push_unique_tool_name(&mut locus_tools, super::multi_agent::SEND_MESSAGE_TOOL);
                let tools = native_agent_tools(&locus_tools, &allowed, native_tool_search);
                let (description, prompt, native_model) = match backend {
                    DelegationBackend::ClaudeNative => (
                        format!("{summary} Model: {label}."),
                        match &native_prompt {
                            Some(prompt) => prompt.clone(),
                            None => native_prompt.insert(self.native_subagent_prompt(&def).await).clone(),
                        },
                        native_model_id(model),
                    ),
                    DelegationBackend::Locus => (
                        format!("{summary} Model: {label} (runs as a Locus subagent)."),
                        String::new(),
                        String::new(),
                    ),
                };
                entries.push(DelegationEntry {
                    name,
                    backend,
                    agent_type: agent_type.clone(),
                    model: model.clone(),
                    description,
                    prompt,
                    tools,
                    native_model,
                });
            }
        }
        DelegationDirectory {
            version: DIRECTORY_VERSION,
            builtins,
            entries,
        }
    }

    /// Published agent types that the current settings still allow.
    pub(super) fn claude_agent_choices(&self, directory: &DelegationDirectory) -> String {
        let mut names = directory
            .entries
            .iter()
            .filter(|entry| {
                self.resolve_delegation_target(&entry.agent_type).is_ok()
                    && self.subagent_model_choices(&entry.agent_type).contains(&entry.model)
            })
            .map(|entry| format!("`locus:{}`", entry.name))
            .collect::<Vec<_>>();
        names.extend(directory.builtins.iter().map(|name| format!("`{name}`")));
        if names.is_empty() {
            "No agent type is currently available for delegation in this conversation.".into()
        } else {
            format!("Currently available subagent_type values: {}.", names.join(", "))
        }
    }

    /// Locus `subagent` arguments for an Agent call the Mod routed to Locus,
    /// checked against the current settings.
    pub(super) fn claude_locus_agent_call(
        &self,
        directory: &DelegationDirectory,
        input: &serde_json::Value,
    ) -> Result<(DelegationEntry, serde_json::Value), String> {
        let subagent_type = input["subagent_type"]
            .as_str()
            .ok_or("Agent requires subagent_type")?;
        let entry = directory
            .entry_for(subagent_type)
            .filter(|entry| entry.backend == DelegationBackend::Locus)
            .ok_or_else(|| {
                format!(
                    "Agent type '{subagent_type}' does not run in Locus. {}",
                    self.claude_agent_choices(directory)
                )
            })?
            .clone();
        if let Some(error) = unsupported_agent_option(input) {
            return Err(error);
        }
        if input.get("model").is_some_and(|value| !value.is_null()) {
            return Err(format!(
                "Agent type '{subagent_type}' always uses {}. Omit model, or choose another subagent_type.",
                entry.model
            ));
        }
        let prompt = input["prompt"]
            .as_str()
            .filter(|prompt| !prompt.trim().is_empty())
            .ok_or("Agent requires a non-empty prompt")?;
        // Current settings decide availability; the published route never
        // falls back to another model or to the parent's model.
        self.resolve_delegation_target(&entry.agent_type).map_err(|_| {
            format!(
                "Agent type '{subagent_type}' is no longer available for delegation. {}",
                self.claude_agent_choices(directory)
            )
        })?;
        if !self
            .subagent_model_choices(&entry.agent_type)
            .contains(&entry.model)
        {
            return Err(format!(
                "Model '{}' is no longer available for agent type '{}'. {}",
                entry.model,
                entry.agent_type,
                self.claude_agent_choices(directory)
            ));
        }
        let args = serde_json::json!({
            "subagent_type": entry.agent_type,
            "description": input["description"].as_str().unwrap_or("Delegated task"),
            "prompt": prompt,
            "model": entry.model,
        });
        Ok((entry, args))
    }
}

/// Where each Agent type of a conversation runs, for its Agent cards: the
/// pinned directory's types with their backend and bound model, and the
/// offered Claude Code builtins (whose model Claude Code chooses). `None`
/// before the first run and for conversations without the native route.
pub(crate) fn claude_agent_routes(store: &SessionStore, session_id: &str) -> Result<Option<serde_json::Value>, String> {
    let Some(contract) = store.session_tool_contract(session_id, DIRECTORY_CONTRACT)? else {
        return Ok(None);
    };
    let Ok(directory) = serde_json::from_value::<DelegationDirectory>(contract.parameters) else {
        return Ok(None);
    };
    let types: Vec<_> = directory
        .entries
        .iter()
        .map(|entry| {
            serde_json::json!({
                "subagentType": format!("locus:{}", entry.name),
                "agentType": entry.agent_type,
                "backend": entry.backend,
                "model": entry.model,
                "modelLabel": AgentInstance::subagent_model_display_name(&entry.model),
            })
        })
        .collect();
    Ok(Some(serde_json::json!({"types": types, "builtins": directory.builtins})))
}

/// Options neither backend supports.
pub(super) fn unsupported_agent_option(input: &serde_json::Value) -> Option<String> {
    if input.get("isolation").is_some_and(|value| !value.is_null() && value != "") {
        return Some("Agent isolation is not supported; Locus runs subagent tools in this workspace.".into());
    }
    if input.get("cwd").is_some_and(|value| !value.is_null() && value != "") {
        return Some("Agent cwd is not supported; subagent tools run in this conversation's workspace.".into());
    }
    None
}

/// The native Agent tool's structured completed result for a Locus subagent.
/// Claude rejects plain MCP content for Agent, so Locus supplies this shape.
/// `call_id` names the agent when the output carries no task id.
pub(super) fn locus_agent_result(
    entry: &DelegationEntry,
    input: &serde_json::Value,
    output: &str,
    duration_ms: u64,
    tool_uses: usize,
    call_id: &str,
) -> serde_json::Value {
    let (task_id, text) = match output.strip_prefix("Task id: ") {
        Some(rest) => match rest.split_once('\n') {
            Some((id, text)) => (id.trim().to_string(), text.to_string()),
            None => (rest.trim().to_string(), String::new()),
        },
        None => (String::new(), output.to_string()),
    };
    let agent_id = if task_id.is_empty() { call_id } else { task_id.as_str() };
    serde_json::json!({
        "status": "completed",
        "prompt": input["prompt"],
        "agentId": format!("locus:{agent_id}"),
        "agentType": input["subagent_type"],
        "content": [{"type": "text", "text": text}],
        "resolvedModel": entry.model,
        "totalDurationMs": duration_ms,
        "totalTokens": 0,
        "totalToolUseCount": tool_uses,
        "usage": empty_agent_usage(),
        // SendMessage continues it through its Locus mailbox.
        "canContinueAgent": !task_id.is_empty(),
    })
}

fn empty_agent_usage() -> serde_json::Value {
    serde_json::json!({
        "input_tokens": 0, "output_tokens": 0, "cache_creation_input_tokens": 0,
        "cache_read_input_tokens": 0, "server_tool_use": null, "service_tier": null, "cache_creation": null,
    })
}

/// Launch result of a background Locus subagent. It is completed-shaped on
/// purpose: an `async_launched` result would make Claude track a task it
/// cannot see, and report it as lost when the conversation resumes. Locus
/// delivers the subagent's result like any other notifying task.
pub(super) fn locus_background_agent_result(
    entry: &DelegationEntry,
    input: &serde_json::Value,
    task_id: &str,
) -> serde_json::Value {
    let id = format!("locus:{task_id}");
    serde_json::json!({
        "status": "completed",
        "prompt": input["prompt"],
        "agentId": id,
        "agentType": input["subagent_type"],
        "content": [{"type": "text", "text": format!(
            "Started in the background as Locus agent {id}. Its result is delivered to you automatically when it finishes, also after this turn ends; do not poll it or predict its result. Use SendMessage with to: \"{id}\" to give it more instructions, or TaskStop with task_id \"{id}\" to stop it."
        )}],
        "resolvedModel": entry.model,
        "totalDurationMs": 0,
        "totalTokens": 0,
        "totalToolUseCount": 0,
        "usage": empty_agent_usage(),
        "canContinueAgent": true,
    })
}

/// Model-facing delegation guidance for the native route.
pub(super) const CLAUDE_UNIFIED_GUIDANCE: &str = "Delegate through the Agent tool. Each subagent_type is bound to one model: Claude Code types run as Claude subagents, the other types run as Locus subagents on their own provider. Every subagent's tools run through Locus in this workspace, under your permissions. Agents run in the foreground unless you set run_in_background: true; a background agent's result is delivered to you automatically when it finishes. Use SendMessage with an agent's id to give a running agent more instructions or to continue a finished one, and TaskStop to stop one. Subagents cannot delegate further.";
