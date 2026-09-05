import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const cwd = process.cwd();

function read(relPath: string) {
  return readFileSync(resolve(cwd, relPath), "utf8");
}

describe("runtime CDP debug mode", () => {
  it("starts and stops the loopback CDP server with persisted debug mode", () => {
    const command = read("src-tauri/src/commands/workspace.rs");
    const runtime = read("src-tauri/src/cdp_debug.rs");

    expect(command).toMatch(
      /set_debug_mode[\s\S]*?set_debug_enabled\(value\)[\s\S]*?cdp_debug::reconcile\(app_handle, value\)/,
    );
    expect(runtime).toContain('TcpListener::bind(("127.0.0.1", port))');
    expect(runtime).toContain("if !enabled {");
    expect(runtime).toContain("stop_locked(&app, &handle, &mut running).await;");
    expect(runtime).toContain("task.abort();");
  });

  it("keeps the disabled path free of listeners, polling, and WebView2 startup flags", () => {
    const runtime = read("src-tauri/src/cdp_debug.rs");
    const app = read("src-tauri/src/lib.rs");
    const frontendDiagnostics = read("src/services/webviewBridgeDiagnostics.ts");

    expect(runtime.indexOf("if !enabled {")).toBeLessThan(runtime.indexOf("bind_listener().await"));
    expect(app).not.toContain("--remote-debugging-port");
    expect(app).not.toContain("WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS");
    expect(app).toContain('if app.state::<Arc<AppConfig>>().debug_enabled() {');
    expect(frontendDiagnostics).toContain(
      'performanceDiagnosticsModulePromise ??= import("./runtimePerformanceDiagnostics")',
    );
    expect(frontendDiagnostics).toMatch(
      /function startDiagnostics[\s\S]*?startPerformanceDiagnostics\(\)[\s\S]*?scheduleHeartbeat/,
    );
    expect(frontendDiagnostics).toMatch(
      /function stopDiagnostics[\s\S]*?stopPerformanceDiagnostics\(\)/,
    );
  });

  it("exposes native WebView2 CDP calls and forwards runtime protocol events", () => {
    const runtime = read("src-tauri/src/cdp_debug.rs");

    expect(runtime).toContain("CallDevToolsProtocolMethod(");
    expect(runtime).toContain("CallDevToolsProtocolMethodForSession(");
    expect(runtime).toContain("GetDevToolsProtocolEventReceiver(");
    expect(runtime).toContain('"Runtime.consoleAPICalled"');
    expect(runtime).toContain('"Target.targetCreated"');
    expect(runtime).toContain('matches!(path, "/json" | "/json/list")');
    expect(runtime).toContain('path == "/json/version"');
  });

  it("assigns a distinct synthetic session to every browser-level attachment", () => {
    const runtime = read("src-tauri/src/cdp_debug.rs");

    expect(runtime).toContain("struct BrowserConnectionState");
    expect(runtime).toContain("self.next_session_sequence.saturating_add(1)");
    expect(runtime).toContain("browser_state.attach()");
    expect(runtime).toContain("browser_state.contains(session_id)");
    expect(runtime).toMatch(/browser_state\s*\.sessions/);
    expect(runtime).not.toContain('const MAIN_TARGET_SESSION_ID: &str');
  });

  it("provides an event-triggered circular stall trace recorder", () => {
    const packageJson = read("package.json");
    const recorder = read("scripts/locus-stall-recorder.ts");

    expect(packageJson).toContain('"locus:test:stall-capture"');
    expect(recorder).toContain('recordMode: "recordContinuously"');
    expect(recorder).toContain('transferMode: "ReturnAsStream"');
    expect(recorder).toContain('cdpClient.send("IO.read"');
    expect(recorder).toContain('event.method !== "Runtime.consoleAPICalled"');
    expect(recorder).toContain("__LOCUS_RUNTIME_PERFORMANCE_INCIDENT__");
    expect(recorder).toContain("createGzip");
  });
});
