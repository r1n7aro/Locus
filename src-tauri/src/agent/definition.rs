use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

pub const DEFAULT_AGENT_ID: &str = "unity";
pub const USER_AGENTS_DIR_NAME: &str = "user-agents";
pub const GENERIC_PROJECT_TYPE: &str = "generic";
pub const UNITY_PROJECT_TYPE: &str = "unity";

pub fn canonical_agent_id(agent_id: &str) -> &str {
    match agent_id {
        "doc" | "wiki" | "git" | "knowledge" | "runtime_debugger" => {
            DEFAULT_AGENT_ID
        }
        _ => agent_id,
    }
}

pub fn is_removed_agent_id(agent_id: &str) -> bool {
    agent_id == "dev"
}

pub fn is_hidden_legacy_agent_id(agent_id: &str) -> bool {
    matches!(
        agent_id,
        "dev" | "doc" | "wiki" | "git" | "knowledge" | "runtime_debugger"
    )
}

pub fn user_agent_dir(app_agent_dir: &Path) -> PathBuf {
    app_agent_dir
        .parent()
        .unwrap_or(app_agent_dir)
        .join(USER_AGENTS_DIR_NAME)
}

pub fn app_agent_layer_dirs(app_agent_dir: &Path, agent_id: &str) -> Vec<PathBuf> {
    if is_removed_agent_id(agent_id) {
        return Vec::new();
    }
    let agent_id = canonical_agent_id(agent_id);
    let user_dir = user_agent_dir(app_agent_dir);
    vec![app_agent_dir.join(agent_id), user_dir.join(agent_id)]
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentToolDescriptionOverride {
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub parameters: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentDef {
    #[serde(default)]
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub project_types: Vec<String>,
    #[serde(skip)]
    pub system_prompt: String,
    #[serde(skip)]
    pub env_template: String,
    #[serde(default)]
    pub tools: Vec<String>,
    #[serde(default)]
    pub sub_agents: Vec<String>,
    #[serde(default)]
    pub default: bool,
    #[serde(default)]
    pub default_effort: Option<String>,
    #[serde(default)]
    pub model_recommendation: Option<String>,
    #[serde(skip)]
    pub tool_description_overrides: HashMap<String, AgentToolDescriptionOverride>,
    #[serde(skip)]
    pub source: String,
}

#[derive(Clone)]
pub struct AgentDefRegistry {
    defs: HashMap<String, AgentDef>,
    default_id: String,
}

impl AgentDefRegistry {
    ///
    pub fn load(app_agent_dir: Option<&Path>, project_agent_dir: Option<&Path>) -> Self {
        Self::load_with_plugins(app_agent_dir, project_agent_dir, &[])
    }

    pub fn load_with_plugins(
        app_agent_dir: Option<&Path>,
        project_agent_dir: Option<&Path>,
        plugin_agent_sources: &[crate::plugin::PluginComponentSource],
    ) -> Self {
        let mut defs = HashMap::new();
        let mut default_id: Option<String> = None;

        if let Some(app_dir) = app_agent_dir {
            Self::scan_agent_dir(app_dir, &mut defs, &mut default_id);
        }

        Self::scan_plugin_agent_sources(plugin_agent_sources, &mut defs, &mut default_id);

        if let Some(app_dir) = app_agent_dir {
            Self::scan_user_agent_dir(&user_agent_dir(app_dir), &mut defs, &mut default_id);
        }

        if let Some(project_dir) = project_agent_dir {
            Self::scan_agent_dir_with_merge(project_dir, &mut defs, &mut default_id);
        }

        if defs.is_empty() {
            println!("[Locus] no agent defs found");
            return AgentDefRegistry {
                defs,
                default_id: String::new(),
            };
        }

        let default_id = default_id.unwrap_or_else(|| {
            let id = defs.keys().next().expect("at least one AgentDef").clone();
            println!("[Locus] no default agent marked, using '{}'", id);
            id
        });

        println!("[Locus] default agent: '{}'", default_id);

        AgentDefRegistry { defs, default_id }
    }

    fn scan_plugin_agent_sources(
        sources: &[crate::plugin::PluginComponentSource],
        defs: &mut HashMap<String, AgentDef>,
        default_id: &mut Option<String>,
    ) {
        for source in sources {
            let dir = if source.root.is_file() {
                source
                    .root
                    .parent()
                    .map(Path::to_path_buf)
                    .unwrap_or_else(|| source.root.clone())
            } else {
                source.root.clone()
            };
            let id = source
                .id
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
                .or_else(|| {
                    dir.file_name()
                        .and_then(|value| value.to_str())
                        .map(str::to_string)
                });
            let Some(raw_id) = id else {
                continue;
            };
            if is_removed_agent_id(&raw_id) {
                continue;
            }
            let id = raw_id;
            match Self::load_agent_from_dir(&dir, &id) {
                Ok(mut def) => {
                    def.source =
                        format!("{}:{}", source.scope.component_source(), source.plugin_id);
                    println!("[Locus] loaded plugin agent def '{}' from {:?}", id, dir);
                    if def.default {
                        *default_id = Some(id.clone());
                    }
                    defs.insert(id, def);
                }
                Err(error) => eprintln!(
                    "[Locus] failed to load plugin agent '{}' from {:?}: {}",
                    id, dir, error
                ),
            }
        }
    }

    fn scan_agent_dir(
        dir: &Path,
        defs: &mut HashMap<String, AgentDef>,
        default_id: &mut Option<String>,
    ) {
        if !dir.is_dir() {
            return;
        }
        let entries = match fs::read_dir(dir) {
            Ok(e) => e,
            Err(e) => {
                eprintln!("[Locus] failed to read agent dir {:?}: {}", dir, e);
                return;
            }
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let raw_id = match path.file_name().and_then(|n| n.to_str()) {
                Some(name) => name.to_string(),
                None => continue,
            };
            if is_removed_agent_id(&raw_id) {
                continue;
            }
            let id = raw_id;
            match Self::load_agent_from_dir(&path, &id) {
                Ok(mut def) => {
                    def.source = "app".to_string();
                    println!("[Locus] loaded agent def '{}' from {:?}", id, path);
                    if def.default {
                        *default_id = Some(id.clone());
                    }
                    defs.insert(id, def);
                }
                Err(e) => {
                    eprintln!(
                        "[Locus] failed to load agent '{}' from {:?}: {}",
                        id, path, e
                    );
                }
            }
        }
    }

    fn scan_agent_dir_with_merge(
        dir: &Path,
        defs: &mut HashMap<String, AgentDef>,
        default_id: &mut Option<String>,
    ) {
        if !dir.is_dir() {
            return;
        }
        let entries = match fs::read_dir(dir) {
            Ok(e) => e,
            Err(e) => {
                eprintln!("[Locus] failed to read project agent dir {:?}: {}", dir, e);
                return;
            }
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let raw_id = match path.file_name().and_then(|n| n.to_str()) {
                Some(name) => name.to_string(),
                None => continue,
            };
            if is_removed_agent_id(&raw_id) {
                continue;
            }
            let id = raw_id;

            if let Some(existing) = defs.get_mut(&id) {
                Self::merge_project_overlay(existing, &path);
                existing.source = "both".to_string();
                println!("[Locus] merged project overlay for agent '{}'", id);
            } else {
                match Self::load_agent_from_dir(&path, &id) {
                    Ok(mut def) => {
                        def.source = "project".to_string();
                        println!("[Locus] loaded project-only agent def '{}'", id);
                        if def.default {
                            *default_id = Some(id.clone());
                        }
                        defs.insert(id, def);
                    }
                    Err(e) => {
                        eprintln!("[Locus] failed to load project agent '{}': {}", id, e);
                    }
                }
            }
        }
    }

    fn scan_user_agent_dir(
        dir: &Path,
        defs: &mut HashMap<String, AgentDef>,
        default_id: &mut Option<String>,
    ) {
        if !dir.is_dir() {
            return;
        }
        let entries = match fs::read_dir(dir) {
            Ok(entries) => entries,
            Err(error) => {
                eprintln!("[Locus] failed to read user agent dir {:?}: {}", dir, error);
                return;
            }
        };

        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let Some(raw_id) = path
                .file_name()
                .and_then(|name| name.to_str())
                .map(str::to_string)
            else {
                continue;
            };
            if is_removed_agent_id(&raw_id) {
                continue;
            }
            let id = raw_id;

            if let Some(existing) = defs.get_mut(&id) {
                Self::merge_project_overlay(existing, &path);
                existing.source = "appUser".to_string();
                if existing.default {
                    *default_id = Some(id.clone());
                }
                println!("[Locus] merged user overlay for agent '{}'", id);
                continue;
            }

            match Self::load_agent_from_dir(&path, &id) {
                Ok(mut def) => {
                    def.source = "user".to_string();
                    if def.default {
                        *default_id = Some(id.clone());
                    }
                    println!("[Locus] loaded user agent def '{}' from {:?}", id, path);
                    defs.insert(id, def);
                }
                Err(error) => eprintln!(
                    "[Locus] failed to load user agent '{}' from {:?}: {}",
                    id, path, error
                ),
            }
        }
    }

    fn load_agent_from_dir(dir: &Path, id: &str) -> Result<AgentDef, String> {
        let config_path = dir.join("config.json");
        if !config_path.is_file() {
            return Err(format!("config.json not found in {:?}", dir));
        }
        let content = fs::read_to_string(&config_path)
            .map_err(|e| format!("read config.json error: {}", e))?;
        let mut def: AgentDef = serde_json::from_str(&content)
            .map_err(|e| format!("parse config.json error: {}", e))?;

        def.id = id.to_string();
        Self::normalize_project_types(&mut def.project_types);
        Self::normalize_agent_tools(id, &mut def.tools);

        if let Some(prompt_path) = Self::system_prompt_path(dir) {
            def.system_prompt = fs::read_to_string(&prompt_path)
                .map_err(|e| format!("read {:?} error: {}", prompt_path, e))?;
        }

        let env_path = dir.join("env.md");
        if env_path.is_file() {
            def.env_template =
                fs::read_to_string(&env_path).map_err(|e| format!("read env.md error: {}", e))?;
        }

        def.tool_description_overrides = Self::load_tool_description_overrides(dir)?;

        Ok(def)
    }

    fn load_tool_description_overrides(
        agent_dir: &Path,
    ) -> Result<HashMap<String, AgentToolDescriptionOverride>, String> {
        let tools_dir = agent_dir.join("tools");
        if !tools_dir.is_dir() {
            return Ok(HashMap::new());
        }

        let entries = fs::read_dir(&tools_dir)
            .map_err(|error| format!("read tools directory error: {}", error))?;
        let mut overrides = HashMap::new();
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_file() || path.extension().and_then(|value| value.to_str()) != Some("json")
            {
                continue;
            }
            let tool_name = path
                .file_stem()
                .and_then(|value| value.to_str())
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .ok_or_else(|| format!("invalid tool override file name: {:?}", path))?
                .to_string();
            let content = fs::read_to_string(&path)
                .map_err(|error| format!("read {:?} error: {}", path, error))?;
            let definition: AgentToolDescriptionOverride = serde_json::from_str(&content)
                .map_err(|error| format!("parse {:?} error: {}", path, error))?;
            if definition.description.is_none() && definition.parameters.is_none() {
                return Err(format!(
                    "tool override {:?} must define description or parameters",
                    path
                ));
            }
            if definition
                .parameters
                .as_ref()
                .is_some_and(|parameters| !parameters.is_object())
            {
                return Err(format!(
                    "tool override {:?} parameters must be a JSON object",
                    path
                ));
            }
            overrides.insert(tool_name, definition);
        }
        Ok(overrides)
    }

    fn system_prompt_path(agent_dir: &Path) -> Option<PathBuf> {
        ["soul.md", "system.md"]
            .into_iter()
            .map(|name| agent_dir.join(name))
            .find(|path| path.is_file())
    }

    fn normalize_project_types(project_types: &mut Vec<String>) {
        for project_type in project_types.iter_mut() {
            *project_type = project_type.trim().to_ascii_lowercase();
        }
        let mut seen = HashSet::new();
        project_types
            .retain(|project_type| !project_type.is_empty() && seen.insert(project_type.clone()));
    }

    fn apply_schema_description_overlay(
        target: &mut serde_json::Value,
        overlay: &serde_json::Value,
    ) {
        let (Some(target_object), Some(overlay_object)) =
            (target.as_object_mut(), overlay.as_object())
        else {
            return;
        };

        if let Some(description) = overlay_object
            .get("description")
            .and_then(serde_json::Value::as_str)
        {
            target_object.insert(
                "description".to_string(),
                serde_json::Value::String(description.to_string()),
            );
        }

        for map_key in ["properties", "$defs", "definitions"] {
            let Some(overlay_entries) = overlay_object
                .get(map_key)
                .and_then(serde_json::Value::as_object)
            else {
                continue;
            };
            let Some(target_entries) = target_object
                .get_mut(map_key)
                .and_then(serde_json::Value::as_object_mut)
            else {
                continue;
            };
            for (name, overlay_entry) in overlay_entries {
                if let Some(target_entry) = target_entries.get_mut(name) {
                    Self::apply_schema_description_overlay(target_entry, overlay_entry);
                }
            }
        }

        if let (Some(target_items), Some(overlay_items)) =
            (target_object.get_mut("items"), overlay_object.get("items"))
        {
            Self::apply_schema_description_overlay(target_items, overlay_items);
        }

        for list_key in ["allOf", "anyOf", "oneOf"] {
            let Some(overlay_items) = overlay_object
                .get(list_key)
                .and_then(serde_json::Value::as_array)
            else {
                continue;
            };
            let Some(target_items) = target_object
                .get_mut(list_key)
                .and_then(serde_json::Value::as_array_mut)
            else {
                continue;
            };
            for (target_item, overlay_item) in target_items.iter_mut().zip(overlay_items) {
                Self::apply_schema_description_overlay(target_item, overlay_item);
            }
        }
    }

    fn merge_project_overlay(base: &mut AgentDef, project_dir: &Path) {
        let config_path = project_dir.join("config.json");
        if config_path.is_file() {
            if let Ok(content) = fs::read_to_string(&config_path) {
                if let Ok(overlay) = serde_json::from_str::<serde_json::Value>(&content) {
                    if let Some(name) = overlay.get("name").and_then(|v| v.as_str()) {
                        if !name.is_empty() {
                            base.name = name.to_string();
                        }
                    }
                    if let Some(desc) = overlay.get("description").and_then(|v| v.as_str()) {
                        if !desc.is_empty() {
                            base.description = desc.to_string();
                        }
                    }
                    if let Some(project_types) =
                        overlay.get("project_types").and_then(|v| v.as_array())
                    {
                        base.project_types = project_types
                            .iter()
                            .filter_map(|v| v.as_str().map(String::from))
                            .collect();
                        Self::normalize_project_types(&mut base.project_types);
                    }
                    if let Some(tools) = overlay.get("tools").and_then(|v| v.as_array()) {
                        base.tools = tools
                            .iter()
                            .filter_map(|v| v.as_str().map(String::from))
                            .collect();
                        Self::normalize_agent_tools(&base.id, &mut base.tools);
                    }
                    if let Some(subs) = overlay.get("sub_agents").and_then(|v| v.as_array()) {
                        base.sub_agents = subs
                            .iter()
                            .filter_map(|v| v.as_str().map(String::from))
                            .collect();
                    }
                    if let Some(d) = overlay.get("default").and_then(|v| v.as_bool()) {
                        base.default = d;
                    }
                    if let Some(default_effort) = overlay.get("default_effort") {
                        if default_effort.is_null() {
                            base.default_effort = None;
                        } else if let Some(s) = default_effort.as_str() {
                            let trimmed = s.trim();
                            if trimmed.is_empty() {
                                base.default_effort = None;
                            } else {
                                base.default_effort = Some(trimmed.to_string());
                            }
                        }
                    }
                    if let Some(model_recommendation) = overlay.get("model_recommendation") {
                        if model_recommendation.is_null() {
                            base.model_recommendation = None;
                        } else if let Some(s) = model_recommendation.as_str() {
                            let trimmed = s.trim();
                            if trimmed.is_empty() {
                                base.model_recommendation = None;
                            } else {
                                base.model_recommendation = Some(trimmed.to_string());
                            }
                        }
                    }
                }
            }
        }

        if let Some(prompt_path) = Self::system_prompt_path(project_dir) {
            if let Ok(prompt) = fs::read_to_string(&prompt_path) {
                base.system_prompt = prompt;
            }
        }

        let env_path = project_dir.join("env.md");
        if env_path.is_file() {
            if let Ok(template) = fs::read_to_string(&env_path) {
                base.env_template = template;
            }
        }

        match Self::load_tool_description_overrides(project_dir) {
            Ok(overrides) => base.tool_description_overrides.extend(overrides),
            Err(error) => eprintln!(
                "[Locus] failed to load Agent tool description overrides from {:?}: {}",
                project_dir, error
            ),
        }
    }

    /// Normalize retired aliases and keep the unified knowledge/file tool
    /// surface available for the agents that maintain project knowledge.
    fn normalize_agent_tools(agent_id: &str, tools: &mut Vec<String>) {
        for tool in tools.iter_mut() {
            let normalized = match tool.as_str() {
                "task" => "subagent",
                "view_binding_read" => "view_property_read",
                "view_binding_discover" => "view_property_discover",
                "view_binding_write" => "view_property_write",
                "view_binding_apply" => "view_property_apply",
                _ => continue,
            };
            *tool = normalized.to_string();
        }

        let mut seen = HashSet::new();
        tools.retain(|tool| seen.insert(tool.clone()));

        tools.retain(|tool| {
            !matches!(
                tool.as_str(),
                "knowledge_directory"
                    | "knowledge_update"
                    | "knowledge_list"
                    | "knowledge_read"
                    | "knowledge_create"
                    | "knowledge_edit"
                    | "knowledge_move"
                    | "knowledge_delete"
            )
        });

        if canonical_agent_id(agent_id) != DEFAULT_AGENT_ID {
            return;
        }

        let required_tools: &[&str] = &["knowledge_query"];

        for &tool in required_tools {
            if !tools.iter().any(|name| name == tool) {
                tools.push(tool.to_string());
            }
        }
    }

    pub fn get(&self, id: &str) -> Option<&AgentDef> {
        self.defs.get(canonical_agent_id(id))
    }

    pub fn default_def(&self) -> Option<&AgentDef> {
        self.defs.get(&self.default_id)
    }

    #[allow(dead_code)]
    pub fn list_ids(&self) -> Vec<&str> {
        self.defs.keys().map(|s| s.as_str()).collect()
    }

    pub fn list_subagent_descriptions(&self) -> Vec<(String, String)> {
        let mut defs: Vec<&AgentDef> = self
            .defs
            .values()
            .filter(|def| !is_hidden_legacy_agent_id(&def.id))
            .collect();
        defs.sort_by(|a, b| {
            b.default
                .cmp(&a.default)
                .then(a.name.cmp(&b.name))
                .then(a.id.cmp(&b.id))
        });
        defs.into_iter()
            .map(|def| (def.id.clone(), def.description.clone()))
            .collect()
    }

    pub fn list_all(&self) -> Vec<&AgentDef> {
        self.defs.values().collect()
    }

    pub fn default_id(&self) -> &str {
        &self.default_id
    }
}

impl AgentDef {
    pub fn project_type_match_score(&self, project_type: &str) -> u8 {
        let project_type = project_type.trim().to_ascii_lowercase();
        if self
            .project_types
            .iter()
            .any(|value| value == &project_type)
        {
            return 2;
        }
        if self.project_types.is_empty() || self.project_types.iter().any(|value| value == "*") {
            return 1;
        }
        0
    }

    pub fn supports_project_type(&self, project_type: &str) -> bool {
        self.project_type_match_score(project_type) > 0
    }

    pub fn apply_tool_description_override(
        &self,
        tool_name: &str,
        tool: &mut serde_json::Value,
    ) -> bool {
        let Some(definition) = self.tool_description_overrides.get(tool_name) else {
            return false;
        };
        let Some(function) = tool
            .get_mut("function")
            .and_then(serde_json::Value::as_object_mut)
        else {
            return false;
        };
        if let Some(description) = definition.description.as_ref() {
            function.insert(
                "description".to_string(),
                serde_json::Value::String(description.clone()),
            );
        }
        if let (Some(target_parameters), Some(overlay_parameters)) = (
            function.get_mut("parameters"),
            definition.parameters.as_ref(),
        ) {
            AgentDefRegistry::apply_schema_description_overlay(
                target_parameters,
                overlay_parameters,
            );
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::{canonical_agent_id, user_agent_dir, AgentDefRegistry, DEFAULT_AGENT_ID};
    use std::fs;
    use std::path::PathBuf;

    fn remove_description_fields(value: &mut serde_json::Value) {
        match value {
            serde_json::Value::Array(items) => {
                for item in items {
                    remove_description_fields(item);
                }
            }
            serde_json::Value::Object(object) => {
                object.remove("description");
                for item in object.values_mut() {
                    remove_description_fields(item);
                }
            }
            _ => {}
        }
    }

    fn collect_description_text(value: &serde_json::Value, out: &mut Vec<String>) {
        match value {
            serde_json::Value::Array(items) => {
                for item in items {
                    collect_description_text(item, out);
                }
            }
            serde_json::Value::Object(object) => {
                if let Some(description) = object
                    .get("description")
                    .and_then(serde_json::Value::as_str)
                {
                    out.push(description.to_string());
                }
                for item in object.values() {
                    collect_description_text(item, out);
                }
            }
            _ => {}
        }
    }

    fn repo_agent_dir() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../agent")
    }

    fn assert_unified_knowledge_tools(agent_id: &str) {
        let registry = AgentDefRegistry::load(Some(repo_agent_dir().as_path()), None);
        let agent = registry
            .get(agent_id)
            .unwrap_or_else(|| panic!("agent '{}' should be loadable", agent_id));

        for tool in ["knowledge_query", "read", "write", "edit", "bash"] {
            assert!(
                agent.tools.iter().any(|name| name == tool),
                "agent '{}' should expose '{}'",
                agent_id,
                tool
            );
        }

        for legacy_tool in [
            "knowledge_directory",
            "knowledge_update",
            "knowledge_list",
            "knowledge_read",
            "knowledge_create",
            "knowledge_edit",
            "knowledge_move",
            "knowledge_delete",
        ] {
            assert!(
                agent.tools.iter().all(|name| name != legacy_tool),
                "agent '{}' should not expose legacy tool '{}'",
                agent_id,
                legacy_tool
            );
        }
    }

    #[test]
    fn normalize_agent_tools_replaces_legacy_knowledge_aliases() {
        let mut tools = vec![
            "read".to_string(),
            "knowledge_list".to_string(),
            "knowledge_query".to_string(),
            "knowledge_read".to_string(),
            "knowledge_directory".to_string(),
            "knowledge_update".to_string(),
        ];

        AgentDefRegistry::normalize_agent_tools("unity", &mut tools);

        assert!(tools.iter().any(|name| name == "knowledge_query"));

        for legacy_tool in ["knowledge_directory", "knowledge_update"] {
            assert!(tools.iter().all(|name| name != legacy_tool));
        }
    }

    #[test]
    fn normalize_agent_tools_replaces_legacy_view_property_aliases() {
        let mut tools = vec![
            "read".to_string(),
            "view_binding_read".to_string(),
            "view_property_read".to_string(),
            "view_binding_discover".to_string(),
            "view_binding_write".to_string(),
            "view_binding_apply".to_string(),
        ];

        AgentDefRegistry::normalize_agent_tools("unity", &mut tools);

        for tool in [
            "view_property_read",
            "view_property_discover",
            "view_property_write",
            "view_property_apply",
        ] {
            assert!(tools.iter().any(|name| name == tool));
        }

        for legacy_tool in [
            "view_binding_read",
            "view_binding_discover",
            "view_binding_write",
            "view_binding_apply",
        ] {
            assert!(tools.iter().all(|name| name != legacy_tool));
        }

        assert_eq!(
            tools
                .iter()
                .filter(|name| name.as_str() == "view_property_read")
                .count(),
            1
        );
    }

    #[test]
    fn unity_agent_exposes_unified_knowledge_tools() {
        assert_unified_knowledge_tools("unity");
    }

    #[test]
    fn built_in_agents_declare_project_types_and_simple_stays_minimal() {
        let registry = AgentDefRegistry::load(Some(repo_agent_dir().as_path()), None);
        let unity = registry.get("unity").expect("Unity Agent should load");
        let simple = registry.get("simple").expect("Simple Agent should load");

        assert_eq!(unity.project_types, vec!["unity"]);
        assert_eq!(simple.project_types, vec!["generic"]);
        assert_eq!(
            simple.tools,
            ["read", "write", "edit", "bash", "python", "grep", "list"]
        );
        assert!(simple.sub_agents.is_empty());
        assert!(simple.env_template.is_empty());
        assert!(simple
            .system_prompt
            .contains("general-purpose software development agent"));
        assert!(simple.supports_project_type("generic"));
        assert!(!simple.supports_project_type("unity"));
    }

    #[test]
    fn shared_basic_tool_prompts_stay_generic_and_unity_overrides_preserve_schema() {
        let agents = AgentDefRegistry::load(Some(repo_agent_dir().as_path()), None);
        let unity = agents.get("unity").expect("Unity Agent should load");
        let simple = agents.get("simple").expect("Simple Agent should load");
        let tools = crate::tool::ToolRegistry::with_builtins();

        for name in ["bash", "python", "grep", "list", "write", "edit"] {
            let base = tools
                .resolve_api_tool(name)
                .unwrap_or_else(|| panic!("shared tool '{name}' should resolve"));
            let mut simple_tool = base.clone();
            assert!(!simple.apply_tool_description_override(name, &mut simple_tool));
            assert_eq!(simple_tool, base);

            let mut generic_descriptions = Vec::new();
            collect_description_text(&base, &mut generic_descriptions);
            let generic_text = generic_descriptions.join("\n").to_ascii_lowercase();
            for unity_term in [
                "unity project",
                "get_unity_editor_status",
                "restart_unity_editor",
                "monobehaviour",
                "scriptableobject",
                "editmode",
                "asmdef",
                "c# code analysis",
            ] {
                assert!(
                    !generic_text.contains(unity_term),
                    "shared tool '{name}' still contains Unity-specific prompt text: {unity_term}"
                );
            }

            let mut unity_tool = base.clone();
            assert!(unity.apply_tool_description_override(name, &mut unity_tool));
            let expected_description = unity.tool_description_overrides[name]
                .description
                .as_deref()
                .expect("Unity override should replace the tool description");
            assert_eq!(
                unity_tool["function"]["description"].as_str(),
                Some(expected_description)
            );

            let mut base_without_descriptions = base;
            let mut unity_without_descriptions = unity_tool;
            remove_description_fields(&mut base_without_descriptions);
            remove_description_fields(&mut unity_without_descriptions);
            assert_eq!(
                unity_without_descriptions, base_without_descriptions,
                "Unity override must preserve the executable schema for '{name}'"
            );
        }
    }

    #[test]
    fn explorer_inspection_override_preserves_shared_execution_schema() {
        let agents = AgentDefRegistry::load(Some(repo_agent_dir().as_path()), None);
        let explorer = agents.get("explorer").expect("Explorer should load");
        let unity = agents.get("unity").expect("Unity Agent should load");
        let tools = crate::tool::ToolRegistry::with_builtins();

        assert!(explorer.tools.iter().any(|tool| tool == "unity_execute"));
        assert!(!explorer.tools.iter().any(|tool| tool == "unity_get_console_log"));

        let base = tools
            .resolve_api_tool("unity_execute")
            .expect("Unity execution tool");
        let mut inspection = base.clone();
        assert!(explorer.apply_tool_description_override("unity_execute", &mut inspection));
        assert_eq!(
            inspection["function"]["description"],
            base["function"]["description"]
        );
        for name in ["code", "readonly", "request_editor_status"] {
            assert_ne!(
                inspection["function"]["parameters"]["properties"][name]["description"],
                base["function"]["parameters"]["properties"][name]["description"],
                "Explorer should specialize the {name} guidance"
            );
        }
        let mut unchanged = base.clone();
        assert!(!unity.apply_tool_description_override("unity_execute", &mut unchanged));
        assert_eq!(unchanged, base);

        let mut executable_schema = base;
        remove_description_fields(&mut executable_schema);
        remove_description_fields(&mut inspection);
        assert_eq!(inspection, executable_schema);
    }

    #[test]
    fn unity_agent_exposes_view_property_tools() {
        let registry = AgentDefRegistry::load(Some(repo_agent_dir().as_path()), None);
        let agent = registry
            .get("unity")
            .expect("Unity Agent should be loadable");

        for tool in [
            "view_property_read",
            "view_property_discover",
            "view_property_write",
            "view_property_apply",
        ] {
            assert!(
                agent.tools.iter().any(|name| name == tool),
                "Unity Agent should expose '{}'",
                tool
            );
        }

        for legacy_tool in [
            "view_binding_read",
            "view_binding_discover",
            "view_binding_write",
            "view_binding_apply",
        ] {
            assert!(
                agent.tools.iter().all(|name| name != legacy_tool),
                "Unity Agent should not expose legacy tool '{}'",
                legacy_tool
            );
        }
    }

    #[test]
    fn retired_builtin_agent_ids_resolve_to_unity() {
        let registry = AgentDefRegistry::load(Some(repo_agent_dir().as_path()), None);
        for legacy_id in ["git", "knowledge", "runtime_debugger", "doc", "wiki"] {
            assert_eq!(canonical_agent_id(legacy_id), DEFAULT_AGENT_ID);
            assert_eq!(
                registry.get(legacy_id).map(|def| def.id.as_str()),
                Some("unity")
            );
        }
    }

    #[test]
    fn removed_dev_agent_directories_are_not_registered_or_merged() {
        let root = tempfile::tempdir().expect("temp root");
        let bundled_root = root.path().join("agent");
        let project_root = root.path().join("workspace/Locus/agent");
        for agent_root in [&bundled_root, &user_agent_dir(&bundled_root), &project_root] {
            let legacy = agent_root.join("dev");
            fs::create_dir_all(&legacy).expect("legacy Agent dir");
            fs::write(
                legacy.join("config.json"),
                r#"{"name":"Legacy","tools":["write"],"default":true}"#,
            )
            .expect("legacy config");
            fs::write(legacy.join("system.md"), "Legacy prompt").expect("legacy prompt");
        }

        let registry = AgentDefRegistry::load(Some(&bundled_root), Some(&project_root));
        assert!(registry.list_ids().is_empty());
        assert!(registry.default_def().is_none());
        assert_eq!(canonical_agent_id("dev"), "dev");
        assert!(registry.get("dev").is_none());

        let unity = bundled_root.join("unity");
        fs::create_dir_all(&unity).expect("Unity Agent dir");
        fs::write(
            unity.join("config.json"),
            r#"{"name":"Unity","tools":["read"],"default":true}"#,
        )
        .expect("Unity config");
        fs::write(unity.join("system.md"), "Current Unity prompt").expect("Unity prompt");
        let registry = AgentDefRegistry::load(Some(&bundled_root), Some(&project_root));
        assert_eq!(registry.list_ids(), vec!["unity"]);
        assert_eq!(registry.default_id(), "unity");
        assert!(registry.get("dev").is_none());
        assert_eq!(
            registry
                .get("unity")
                .map(|agent| agent.system_prompt.as_str()),
            Some("Current Unity prompt")
        );
    }

    #[test]
    fn removed_dev_agent_plugin_definition_is_not_registered() {
        let root = tempfile::tempdir().expect("temp root");
        fs::write(
            root.path().join("config.json"),
            r#"{"name":"Legacy","tools":["read"],"default":true}"#,
        )
        .expect("plugin Agent config");
        let source = crate::plugin::PluginComponentSource {
            plugin_id: "test-plugin".to_string(),
            plugin_name: "Test Plugin".to_string(),
            plugin_version: "1.0.0".to_string(),
            scope: crate::plugin::PluginInstallScope::App,
            id: Some("dev".to_string()),
            root: root.path().to_path_buf(),
            rel_path: "agent".to_string(),
        };
        let registry = AgentDefRegistry::load_with_plugins(None, None, &[source]);
        assert!(registry.list_ids().is_empty());
        assert!(registry.get("dev").is_none());
        assert!(registry.get("unity").is_none());
    }

    #[test]
    fn loads_user_agents_from_install_sibling_directory() {
        let root = tempfile::tempdir().expect("temp root");
        let bundled_root = root.path().join("agent");
        let bundled_unity = bundled_root.join("unity");
        fs::create_dir_all(&bundled_unity).expect("bundled Unity dir");
        fs::write(
            bundled_unity.join("config.json"),
            r#"{"name":"Unity","description":"Default","tools":[],"default":true}"#,
        )
        .expect("bundled config");
        fs::write(bundled_unity.join("system.md"), "Bundled prompt").expect("bundled prompt");

        let custom = user_agent_dir(&bundled_root).join("build-auditor");
        fs::create_dir_all(&custom).expect("user agent dir");
        fs::write(
            custom.join("config.json"),
            r#"{"name":"Build Auditor","description":"Audit builds","tools":["read"],"default":false}"#,
        )
        .expect("user config");
        fs::write(custom.join("system.md"), "Audit every build").expect("user prompt");

        let registry = AgentDefRegistry::load(Some(&bundled_root), None);
        let user = registry
            .get("build-auditor")
            .expect("user Agent should be indexed");
        assert_eq!(user.name, "Build Auditor");
        assert_eq!(user.source, "user");
        assert_eq!(registry.default_id(), "unity");
    }

    #[test]
    fn agent_tool_override_changes_descriptions_without_changing_schema() {
        let root = tempfile::tempdir().expect("temp root");
        let bundled_root = root.path().join("agent");
        let custom = user_agent_dir(&bundled_root).join("simple");
        fs::create_dir_all(custom.join("tools")).expect("user Agent tools dir");
        fs::write(
            custom.join("config.json"),
            r#"{"name":"Simple","description":"Simple Agent","tools":["list"],"default":false}"#,
        )
        .expect("user config");
        fs::write(custom.join("system.md"), "Keep the workflow simple.").expect("system prompt");
        fs::write(
            custom.join("tools/list.json"),
            r#"{
  "description": "List ordinary project files.",
  "parameters": {
    "properties": {
      "path": { "description": "Directory to inspect.", "type": "number" },
      "missing": { "description": "Must stay absent." }
    },
    "required": []
  }
}"#,
        )
        .expect("tool override");

        let registry = AgentDefRegistry::load(Some(&bundled_root), None);
        let agent = registry.get("simple").expect("user Agent should load");
        let mut tool = serde_json::json!({
            "function": {
                "name": "list",
                "description": "Unity-specific list description.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "path": { "type": "string", "description": "Unity project directory." }
                    },
                    "required": ["path"]
                }
            }
        });

        assert!(agent.apply_tool_description_override("list", &mut tool));
        assert_eq!(
            tool["function"]["description"],
            serde_json::json!("List ordinary project files.")
        );
        assert_eq!(
            tool["function"]["parameters"]["properties"]["path"]["description"],
            serde_json::json!("Directory to inspect.")
        );
        assert_eq!(
            tool["function"]["parameters"]["properties"]["path"]["type"],
            serde_json::json!("string")
        );
        assert_eq!(
            tool["function"]["parameters"]["required"],
            serde_json::json!(["path"])
        );
        assert!(tool["function"]["parameters"]["properties"]
            .get("missing")
            .is_none());
    }

    #[test]
    fn subagent_descriptions_hide_legacy_aliases() {
        let registry = AgentDefRegistry::load(Some(repo_agent_dir().as_path()), None);
        let descriptions = registry.list_subagent_descriptions();

        assert!(descriptions.iter().all(|(id, _)| !matches!(
            id.as_str(),
            "dev" | "doc" | "wiki" | "git" | "knowledge" | "runtime_debugger"
        )));
        assert!(descriptions.iter().any(|(id, _)| id == "unity"));
    }
}
