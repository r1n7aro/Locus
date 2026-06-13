import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const cwd = process.cwd();

function read(relPath: string) {
  return readFileSync(resolve(cwd, relPath), "utf8");
}

describe("Unity state probe self-test", () => {
  it("covers complex live editor states", () => {
    const source = read("src-tauri/src/unity_bridge/state_probe/selftest.rs");

    expect(source).toContain("Phase 4/4 — play-mode / pause / resume matrix");
    expect(source).toContain("set_editor_status_step(");
    expect(source).toContain("crate::unity_bridge::set_editor_status(&self.project, desired_status)");
    expect(source).toContain("UNITY_EDITOR_STATUS_PLAYING_PAUSED");
    expect(source).toContain("P1 enter play mode");
    expect(source).toContain("P5 fused paused state");
    expect(source).toContain("P8 fused resumed state");
    expect(source).toContain("P10 fused post-play edit state");
  });

  it("reloads a stale running Unity bridge before testing", () => {
    const source = read("src-tauri/src/unity_bridge/state_probe/selftest.rs");

    expect(source).toContain("BRIDGE_CAPABILITY_TIMEOUT");
    expect(source).toContain("BRIDGE_CAPABILITY_RELOAD_TIMEOUT");
    expect(source).toContain("check_installed_plugin_files");
    expect(source).toContain("probe_bridge_capabilities_once");
    expect(source).toContain("request_bridge_runtime_reload");
    expect(source).toContain("wait_for_bridge_capabilities_after_reload");
    expect(source).toContain("check_bridge_capabilities");
    expect(source).toContain('"bridge_capabilities"');
    expect(source).toContain('UnityEditor.EditorUtility.RequestScriptReload(); return \\"requested\\";');
    expect(source).toContain("status_cached");
    expect(source).toContain("set_editor_status_async");
    expect(source).toContain("unknown message type: bridge_capabilities");
    expect(source).toContain("precondition bridge runtime capabilities");
  });

  it("covers paused EditorApplication.update pump stalls", () => {
    const source = read("src-tauri/src/unity_bridge/state_probe/selftest.rs");

    expect(source).toContain("UPDATE_PUMP_REQUEST_BUDGET");
    expect(source).toContain("UPDATE_PUMP_STATUS_BUDGET");
    expect(source).toContain("probe_paused_update_pump_resilience");
    expect(source).toContain('"get_console_text"');
    expect(source).toContain("UP1 paused main-thread queue probe");
    expect(source).toContain("UP2 paused direct status optional");
    expect(source).toContain("UP3 connection status after paused queue probe");
    expect(source).toContain("UP3b observer state while paused");
    expect(source).toContain("state.editor_mode.value == \"paused\"");
    expect(source).toContain("state.channel.control_pipe");
    expect(source).toContain("UP4 native passive sample while paused");
    expect(source).toContain("query_unity_status_with_timeout(");
  });

  it("checks connection status without blocking reload sampling", () => {
    const source = read("src-tauri/src/unity_bridge/state_probe/selftest.rs");

    expect(source).toContain("CONNECTION_STATUS_BUDGET");
    expect(source).toContain("tokio::time::timeout(");
    expect(source).toContain("EXECUTE_TIMEOUT");
    expect(source).toContain("log_diagnostic_snapshot(");
    expect(source).toContain("format_native_sample");
    expect(source).toContain("R1 connection status during pipe-dead reload");
    expect(source).toContain("tokio::spawn(async move");
    expect(source).toContain("query_unity_connection_status(&project).await");
  });

  it("uses observer cache and the native-owned state plane", () => {
    const probe = read("src-tauri/src/unity_bridge/state_probe.rs");
    const bridge = read("locus_unity/Editor/LocusBridge.cs");
    const native = read("locus_native_plugin/src/lib.rs");
    const backend = read("src-tauri/src/unity_bridge/mod.rs");

    expect(probe).toContain("struct ObserverRuntime");
    expect(probe).toContain("async fn observer_loop(");
    expect(probe).toContain("OBSERVER_HISTORY_LIMIT");
    expect(probe).toContain("native_broker_status: Option<super::NativeBrokerStatus>");
    expect(probe).toContain("native_broker: inputs");
    expect(probe).toContain("start_observer(project_path: &str)");
    expect(probe).toContain("if inputs.native_hook.source_available");
    expect(probe).toContain("native_hook_observation_for_process(");
    expect(probe).toContain("state.state_plane = ObservedStatePlane");

    expect(backend).toContain("read_native_broker_status_payload_from_shared_memory");
    expect(backend).toContain('r"Local\\LocusNativeBrokerState_{}"');
    expect(native).toContain("struct NativeStatePlane");
    expect(native).toContain("NATIVE_STATE_MMF_SLOT_COUNT");
    expect(native).toContain("broker_state_mmf_v1");
    expect(bridge).not.toContain("MemoryMappedFile.CreateOrOpen(");
  });
});
