//! Session-pinned CLI schemas. Discovery only appends deferred definitions;
//! it never promotes a loaded tool or replaces an earlier cache-prefix schema.
use super::ClaudeCodeToolDefinition;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub(crate) struct ToolCatalog {
    version: u32,
    pub native_tool_search: bool,
    initialized: bool,
    tools: Vec<ClaudeCodeToolDefinition>,
    /// Multi-agent route fixed by the conversation's first turn. Catalogs
    /// written before native agents existed have none: they stay on Locus.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    multi_agent_route: Option<String>,
}

/// Route values stored in the catalog.
pub(crate) const ROUTE_CLAUDE_UNIFIED: &str = "claudeUnified";
pub(crate) const ROUTE_LOCUS: &str = "locus";

impl ToolCatalog {
    pub fn load(path: &Path, native_tool_search: bool) -> Result<Self, String> {
        match std::fs::read(path) {
            Ok(bytes) => {
                let catalog: Self = serde_json::from_slice(&bytes)
                    .map_err(|error| format!("Cannot restore Claude tool catalog: {error}"))?;
                if catalog.version != 1 || catalog.native_tool_search != native_tool_search {
                    return Err("Claude tool loading is pinned for this conversation to preserve its prompt cache. Restore the original loading mode or start a new conversation.".into());
                }
                Ok(catalog)
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Self {
                version: 1, native_tool_search, initialized: false, tools: Vec::new(), multi_agent_route: None,
            }),
            Err(error) => Err(format!("Cannot read Claude tool catalog: {error}")),
        }
    }

    /// Validate the complete delta before changing anything. Absence is not
    /// removal: old tool_reference blocks must keep their original definitions.
    /// Execution authorization remains the Agent host's responsibility.
    pub fn merge(&mut self, incoming: &[ClaudeCodeToolDefinition]) -> Result<Vec<ClaudeCodeToolDefinition>, String> {
        let mut next = self.tools.clone();
        let mut added = Vec::new();
        for tool in incoming {
            if tool.name == "read" || tool.name == "web_fetch" { continue; }
            if let Some(prior) = next.iter().find(|prior| prior.name == tool.name) {
                if prior.description != tool.description || prior.input_schema != tool.input_schema {
                    return Err(format!("Claude tool '{}' changed its definition. No model request was sent with a changed tool prefix. Start a new conversation to use the new schema.", tool.name));
                }
                continue;
            }
            // Saved duplicates are intentionally retained above: removing an
            // already-published eager schema would invalidate the prefix.
            // Never publish a new duplicate, including from a later Skill or
            // a low-level caller that bypasses Agent request assembly.
            if self.native_tool_search && super::native_tools::native_equivalent_for_locus_tool(&tool.name).is_some() {
                continue;
            }
            let mut tool = tool.clone();
            // This also covers a Skill that introduces a normally-direct tool.
            // Only the very first catalog may choose the eager prefix.
            tool.defer_loading = self.native_tool_search && (self.initialized || tool.defer_loading);
            next.push(tool.clone());
            added.push(tool);
        }
        self.tools = next;
        self.initialized = true;
        Ok(added)
    }

    /// Retain saved schemas for historical references, but never register the
    /// retired Locus fetch tool again (including in compatibility sessions).
    /// This is a runtime capability change, not a rewrite of stored history.
    pub fn active_tools(&self) -> Vec<ClaudeCodeToolDefinition> {
        self.tools.iter().filter(|tool| tool.name != "web_fetch").cloned().collect()
    }

    /// A process started before the route was pinned (an SSH spare) must not
    /// drop the pin when it saves its catalog. Called only before a save.
    pub fn keep_pinned_route(&mut self, path: &Path) {
        if self.multi_agent_route.is_some() { return; }
        if let Some(saved) = std::fs::read(path).ok().and_then(|bytes| serde_json::from_slice::<ToolCatalog>(&bytes).ok()) {
            self.multi_agent_route = saved.multi_agent_route;
        }
    }

    pub fn save(&self, path: &Path) -> Result<(), String> {
        let bytes = serde_json::to_vec(self).map_err(|error| error.to_string())?;
        crate::config::atomic_write_config(path, &bytes).map_err(|error| error.to_string())
    }
}

pub(crate) fn path(session_id: &str) -> Result<PathBuf, String> {
    Ok(super::ssh::profile_root(session_id)?.join("tool-catalog.json"))
}

/// The definition this conversation already published for `name`, if any.
/// Read-only: it neither validates nor changes the loading mode.
pub(crate) fn pinned_definition(session_id: &str, name: &str) -> Option<ClaudeCodeToolDefinition> {
    pinned_definition_at(&path(session_id).ok()?, name)
}

fn pinned_definition_at(path: &Path, name: &str) -> Option<ClaudeCodeToolDefinition> {
    let catalog: ToolCatalog = serde_json::from_slice(&std::fs::read(path).ok()?).ok()?;
    catalog.tools.into_iter().find(|tool| tool.name == name)
}

pub(crate) fn native_mode_for_session(session_id: &str, resuming: bool) -> Result<bool, String> {
    native_mode_at(&path(session_id)?, resuming)
}

fn native_mode_at(path: &Path, resuming: bool) -> Result<bool, String> {
    match std::fs::read(path) {
        Ok(bytes) => {
            let catalog: ToolCatalog = serde_json::from_slice(&bytes).map_err(|error| error.to_string())?;
            if catalog.version != 1 { return Err("Unsupported Claude tool catalog version".into()); }
            Ok(catalog.native_tool_search)
        }
        // Never silently migrate an old conversation's eager prefix. Native
        // loading starts with a new conversation, not its first later search.
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(!resuming),
        Err(error) => Err(error.to_string()),
    }
}

/// Whether this conversation uses the Claude-native multi-agent route. The
/// first turn of a new conversation pins `desired`; a resumed conversation
/// without a pin (created before native agents) always stays on Locus.
/// `desired` runs only when this call pins a new conversation.
pub(crate) fn pin_multi_agent_route(session_id: &str, resuming: bool, native_tool_search: bool, desired: impl FnOnce() -> bool) -> Result<bool, String> {
    pin_multi_agent_route_with(&path(session_id)?, resuming, native_tool_search, desired)
}

#[cfg(test)]
fn pin_multi_agent_route_at(path: &Path, resuming: bool, native_tool_search: bool, desired: bool) -> Result<bool, String> {
    pin_multi_agent_route_with(path, resuming, native_tool_search, || desired)
}

fn pin_multi_agent_route_with(path: &Path, resuming: bool, native_tool_search: bool, desired: impl FnOnce() -> bool) -> Result<bool, String> {
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _lock = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let route = |catalog: &ToolCatalog| catalog.multi_agent_route.as_deref() == Some(ROUTE_CLAUDE_UNIFIED);
    let pin = |unified: bool| Some(if unified { ROUTE_CLAUDE_UNIFIED } else { ROUTE_LOCUS }.to_string());
    match std::fs::read(path) {
        Ok(bytes) => {
            let catalog: ToolCatalog = serde_json::from_slice(&bytes).map_err(|error| format!("Cannot restore Claude tool catalog: {error}"))?;
            if catalog.version != 1 { return Err("Unsupported Claude tool catalog version".into()); }
            // A catalog that never published a tool and has no pin belongs to
            // a conversation that has not completed its first turn yet.
            if catalog.multi_agent_route.is_none() && !catalog.initialized && !resuming {
                let mut pinned = catalog;
                pinned.multi_agent_route = pin(desired());
                pinned.save(path)?;
                return Ok(route(&pinned));
            }
            Ok(route(&catalog))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            if resuming { return Ok(false); }
            let unified = desired();
            let catalog = ToolCatalog {
                version: 1, native_tool_search, initialized: false, tools: Vec::new(), multi_agent_route: pin(unified),
            };
            std::fs::create_dir_all(path.parent().ok_or("Missing Claude profile directory")?).map_err(|error| error.to_string())?;
            catalog.save(path)?;
            Ok(unified)
        }
        Err(error) => Err(format!("Cannot read Claude tool catalog: {error}")),
    }
}

pub(crate) fn registered_names(session_id: &str) -> Result<Vec<String>, String> {
    let path = path(session_id)?;
    match std::fs::read(path) {
        Ok(bytes) => {
            let catalog: ToolCatalog = serde_json::from_slice(&bytes).map_err(|error| error.to_string())?;
            if catalog.version != 1 { return Err("Unsupported Claude tool catalog version".into()); }
            Ok(catalog.active_tools().into_iter().map(|tool| tool.name).collect())
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(error) => Err(error.to_string()),
    }
}

pub(super) fn copy_for_fork(source: &str, target: &str) -> Result<(), String> {
    let source = path(source)?;
    if source.is_file() {
        let bytes = std::fs::read(source).map_err(|error| error.to_string())?;
        crate::config::atomic_write_config(&path(target)?, &bytes).map_err(|error| error.to_string())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn tool(name: &str, deferred: bool) -> ClaudeCodeToolDefinition {
        ClaudeCodeToolDefinition { name: name.into(), description: name.into(), input_schema: serde_json::json!({"type":"object"}), defer_loading: deferred }
    }
    #[test]
    fn published_definitions_are_read_in_either_loading_mode() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("catalog.json");
        assert!(pinned_definition_at(&path, "subagent").is_none());
        for native in [false, true] {
            let mut catalog = ToolCatalog::load(&path, native).unwrap_or_else(|_| {
                std::fs::remove_file(&path).unwrap();
                ToolCatalog::load(&path, native).unwrap()
            });
            catalog.merge(&[tool("subagent", false), tool("python", false)]).unwrap();
            catalog.save(&path).unwrap();
            assert_eq!(pinned_definition_at(&path, "subagent").unwrap().description, "subagent");
            assert!(pinned_definition_at(&path, "missing").is_none());
        }
    }
    #[test]
    fn web_fetch_is_unavailable_in_new_and_restored_catalogs_without_rewriting_history() {
        for native in [false, true] {
            let root = tempfile::tempdir().unwrap();
            let path = root.path().join("catalog.json");
            let mut fresh = ToolCatalog::load(&path, native).unwrap();
            assert!(fresh.merge(&[tool("web_fetch", false)]).unwrap().is_empty());
            assert!(fresh.active_tools().is_empty());
            // Older catalogs may contain an eager or deferred fetch schema.
            fresh.tools = vec![tool("web_fetch", native), tool("python", false)];
            fresh.save(&path).unwrap();
            let history = std::fs::read(&path).unwrap();
            let mut restored = ToolCatalog::load(&path, native).unwrap();
            assert_eq!(restored.active_tools().iter().map(|tool| tool.name.as_str()).collect::<Vec<_>>(), vec!["python"]);
            let mut changed_fetch = tool("web_fetch", !native);
            changed_fetch.description = "new description must not re-register the tool".into();
            assert!(restored.merge(&[changed_fetch]).unwrap().is_empty());
            restored.save(&path).unwrap();
            assert_eq!(std::fs::read(&path).unwrap(), history);
        }
    }
    #[test]
    fn native_catalog_keeps_eager_prefix_and_appends_only_deferred_tools() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("catalog.json");
        let mut catalog = ToolCatalog::load(&path, true).unwrap();
        catalog.merge(&[tool("core", false), tool("first_skill", true)]).unwrap();
        let eager = catalog.tools[0].clone();
        let delta = catalog.merge(&[tool("core", true), tool("new_skill", false)]).unwrap();
        assert_eq!(catalog.tools[0], eager);
        assert!(delta[0].defer_loading);
        assert_eq!(catalog.tools.len(), 3);
        catalog.save(&path).unwrap();
        let mut resumed = ToolCatalog::load(&path, true).unwrap();
        assert!(resumed.merge(&[tool("after_resume", false)]).unwrap()[0].defer_loading);
        assert_eq!(resumed.tools[0], eager);
        assert!(ToolCatalog::load(&path, false).is_err());
    }
    #[test]
    fn new_native_catalog_omits_covered_tools_but_keeps_sdk_and_skill_tools() {
        let root = tempfile::tempdir().unwrap();
        let mut catalog = ToolCatalog::load(&root.path().join("catalog.json"), true).unwrap();
        let covered = ["read", "write", "edit", "bash", "grep", "list", "ask_user_question", "todowrite"];
        let mut incoming: Vec<_> = covered.iter().map(|name| tool(name, false)).collect();
        incoming.extend([tool("python", false), tool("unity_execute", false), tool("skill_list", true)]);
        let delta = catalog.merge(&incoming).unwrap();
        assert_eq!(delta.iter().map(|tool|tool.name.as_str()).collect::<Vec<_>>(), vec!["python", "unity_execute", "skill_list"]);
        assert!(catalog.merge(&covered.iter().map(|name|tool(name, true)).collect::<Vec<_>>()).unwrap().is_empty());
    }
    #[test]
    fn restored_eager_duplicates_remain_byte_stable_while_new_duplicates_are_omitted() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("catalog.json");
        // A catalog published before native alias deduplication.
        let mut prior = ToolCatalog::load(&path, true).unwrap();
        prior.tools = vec![tool("edit", false), tool("python", false)];
        prior.initialized = true;
        prior.save(&path).unwrap();
        let prefix = serde_json::to_vec(&prior.tools).unwrap();
        let mut restored = ToolCatalog::load(&path, true).unwrap();
        let delta = restored.merge(&[tool("python", false), tool("write", false), tool("skill_new", false)]).unwrap();
        assert_eq!(delta.len(), 1);
        assert_eq!(delta[0].name, "skill_new");
        assert!(delta[0].defer_loading);
        assert_eq!(serde_json::to_vec(&restored.tools[..2]).unwrap(), prefix);
    }
    #[test]
    fn multi_agent_route_is_pinned_by_the_first_turn_and_legacy_catalogs_stay_on_locus() {
        let root = tempfile::tempdir().unwrap();
        // A new conversation pins the setting once; later changes do not apply.
        let path = root.path().join("new.json");
        assert!(pin_multi_agent_route_at(&path, false, true, true).unwrap());
        assert!(pin_multi_agent_route_at(&path, false, true, false).unwrap());
        assert!(pin_multi_agent_route_at(&path, true, true, false).unwrap());
        // The TUI's own catalog saves keep the pin.
        let mut catalog = ToolCatalog::load(&path, true).unwrap();
        catalog.merge(&[tool("python", false)]).unwrap();
        catalog.save(&path).unwrap();
        assert!(pin_multi_agent_route_at(&path, true, true, false).unwrap());
        let off = root.path().join("off.json");
        assert!(!pin_multi_agent_route_at(&off, false, false, false).unwrap());
        assert!(!pin_multi_agent_route_at(&off, false, false, true).unwrap());
        assert!(ToolCatalog::load(&off, false).is_ok());
        // Catalogs written before native agents have no pin and stay on Locus.
        let legacy = root.path().join("legacy.json");
        let mut old = ToolCatalog::load(&legacy, true).unwrap();
        old.merge(&[tool("python", false)]).unwrap();
        old.save(&legacy).unwrap();
        assert!(!std::fs::read_to_string(&legacy).unwrap().contains("multi_agent_route"));
        assert!(!pin_multi_agent_route_at(&legacy, true, true, true).unwrap());
        assert!(!pin_multi_agent_route_at(&legacy, false, true, true).unwrap());
        let missing = root.path().join("missing.json");
        assert!(!pin_multi_agent_route_at(&missing, true, true, true).unwrap());
        assert!(!missing.exists());
        // A process started before the pin (an SSH spare) does not drop it.
        let spare_path = root.path().join("spare.json");
        let mut spare = ToolCatalog::load(&spare_path, true).unwrap();
        assert!(pin_multi_agent_route_at(&spare_path, false, true, true).unwrap());
        spare.merge(&[tool("python", false)]).unwrap();
        spare.keep_pinned_route(&spare_path);
        spare.save(&spare_path).unwrap();
        assert!(pin_multi_agent_route_at(&spare_path, true, true, false).unwrap());
    }
    #[test]
    fn existing_conversations_are_not_migrated_on_first_search() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("catalog.json");
        assert!(native_mode_at(&path, false).unwrap());
        assert!(!native_mode_at(&path, true).unwrap());
        let mut catalog = ToolCatalog::load(&path, true).unwrap();
        catalog.merge(&[]).unwrap();
        catalog.save(&path).unwrap();
        assert!(native_mode_at(&path, true).unwrap());
    }
    #[test]
    fn empty_initial_catalog_is_sealed_and_schema_changes_are_atomic() {
        let root = tempfile::tempdir().unwrap();
        let mut catalog = ToolCatalog::load(&root.path().join("catalog.json"), true).unwrap();
        catalog.merge(&[]).unwrap();
        assert!(catalog.merge(&[tool("late", false)]).unwrap()[0].defer_loading);
        let before = catalog.tools.clone();
        let mut changed = tool("late", true);
        changed.description = "changed".into();
        assert!(catalog.merge(&[tool("not_committed", true), changed]).is_err());
        assert_eq!(catalog.tools, before);
    }
}
