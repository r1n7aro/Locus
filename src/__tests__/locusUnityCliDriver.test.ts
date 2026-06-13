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
    expect(pkg).toContain("--no-progress-timeout-ms 20000");
    expect(pkg).toContain('"locus:test:unity:smoke"');
    expect(pkg).toContain("--suite connect,native-bridge,state-probe");
    expect(pkg).toContain('"locus:test:unity:full"');
    expect(pkg).toContain("--suite all --connect-timeout-ms 60000 --timeout-ms 1200000");
    expect(normalizedScript).toContain('"dev",\n  "--",\n  "--",\n  "--locus-driver"');
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
    expect(driver).toContain("unity_bridge::launch_project(project)");
    expect(driver).toContain("DEFAULT_CONNECT_TIMEOUT_MS: u64 = 60_000");
    expect(driver).toContain("DEFAULT_NO_PROGRESS_TIMEOUT_MS: u64 = 20_000");
    expect(driver).toContain("--no-progress-timeout-ms");
    expect(driver).toContain("connection_stalled");
    expect(driver).toContain("connection_progress_signature");
    expect(driver).toContain("prepare_suite_environment");
    expect(driver).toContain("run_event_selftest");
    expect(driver).toContain("unity_bridge::run_state_probe_selftest");
    expect(driver).toContain("unity_bridge::run_native_bridge_selftest");
    expect(driver).toContain("crate::unity_hotreload::selftest::run");
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
    expect(script).toContain('mkdtempSync(join(tmpdir(), "locus-unity-test-"))');
    expect(script).toContain('const logPath = join(logDir, "driver.log");');
    expect(script).toContain("logStream.write(chunk)");
    expect(script).toContain("replayDriverEventsFromLog");
    expect(script).toContain("readTextFileTail");
    expect(script).toContain("terminateChildTree");
    expect(script).toContain('"taskkill"');
    expect(script).toContain('"/T"');
    expect(script).toContain("treating the driver result as authoritative");
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
