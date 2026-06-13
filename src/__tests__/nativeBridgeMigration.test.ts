import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const cwd = process.cwd();

function read(relPath: string) {
  return readFileSync(resolve(cwd, relPath), "utf8");
}

describe("native bridge migration", () => {
  it("uses one short hash native pipe path across marker, managed, native, and transport", () => {
    const bridge = read("src-tauri/src/unity_bridge/mod.rs");
    const managed = read("locus_unity/Editor/LocusBridge.Native.cs");
    const native = read("locus_native_plugin/src/lib.rs");
    const transport = read("src-tauri/src/unity_bridge/transport.rs");

    expect(bridge).toContain("let body = format!(\"{}\\n\", get_native_pipe_name(project_path));");
    expect(bridge).toContain("project_state_plane_key(project_path)");
    expect(managed).toContain('private const string NativePipePrefix = @"\\\\.\\pipe\\";');
    expect(managed).toContain("NormalizeNativePipeName(line)");
    expect(managed).toContain("NativeProjectKey(projectPath)");
    expect(native).toContain("fn normalize_pipe_name(pipe_name: String) -> String");
    expect(native).toContain('format!(r"\\\\.\\pipe\\{}", value.trim_start_matches(\'\\\\\'))');
    expect(transport).toContain("get_native_pipe_name(project_path)");
    expect(transport).not.toContain("get_pipe_name(project_path)");
  });

  it("keeps native broker status cheap and splits broker vs managed capabilities", () => {
    const native = read("locus_native_plugin/src/lib.rs");
    const bridge = read("src-tauri/src/unity_bridge/mod.rs");
    const managed = read("locus_unity/Editor/LocusBridge.Native.cs");

    expect(native).toContain("process_id: std::process::id()");
    expect(native).toContain("process_path: std::env::current_exe()");
    expect(native).toContain("process_id: Some(self.process_id)");
    expect(native).toContain("process_path: if self.process_path.is_empty()");
    expect(native).toContain('"brokerCapabilities": NATIVE_CAPABILITIES');
    expect(native).toContain('"managedCapabilities": managed_capabilities');
    expect(native).toContain("guard.clear();");
    expect(native).toContain("const NATIVE_CAPABILITIES: &str =");
    expect(native).toContain('"broker_v1,broker_state_mmf_v1,broker_queue_limits_v1"');
    expect(managed).toContain('private const string ManagedCapabilities = "managed_executor_v1,status_cached,set_editor_status_async";');
    expect(bridge).toContain("pub broker_capabilities: Vec<String>");
    expect(bridge).toContain("pub managed_capabilities: Vec<String>");
  });

  it("bounds native broker queues and keeps the shared-memory state plane publishing", () => {
    const native = read("locus_native_plugin/src/lib.rs");
    const bridge = read("src-tauri/src/unity_bridge/mod.rs");

    expect(native).toContain("const MAX_PENDING_REQUESTS: usize = 128;");
    expect(native).toContain("const MAX_INFLIGHT_REQUESTS: usize = 64;");
    expect(native).toContain("const MAX_REQUEST_BYTES: usize = 16 * 1024 * 1024;");
    expect(native).toContain('return Err("native_queue_full");');
    expect(native).toContain('return Err("native_inflight_full");');
    expect(native).toContain('return Err("native_payload_too_large");');
    expect(native).toContain('self.respond_error(&id, "native_request_timed_out");');
    expect(native).toContain("mpsc::channel::<Vec<u8>>(WRITER_CHANNEL_LIMIT)");
    expect(native).toContain("tx.try_send(bytes)");
    expect(native).toContain("NATIVE_STATE_MMF_MAX_PAYLOAD");
    expect(native).toContain("events.drain(0..drop_count.min(events.len()))");
    expect(native).toContain("self.write_u64(slot_offset, 0);");
    expect(native).toContain("shared_memory_status_trims_events_to_payload_budget");
    expect(bridge).toContain("let payload_bytes = slot");
    expect(bridge).toContain(".get(payload_offset..payload_offset + payload_len)?");
    expect(bridge).toContain(".to_vec();");
    expect(bridge).toContain("let slot_seq_after = read_u64(slot, 0)?;");
    expect(bridge).toContain("let writer_seq_after = read_u64(bytes, 16)?;");
  });

  it("gates readiness and semantic state on native managedState", () => {
    const bridge = read("src-tauri/src/unity_bridge/mod.rs");
    const probe = read("src-tauri/src/unity_bridge/state_probe.rs");
    const native = read("locus_native_plugin/src/lib.rs");

    expect(bridge).toContain("Native broker managed state is");
    expect(bridge).toContain("native_managed_not_ready");
    expect(probe).toContain("native_broker_status: Option<super::NativeBrokerStatus>");
    expect(probe).toContain('SemanticPhase::Reloading');
    expect(probe).toContain('"native_broker"');
    expect(native).toContain('self.respond_error(id, "managed_reloading");');
    expect(native).toContain('self.respond_error(id, "managed_not_ready");');
  });

  it("adds a live native bridge self-test command and settings page", () => {
    const backend = read("src-tauri/src/unity_bridge/native_selftest.rs");
    const system = read("src-tauri/src/commands/system.rs");
    const app = read("src-tauri/src/lib.rs");
    const service = read("src/services/csharpLsp.ts");
    const settings = read("src/components/SettingsView.vue");
    const testing = read("src/components/settings/TestingSettings.vue");
    const zh = read("src/language/zh.json");
    const en = read("src/language/en.json");

    expect(backend).toContain("unity-native-bridge-selftest");
    expect(backend).toContain("RequestScriptReload");
    expect(backend).toContain("query_native_broker_status");
    expect(backend).toContain("reload_request_was_accepted");
    expect(backend).toContain('"managed_reloading" | "domain_reload_interrupted"');
    expect(system).toContain("pub async fn unity_native_bridge_selftest_run");
    expect(app).toContain("commands::unity_native_bridge_selftest_run");
    expect(service).toContain("subscribeUnityNativeBridgeSelfTest");
    expect(settings).toContain("TestingSettings");
    expect(settings).toContain("activeCategory === 'testing'");
    expect(testing).toContain("unityNativeBridgeSelfTestRun");
    expect(zh).toContain('"settings.tab.testing": "测试"');
    expect(en).toContain('"settings.tab.testing": "Testing"');
  });

  it("keeps Unity asset import workers from owning the native broker pipe", () => {
    const bridge = read("locus_unity/Editor/LocusBridge.cs");
    const managed = read("locus_unity/Editor/LocusBridge.Native.cs");

    expect(bridge).toContain("private static readonly bool _isUnityWorkerProcess = DetectUnityWorkerProcess();");
    expect(bridge).toContain('arg.IndexOf("AssetImportWorker", StringComparison.OrdinalIgnoreCase) >= 0');
    expect(bridge).toContain("NativeShutdownInWorkerProcess();");
    expect(bridge).toContain("if (_isUnityWorkerProcess)\n                return;\n\n            NativeStartIfEnabled();");
    expect(managed).toContain("if (_isUnityWorkerProcess)\n                return;");
    expect(managed).toContain("private static void NativeShutdownInWorkerProcess()");
    expect(managed).toContain("locus_shutdown();");
  });

  it("defaults the native command channel on, with an explicit opt-out", () => {
    const config = read("src-tauri/src/config.rs");
    const driver = read("src-tauri/src/cli_driver.rs");

    expect(config).toContain(
      "Default-on: the native command channel survives domain reloads",
    );
    expect(config).toContain("fn unity_native_bridge_defaults_to_enabled()");
    expect(config).toContain("assert!(config.unity_native_bridge_enabled());");
    expect(config).toContain("fn unity_native_bridge_respects_explicit_opt_out()");
    // The CLI surfaces the resolved transport on connect so a default-on run is observable.
    expect(driver).toContain('"transport": transport,');
  });

  it("runs the background hook in-process via the broker, failing open to the cross-process patch", () => {
    const native = read("locus_native_plugin/src/lib.rs");
    const bridge = read("src-tauri/src/unity_bridge/mod.rs");
    const managed = read("locus_unity/Editor/LocusBridge.Native.cs");

    // native in-process patcher + FFI + status surface
    expect(native).toContain("pub extern \"C\" fn locus_set_background_active");
    expect(native).toContain("const PATCH_BYTES: [u8; 6] = [0xB8, 0x01, 0x00, 0x00, 0x00, 0xC3];");
    expect(native).toContain('"Unity!IsApplicationActive"');
    expect(native).toContain('"backgroundPatched": background_patched');
    expect(native).toContain("if !st.records.is_empty()");
    expect(native).toContain("return Ok(st.symbol_count);");
    // Tauri gates the cross-process patch on native ownership (fail-open)
    expect(bridge).toContain("async fn native_owned_background_hook");
    expect(bridge).toContain("pub background_patched: bool");
    expect(bridge).toContain("pub fn sync_background_hook_marker");
    expect(bridge).toContain("BackgroundHook.enabled");
    // managed applies it from the marker, idempotent across reloads
    expect(managed).toContain("locus_set_background_active");
    expect(managed).toContain("NativeApplyBackgroundHook()");
    expect(managed).toContain("NativeBackgroundHookEnabled()");
    expect(managed).toContain("bool wasApplied = _nativeBackgroundHookApplied;");
    expect(managed).toContain("if (_nativeBackgroundHookApplied && !wasApplied)");
  });

  it("routes the overlay control pipe through the native client and retains it across reloads", () => {
    const native = read("locus_native_plugin/src/lib.rs");
    const embed = read("src-tauri/src/commands/unity_embed.rs");
    const window = read("locus_unity/Editor/LocusEditorWindow.cs");
    const managed = read("locus_unity/Editor/LocusBridge.Native.cs");

    // native owns a persistent write-only overlay client
    expect(native).toContain('pub unsafe extern "C" fn locus_overlay_connect');
    expect(native).toContain('pub unsafe extern "C" fn locus_overlay_push');
    expect(native).toContain("mod overlay {");
    // Tauri server retains the overlay on the reloading marker
    expect(embed).toContain("managed_overlay_state: String");
    expect(embed).toContain('if msg.managed_overlay_state == "reloading"');
    // managed routes through native (fail-open) and signals reloading
    expect(window).toContain("LocusBridge.NativeOverlayConnect(pipeName)");
    expect(window).toContain("SendOverlayReloading()");
    expect(managed).toContain("internal static bool NativeOverlayPush");
    expect(managed).toContain("internal static bool IsNativeBridgeActive");
  });

  it("drops stale native completions after reload or client disconnect cleanup", () => {
    const native = read("locus_native_plugin/src/lib.rs");

    expect(native).toContain("dropping stale completion for non-inflight request");
    expect(native).toContain("inflight.remove(id).is_some()");
    expect(native).toContain("complete_drops_stale_or_already_interrupted_requests");
  });

  it("closes and relaunches Unity around plugin installs", () => {
    const plugin = read("src-tauri/src/unity_bridge/plugin.rs");
    const process = read("src-tauri/src/unity_bridge/process.rs");
    const cli = read("src-tauri/src/cli_driver.rs");
    const workspace = read("src-tauri/src/commands/workspace.rs");

    expect(plugin).toContain("close_current_project_unity_processes");
    expect(plugin).toContain("Unity closed for plugin update");
    expect(plugin).toContain("PLUGIN_INSTALL_LOCK_RELEASE_SETTLE");
    expect(plugin).toContain("plugin install hit a locked file after Unity close");
    expect(plugin).toContain("super::launch_project(project_path)?");
    expect(process).toContain("taskkill");
    expect(process).toContain("force_close_current_project_unity_processes");
    expect(process).toContain("unity_process_args_are_worker");
    expect(cli).toContain("check_or_install_plugin(&project, config.install_plugin).await?");
    expect(workspace).toContain("install_or_update_plugin_with_force_close");
  });
});
