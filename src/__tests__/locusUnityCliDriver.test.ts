import { describe, expect, it } from "vitest";
import { readFileSync } from "node:fs";

function read(path: string): string {
  return readFileSync(path, "utf8");
}

describe("Locus Unity CLI driver", () => {
  it("exposes the repository script entry", () => {
    const pkg = read("package.json");
    const script = read("scripts/locus-unity-test.mjs");
    const normalizedScript = script.replace(/\r\n/g, "\n");

    expect(pkg).toContain('"locus:test:unity": "bun run scripts/locus-unity-test.mjs"');
    expect(pkg).toContain('"locus:test:unity:native"');
    expect(pkg).toContain("--suite connect,native-bridge");
    expect(pkg).toContain("--connect-timeout-ms 60000");
    expect(pkg).toContain("--no-progress-timeout-ms 60000");
    expect(pkg).toContain('"locus:test:unity:smoke"');
    expect(pkg).toContain("--suite connect,native-bridge,state-probe");
    expect(pkg).toContain('"locus:test:unity:release"');
    expect(pkg).toContain("--suite connect,hot-reload-release");
    expect(pkg).toContain('"locus:test:unity:full"');
    expect(pkg).toContain("--suite all --connect-timeout-ms 60000 --timeout-ms 1200000");
    expect(normalizedScript).toContain('"dev",\n    "--",\n    "--",\n    "--locus-driver"');
    expect(script).toContain('"--locus-driver"');
    expect(script).toContain('"unity-test"');
    expect(script).toContain('"dev"');
    expect(script).toContain("--prepare-native");
  });

  it("runs through the Rust backend without a WebView/CDP dependency", () => {
    const lib = read("src-tauri/src/lib.rs");
    const driver = read("src-tauri/src/cli_driver.rs");

    expect(lib).toContain("cli_driver::CliDriverConfig::from_env_args()");
    expect(lib).toContain("cli_driver::spawn(app.handle().clone(), workspace.clone(), cli_driver_config)");
    expect(lib.indexOf("setup_cli_driver_scheduled")).toBeLessThan(
      lib.indexOf("main_window_build_start"),
    );

    expect(driver).toContain("ensure_connected");
    expect(driver).toContain("unity_bridge::launch_project_with_options(project, launch_code_optimization)");
    expect(driver).toContain('"processId": launch.process_id');
    expect(driver).toContain('"codeOptimization": match launch_code_optimization');
    expect(driver).toContain("DEFAULT_CONNECT_TIMEOUT_MS: u64 = 60_000");
    expect(driver).toContain("DEFAULT_NO_PROGRESS_TIMEOUT_MS: u64 = 60_000");
    expect(driver).toContain("--no-progress-timeout-ms");
    expect(driver).toContain("connection_stalled");
    expect(driver).toContain("connection_progress_signature");
    expect(driver).toContain("prepare_suite_environment");
    expect(driver).toContain("run_event_selftest");
    expect(driver).toContain("CliDriverSuite::Sidecar");
    expect(driver).toContain("CliDriverSuite::TypeIndex");
    expect(driver).toContain("--type-index-sample");
    expect(driver).toContain("run_sidecar_suite");
    expect(driver).toContain("run_type_index_suite");
    expect(driver).toContain("CliDriverSuite::Execute");
    expect(driver).toContain("run_execute_suite");
    expect(driver).toContain("unity_bridge::unity_run_states");
    expect(driver).toContain("UNITY_EXECUTE_CANCELLED");
    expect(driver).toContain("unity_bridge::run_state_probe_selftest");
    expect(driver).toContain("unity_bridge::run_native_bridge_selftest");
    expect(driver).toContain("crate::unity_hotreload::selftest::run");
    expect(driver).toContain("let mut suite_failures = Vec::new();");
    expect(driver).toContain('"suite_error"');
    expect(driver).toContain('"suite_no_progress"');
    expect(driver).toContain("should_stop_after_suite_error");
    expect(driver).toContain("format_suite_failures");
    expect(driver).toContain("Unity integration test suite(s) failed");
    expect(driver).toContain("LOCUS_DRIVER_JSON");
    expect(driver).not.toContain("remote-debugging-port");
    expect(driver).not.toContain("chrome-devtools");
  });

  it("confirms the native broker is the active transport", () => {
    const driver = read("src-tauri/src/cli_driver.rs");

    expect(driver).toContain("async fn resolve_active_transport");
    expect(driver).toContain('return "native_broker";');
    expect(driver).toContain("native_transport_confirmed");
    // The native-bridge suite hard-fails unless the required broker is active.
    expect(driver).toContain("expected 'native_broker'");
  });

  it("exposes the Settings integration-test runner", () => {
    const lib = read("src-tauri/src/lib.rs");
    const system = read("src-tauri/src/commands/system.rs");
    const service = read("src/services/integrationTests.ts");
    const settings = read("src/components/settings/TestingSettings.vue");
    const typeIndex = read("src-tauri/src/unity_type_index_selftest.rs");

    expect(lib).toContain("commands::unity_integration_test_run");
    expect(system).toContain("pub async fn unity_integration_test_run");
    expect(service).toContain("unity_integration_test_run");
    expect(service).toContain("unity-integration-test");
    expect(service).toContain('"execute"');
    expect(settings).toContain('id: "execute"');
    expect(settings).toContain("typeIndexSampleMode");
    expect(settings).toContain("currentRunId.value = \"\";");
    expect(settings).toContain("noProgressTimeoutMs: 60_000");
    expect(settings).toContain('case "suite_no_progress":');
    expect(settings).toContain('case "suite_error":');
    expect(settings).toContain("markRunningSuitesMissingResult");
    expect(settings).toContain('"sample32"');
    expect(settings).toContain('"all"');
    expect(settings).not.toContain("@tauri-apps/plugin-dialog");
    expect(settings).not.toContain("getWorkingDir");
    expect(settings).not.toContain("browseProjectPath");
    expect(settings).not.toContain("projectPath:");
    expect(typeIndex).toContain("TypeIndexSampleMode");
    expect(typeIndex).toContain("Sample32");
    expect(typeIndex).toContain("All");
  });

  it("keeps the Hot Reload self-test owned by the driver task", () => {
    const selftest = read("src-tauri/src/unity_hotreload/selftest.rs");

    expect(selftest).toContain("struct SelfTestRunningGuard;");
    expect(selftest).toContain("impl Drop for SelfTestRunningGuard");
    expect(selftest).toContain("test.run().await;");
    expect(selftest).not.toContain("tauri::async_runtime::spawn(async move");
  });

  it("guards the Type Index suite against transient Unity reload windows", () => {
    const bridge = read("src-tauri/src/unity_bridge/mod.rs");
    const params = read("src-tauri/src/csharp_compile/params.rs");
    const driver = read("src-tauri/src/cli_driver.rs");
    const settings = read("src/components/settings/TestingSettings.vue");

    expect(bridge).toContain("send_message_with_transient_retry");
    expect(bridge).toContain("SHORT_MESSAGE_TRANSIENT_RETRY_ATTEMPTS");
    expect(bridge).toContain("wait_for_unity_bridge_ready(project_path, SHORT_MESSAGE_TRANSIENT_READY_WAIT");
    expect(bridge).toContain("send_message_without_timeout_with_transient_retry(project_path, message_type, &payload)");
    expect(params).toContain("send_message_with_transient_retry(");
    expect(driver).toContain("emit_suite_failure(sink, suite, &error);");
    expect(settings).toContain("const previousLine = suiteLogs[target][suiteLogs[target].length - 1];");
    expect(settings).toContain("if (previousLine === line) return;");
  });

  it("keeps Type Index include-all discover large enough for dense Unity assets", () => {
    const typeIndex = read("src-tauri/src/unity_type_index_selftest.rs");
    const viewBindings = read("locus_unity/Editor/LocusBridge.ViewBindings.cs");

    expect(typeIndex).toContain("DISCOVER_INCLUDE_ALL_MAX_RESULTS: i32 = 50_000");
    expect(typeIndex).toContain(
      "if include_all { DISCOVER_INCLUDE_ALL_MAX_RESULTS } else { DISCOVER_MAX_RESULTS }",
    );
    expect(viewBindings).toContain("ViewBindingIncludeAllDiscoverMaxResults = 50000");
    expect(viewBindings).toContain("ViewBindingFilteredDiscoverMaxResults = 500");
  });

  it("streams Type Index progress every ~1% of sampled targets", () => {
    const typeIndex = read("src-tauri/src/unity_type_index_selftest.rs");
    const driver = read("src-tauri/src/cli_driver.rs");

    // Progress is computed over the sampled targets and throttled to whole percents,
    // collapsing to one line per target when fewer than 100 are sampled.
    expect(typeIndex).toContain("pub struct TypeIndexProgress");
    expect(typeIndex).toContain(
      "fn next_progress_percent(processed: u32, total: u32, last_percent: u32) -> Option<u32>",
    );
    expect(typeIndex).toContain("on_progress: &mut (dyn FnMut(TypeIndexProgress) + Send)");
    expect(typeIndex).toContain(
      "next_progress_percent(processed_targets, total_targets, last_percent)",
    );

    // The driver turns each milestone into a neutral suite_event progress line.
    expect(driver).toContain(
      "crate::unity_type_index_selftest::run(project, sample_mode, &mut on_progress)",
    );
    expect(driver).toContain("type-index: {}/{} targets ({}%) · {} properties checked");
  });

  it("keeps undersized Type Index samples as warnings", () => {
    const typeIndex = read("src-tauri/src/unity_type_index_selftest.rs");
    const driver = read("src-tauri/src/cli_driver.rs");

    expect(typeIndex).toContain("pub warnings: Vec<String>");
    expect(typeIndex).toContain("sample32 requested {} targets, found {}; continuing with available targets");
    expect(typeIndex).toContain("summary.warning(\"no custom ScriptableObject or prefab component targets found\")");
    expect(typeIndex).toContain("summary.warning(\"no target was eligible for static schema enrichment\")");
    expect(driver).toContain("WARN  type-index: {warning}");
    expect(driver).toContain("\"warnings\": summary.warnings");
    expect(driver).toContain("if summary.failed == 0");
  });

  it("uses driver JSON as the authoritative Unity test result", () => {
    const script = read("scripts/locus-unity-test.mjs");

    expect(script).toContain("runUnityDriver");
    expect(script).toContain("LOCUS_DRIVER_JSON ");
    expect(script).toContain("extractJsonObject");
    expect(script).toContain('event?.event === "finished"');
    expect(script).toContain("state.finishedOk = true");
    expect(script).toContain("recent driver events");
    expect(script).toContain("driverError");
    expect(script).toContain("terminalEventSeen");
    expect(script).toContain("--output-dir");
    expect(script).toContain('mkdtempSync(join(tmpdir(), "locus-unity-test-"))');
    expect(script).toContain('const logPath = join(logDir, "driver.log");');
    expect(script).toContain("logStream.write(chunk)");
    expect(script).toContain("replayDriverEventsFromLog");
    expect(script).toContain("readTextFileTail");
    expect(script).toContain("terminateChildTree");
    expect(script).toContain('"taskkill"');
    expect(script).toContain('"/T"');
    expect(script).toContain("treating the driver result as authoritative");
    expect(script).toContain("process.exit(0);");
  });

  it("exposes a serial Unity project matrix runner", () => {
    const pkg = read("package.json");
    const script = read("scripts/locus-unity-test-matrix.mjs");
    const viteConfig = read("vite.config.ts");

    expect(pkg).toContain('"locus:test:unity:matrix"');
    expect(pkg).toContain("scripts/locus-unity-test-matrix.mjs --prepare-native --install-plugin");
    expect(pkg).toContain('"locus:test:unity:matrix:smoke"');
    expect(pkg).toContain("--suite connect,native-bridge,state-probe");
    expect(script).toContain('path.join(repoRoot, "testproject")');
    expect(script).toContain('"ProjectSettings", "ProjectVersion.txt"');
    expect(script).toContain('"scripts", "locus-unity-test.mjs"');
    expect(script).toContain("--project-root");
    expect(script).toContain("--project");
    expect(script).toContain("--include");
    expect(script).toContain("--exclude");
    expect(script).toContain("--jobs");
    expect(script).toContain("--output-dir");
    expect(script).toContain('"matrix.log"');
    expect(script).toContain('"project.log"');
    expect(script).toContain('"driver.log"');
    expect(script).toContain('"summary.json"');
    expect(script).toContain('driverArgs.push("--suite", "connect,native-bridge")');
    expect(script).toContain("Parallel Unity matrix jobs are currently disabled");
    expect(script).toContain("Vite strictPort 14901");
    expect(script).toContain("per-worker devUrl/port isolation or a built-app runner");
    expect(viteConfig).toContain('"**/testproject/**"');
  });

  it("uses the native broker as the only Unity command transport", () => {
    const transport = read("src-tauri/src/unity_bridge/transport.rs");

    expect(transport).toContain("Native-only: all desktop Unity traffic goes through the broker pipe");
    expect(transport).toContain("Unity native broker is disabled");
    expect(transport).toContain("get_native_pipe_name(project_path)");
    expect(transport).not.toContain("get_pipe_name(project_path)");
    expect(transport).not.toContain("falling back to managed pipe");
  });
});
