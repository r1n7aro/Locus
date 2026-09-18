use super::*;
use crate::tool::apply_patch::{from_arguments, Change};
use std::path::PathBuf;

impl AgentInstance {
    pub(super) fn uses_codex_apply_patch(&self) -> bool {
        self.codex_use_apply_patch.load(Ordering::Relaxed)
            && (matches!(self.backend, LlmBackend::OpenAiCodex { .. })
                || self.preview_codex_subscription)
            && self
                .effective_model
                .trim()
                .rsplit('/')
                .next()
                .unwrap_or_default()
                .to_ascii_lowercase()
                .starts_with("gpt-")
    }

    pub(super) fn apply_codex_edit_tool_choice(&self, names: &mut Vec<String>) {
        let enabled = self.uses_codex_apply_patch();
        names.retain(|name| !name.eq_ignore_ascii_case("apply_patch"));
        if enabled {
            for name in names.iter_mut() {
                if name.eq_ignore_ascii_case("edit") {
                    *name = "apply_patch".into();
                }
            }
        }
    }

    pub(super) fn patch_policy_targets(
        &self,
        name: &str,
        args: &serde_json::Value,
    ) -> Result<Vec<(String, serde_json::Value)>, String> {
        if name != "apply_patch" {
            return Ok(if matches!(name, "write" | "edit") {
                vec![(name.to_owned(), args.clone())]
            } else {
                Vec::new()
            });
        }
        let mut result = Vec::new();
        for file in from_arguments(args)? {
            for (path, tool) in file.targets() {
                let path = PathBuf::from(path);
                let path = if path.is_absolute() || !self.has_selected_working_dir() {
                    path
                } else {
                    PathBuf::from(&self.working_dir).join(path)
                };
                result.push((
                    tool.to_owned(),
                    serde_json::json!({"filePath":path.to_string_lossy()}),
                ));
            }
        }
        Ok(result)
    }

    pub(super) fn patch_plan_violation(
        &self,
        runtime: &PlanRuntime,
        args: &serde_json::Value,
    ) -> Option<String> {
        let files = match from_arguments(args) {
            Ok(files) => files,
            Err(error) => return Some(error),
        };
        for file in files {
            if matches!(
                file.change,
                Change::Delete
                    | Change::Update {
                        move_to: Some(_),
                        ..
                    }
            ) {
                return Some("apply_patch cannot delete or move files in plan mode".into());
            }
            for (path, tool) in file.targets() {
                if let Some(error) = self.plan_mode_tool_violation(
                    runtime,
                    tool,
                    &serde_json::json!({"filePath":path}),
                ) {
                    return Some(error);
                }
            }
        }
        None
    }

    pub(super) fn validate_patch_paths(
        &self,
        args: &serde_json::Value,
        enforce_boundary: bool,
    ) -> Option<String> {
        let targets = match self.patch_policy_targets("apply_patch", args) {
            Ok(targets) => targets,
            Err(error) => return Some(error),
        };
        let registry = crate::knowledge_source_registry::KnowledgeSourceRegistry::build(
            &self.working_dir,
            self.app_knowledge_dir.as_ref().as_ref(),
        );
        for (tool, args) in targets {
            let plan_grant = matches!(self.plan_runtime_snapshot(), Some(PlanRuntime::Main { plan_file }) if self.args_target_plan_file(&args, &plan_file));
            let knowledge_grant = args["filePath"]
                .as_str()
                .is_some_and(|path| registry.classify_path_string(path).is_some());
            if !plan_grant && !knowledge_grant {
                if let Some(error) = Self::validate_tool_path_requirements_with_app_agent_dir(
                    &self.working_dir,
                    self.app_agent_dir.as_ref(),
                    &tool,
                    &args,
                    enforce_boundary,
                ) {
                    return Some(error);
                }
            }
            if let Some(error) = self
                .validate_read_only_extra_workdir_access(&tool, &args)
                .or_else(|| self.validate_knowledge_tool_routing(&tool, &args))
            {
                return Some(error);
            }
        }
        None
    }

    pub(super) fn file_write_paths(&self, name: &str, args: &serde_json::Value) -> Vec<String> {
        if !matches!(name, "edit" | "write" | "apply_patch") {
            return Vec::new();
        }
        self.patch_policy_targets(name, args)
            .unwrap_or_default()
            .into_iter()
            .filter_map(|(_, args)| args["filePath"].as_str().map(str::to_owned))
            .collect()
    }
}

#[cfg(test)]
#[path = "codex_patch_tests.rs"]
mod tests;
