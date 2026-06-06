use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::Path;

pub const KNOWLEDGE_AGENT_ID: &str = "knowledge";

pub fn canonical_agent_id(agent_id: &str) -> &str {
    match agent_id {
        "doc" | "wiki" => KNOWLEDGE_AGENT_ID,
        _ => agent_id,
    }
}

pub fn is_hidden_legacy_agent_id(agent_id: &str) -> bool {
    matches!(agent_id, "doc" | "wiki")
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentDef {
    #[serde(default)]
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
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
            let Some(id) = id else {
                continue;
            };
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
            let id = match path.file_name().and_then(|n| n.to_str()) {
                Some(name) => name.to_string(),
                None => continue,
            };
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
            let id = match path.file_name().and_then(|n| n.to_str()) {
                Some(name) => name.to_string(),
                None => continue,
            };

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
        Self::normalize_agent_tools(id, &mut def.tools);

        let system_path = dir.join("system.md");
        if system_path.is_file() {
            def.system_prompt = fs::read_to_string(&system_path)
                .map_err(|e| format!("read system.md error: {}", e))?;
        }

        let env_path = dir.join("env.md");
        if env_path.is_file() {
            def.env_template =
                fs::read_to_string(&env_path).map_err(|e| format!("read env.md error: {}", e))?;
        }

        Ok(def)
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

        let system_path = project_dir.join("system.md");
        if system_path.is_file() {
            if let Ok(prompt) = fs::read_to_string(&system_path) {
                base.system_prompt = prompt;
            }
        }

        let env_path = project_dir.join("env.md");
        if env_path.is_file() {
            if let Ok(template) = fs::read_to_string(&env_path) {
                base.env_template = template;
            }
        }
    }

    fn normalize_agent_tools(agent_id: &str, tools: &mut Vec<String>) {
        for tool in tools.iter_mut() {
            let normalized = match tool.as_str() {
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

        if !matches!(canonical_agent_id(agent_id), "dev" | KNOWLEDGE_AGENT_ID) {
            return;
        }

        tools.retain(|tool| !matches!(tool.as_str(), "knowledge_directory" | "knowledge_update"));

        let required_tools: &[&str] = if canonical_agent_id(agent_id) == "dev" {
            &[
                "knowledge_create",
                "knowledge_edit",
                "knowledge_move",
                "knowledge_delete",
                "graph_view",
            ]
        } else {
            &[
                "knowledge_create",
                "knowledge_edit",
                "knowledge_move",
                "knowledge_delete",
            ]
        };

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

    pub fn list_task_agent_descriptions(&self) -> Vec<(String, String)> {
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

#[cfg(test)]
mod tests {
    use super::AgentDefRegistry;
    use std::path::PathBuf;

    fn repo_agent_dir() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../agent")
    }

    fn assert_knowledge_mutation_tools(agent_id: &str) {
        let registry = AgentDefRegistry::load(Some(repo_agent_dir().as_path()), None);
        let agent = registry
            .get(agent_id)
            .unwrap_or_else(|| panic!("agent '{}' should be loadable", agent_id));

        for tool in [
            "knowledge_create",
            "knowledge_edit",
            "knowledge_move",
            "knowledge_delete",
        ] {
            assert!(
                agent.tools.iter().any(|name| name == tool),
                "agent '{}' should expose '{}'",
                agent_id,
                tool
            );
        }

        for legacy_tool in ["knowledge_directory", "knowledge_update"] {
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

        AgentDefRegistry::normalize_agent_tools("dev", &mut tools);

        for tool in [
            "knowledge_create",
            "knowledge_edit",
            "knowledge_move",
            "knowledge_delete",
        ] {
            assert!(tools.iter().any(|name| name == tool));
        }

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

        AgentDefRegistry::normalize_agent_tools("dev", &mut tools);

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
    fn dev_agent_exposes_knowledge_mutation_tools() {
        assert_knowledge_mutation_tools("dev");
    }

    #[test]
    fn dev_agent_exposes_graph_view_tool() {
        let registry = AgentDefRegistry::load(Some(repo_agent_dir().as_path()), None);
        let agent = registry.get("dev").expect("dev agent should be loadable");

        assert!(
            agent.tools.iter().any(|name| name == "graph_view"),
            "dev agent should expose graph_view"
        );
    }

    #[test]
    fn dev_agent_exposes_view_property_tools() {
        let registry = AgentDefRegistry::load(Some(repo_agent_dir().as_path()), None);
        let agent = registry.get("dev").expect("dev agent should be loadable");

        for tool in [
            "view_property_read",
            "view_property_discover",
            "view_property_write",
            "view_property_apply",
        ] {
            assert!(
                agent.tools.iter().any(|name| name == tool),
                "dev agent should expose '{}'",
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
                "dev agent should not expose legacy tool '{}'",
                legacy_tool
            );
        }
    }

    #[test]
    fn knowledge_agent_exposes_knowledge_mutation_tools() {
        assert_knowledge_mutation_tools("knowledge");
    }

    #[test]
    fn git_agent_exposes_knowledge_inspection_tools() {
        let registry = AgentDefRegistry::load(Some(repo_agent_dir().as_path()), None);
        let agent = registry.get("git").expect("git agent should be loadable");

        for tool in ["knowledge_list", "knowledge_query", "knowledge_read"] {
            assert!(
                agent.tools.iter().any(|name| name == tool),
                "git agent should expose '{}'",
                tool
            );
        }
    }

    #[test]
    fn task_agent_descriptions_hide_legacy_aliases() {
        let registry = AgentDefRegistry::load(Some(repo_agent_dir().as_path()), None);
        let descriptions = registry.list_task_agent_descriptions();

        assert!(descriptions
            .iter()
            .all(|(id, _)| id != "doc" && id != "wiki"));
        assert!(descriptions.iter().any(|(id, _)| id == "knowledge"));
    }
}
