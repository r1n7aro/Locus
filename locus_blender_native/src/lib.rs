//! `locus_blender_native` — the in-process Blender broker DLL.
//!
//! The Blender host add-on (`locus_blender/locus_host`) loads this DLL with
//! ctypes and calls `locus_init_host`. The broker (pipe server, queue,
//! heartbeat, execution registers, state plane, main-thread stack capture)
//! comes from the shared `locus-native-broker` crate and runs on its own
//! thread, so Locus gets answers to `ping`/`status` and stack captures even
//! while Blender's main thread holds the GIL in a long C call. All bpy work
//! stays in Python on Blender's main thread: the 50 ms timer (interactive) or
//! the `locus_wait_request` loop (`-b`) takes requests from the queue.
//!
//! First release: only the common entry points plus the host entry points.
//! Blender-specific native features go here later. The exported symbol list
//! is pinned by `exports.txt` (checked by `scripts/build-locus-blender-native.mjs`).

use locus_native_broker::host::{ExecutionCommands, HostConfig};
use locus_native_broker::protocol::{errors, host_kind, names};

/// Blender host request types that run code through the Python executor.
const BLENDER_EXECUTION: ExecutionCommands = ExecutionCommands {
    execute: &["exec"],
    wait: "exec_wait",
    cancel: "cancel",
    progress: "exec_progress",
};

fn blender_config(
    instance_key: String,
    pipe: String,
    state_plane: String,
    protocol_version: i32,
) -> Option<HostConfig> {
    let instance_key = names::sanitize_key(&instance_key);
    if instance_key.is_empty() {
        return None;
    }
    let state_plane_name = if state_plane.trim().is_empty() {
        names::blender_state_plane_name(&instance_key)
    } else {
        state_plane.trim().to_string()
    };
    Some(HostConfig {
        host_kind: host_kind::BLENDER,
        pipe_name: names::normalize_pipe_name(&pipe),
        state_plane_name,
        instance_key,
        project: String::new(),
        protocol_version,
        quitting_error: errors::BLENDER_PROCESS_EXITING,
        default_status: "idle",
        extra_capabilities: &[],
        execution: BLENDER_EXECUTION,
        request_hook: None,
        status_extras: None,
        on_shutdown: None,
        managed_namer: None,
    })
}

/// `locus_init(project, pipe, protocol)`: the instance key is derived from
/// the pipe name. The add-on uses `locus_init_host` instead.
fn blender_config_from_init(_project: String, pipe: String, protocol_version: i32) -> Option<HostConfig> {
    let full = names::normalize_pipe_name(&pipe);
    let bare = full.trim_start_matches(r"\\.\pipe\");
    let key = bare.strip_prefix("locus_blender_").unwrap_or(bare).to_string();
    blender_config(key, pipe, String::new(), protocol_version)
}

locus_native_broker::export_common_ffi!(blender_config_from_init);
locus_native_broker::export_host_ffi!(blender_config);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_uses_blender_names_and_codes() {
        let config = blender_config("1234_ab".into(), "locus_blender_1234_ab".into(), String::new(), 1).unwrap();
        assert_eq!(config.host_kind, "blender");
        assert_eq!(config.pipe_name, r"\\.\pipe\locus_blender_1234_ab");
        assert_eq!(config.state_plane_name, r"Local\LocusBlenderState_1234_ab");
        assert_eq!(config.quitting_error, "blender_process_exiting");
        assert!(config.is_execute("exec"));
        assert!(!config.is_execute("execute_code"));
        assert!(config.capabilities().contains(&"broker_wait_request_v1"));
        assert!(!config.capabilities().contains(&"broker_compile_result_v1"));
        assert!(blender_config("../".into(), "p".into(), String::new(), 1).is_none());

        let from_init = blender_config_from_init(String::new(), r"\\.\pipe\locus_blender_k9".into(), 1).unwrap();
        assert_eq!(from_init.instance_key, "k9");
    }
}
