//! File coordination is independent of Unity's transient execution state.
//! All hosts use this policy; disabling snapshots must not leave a Unity tool
//! holding the filesystem gate while other agents edit source files.
use serde_json::Value;

use super::workspace_execution_lock::{
    normalize_workspace_path_key, WorkspaceExecutionLockRequest,
};

pub(crate) fn is_unity_execution_barrier_tool(name: &str) -> bool {
    matches!(
        name,
        "unity_execute"
            | "unity_run_states"
            | "unity_test_list"
            | "unity_test_run"
            | "unity_recompile"
            | "unity_hot_reload"
            | "unity_set_play_mode"
    )
}

pub(crate) fn needs_unity_execution_barrier(name: &str, args: &Value) -> bool {
    (is_unity_execution_barrier_tool(name)
        || matches!(
            name,
            "unity_import_assets" | "unity_edit_session_cleanup" | "unity_capture_viewport"
        ))
        && !(name == "unity_execute"
            && super::instance::AgentInstance::unity_execute_is_readonly(args))
}

pub(crate) fn workspace_request(
    name: &str,
    args: &Value,
    working_dir: &str,
    mutates_workspace: bool,
    session_undo_enabled: bool,
) -> Option<WorkspaceExecutionLockRequest> {
    use WorkspaceExecutionLockRequest::{Exclusive, PathWrite};
    if name == "execute_typescript" {
        // UI callbacks acquire their own mutation scopes; an outer gate would
        // deadlock their re-entry through IPC.
        return None;
    }
    if matches!(name, "write" | "edit") {
        return Some(
            args.get("filePath")
                .and_then(Value::as_str)
                .map(|path| PathWrite(vec![normalize_workspace_path_key(working_dir, path)]))
                .unwrap_or(Exclusive),
        );
    }
    if name == "apply_patch" {
        return Some(
            crate::tool::apply_patch::from_arguments(args)
                .ok()
                .map(|patches| {
                    let mut paths = patches
                        .iter()
                        .flat_map(|patch| patch.targets())
                        .map(|(path, _)| normalize_workspace_path_key(working_dir, path))
                        .collect::<Vec<_>>();
                    paths.sort();
                    paths.dedup();
                    PathWrite(paths)
                })
                .unwrap_or(Exclusive),
        );
    }
    if !session_undo_enabled {
        return None;
    }
    let needs_snapshot_isolation = match name {
        "bash" => super::instance::AgentInstance::bash_needs_primary_workspace_tracking_for(
            working_dir,
            args,
        ),
        "python" => !crate::tool::builtins::python_is_readonly(args),
        "unity_execute" => !super::instance::AgentInstance::unity_execute_is_readonly(args),
        _ => mutates_workspace || is_unity_execution_barrier_tool(name),
    };
    needs_snapshot_isolation.then_some(Exclusive)
}

pub(crate) fn background_workspace_request(
    name: &str,
    args: &Value,
    working_dir: &str,
    mutates_workspace: bool,
    session_undo_enabled: bool,
    parallel_group_id: &str,
) -> Option<WorkspaceExecutionLockRequest> {
    if name == "subagent" {
        return None;
    }
    let request = workspace_request(
        name,
        args,
        working_dir,
        mutates_workspace,
        session_undo_enabled,
    );
    if matches!(name, "bash" | "python") && request.is_some() {
        Some(WorkspaceExecutionLockRequest::ParallelOpaque(
            parallel_group_id.to_string(),
        ))
    } else {
        request
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn patch_locks_both_move_paths_and_matches_sdk_with_or_without_undo() {
        let root = tempfile::tempdir().unwrap();
        let project = root.path().to_string_lossy();
        let args = json!({"patch":"*** Begin Patch\n*** Update File: Old.cs\n*** Move to: New.cs\n@@\n-old\n+new\n*** Delete File: Removed.cs\n*** End Patch"});
        for undo in [false, true] {
            let expected = workspace_request("apply_patch", &args, &project, true, undo).unwrap();
            let WorkspaceExecutionLockRequest::PathWrite(paths) = &expected else {
                panic!("targeted patch");
            };
            assert_eq!(paths.len(), 3);
            for path in ["Old.cs", "New.cs", "Removed.cs"] {
                assert!(paths.contains(&normalize_workspace_path_key(&project, path)));
            }
            assert_eq!(
                Some(expected),
                crate::sdk::direct_tool_lock_request("apply_patch", &args, &project, true, undo)
            );
        }
    }
}
