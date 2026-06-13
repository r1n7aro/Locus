import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const cwd = process.cwd();

function read(relPath: string) {
  return readFileSync(resolve(cwd, relPath), "utf8").replace(/\r\n/g, "\n");
}

describe("Unity bridge status handling", () => {
  it("keeps status responsive while play mode is paused", () => {
    const bridge = read("locus_unity/Editor/LocusBridge.cs");
    const runStates = read("locus_unity/Editor/LocusBridge.RunStates.cs");

    expect(bridge).toContain("RefreshCachedEditorState();\n            EditorApplication.update += PumpMainThreadQueue;");
    expect(bridge).toContain("EditorApplication.playModeStateChanged += OnPlayModeStateChanged;");
    expect(bridge).toContain("EditorApplication.pauseStateChanged += OnPauseStateChanged;");
    expect(bridge).toContain("AssemblyReloadEvents.beforeAssemblyReload += OnBeforeAssemblyReload;");
    expect(bridge).not.toContain("MemoryMappedFile.CreateOrOpen(");
    expect(bridge).not.toContain("WriteStatePlaneHint(");
    expect(bridge).toContain("NativeOnBeforeReload();");
    expect(bridge).toContain("NativePublishEditorStatusNow();");
    expect(bridge).toContain("private static PipeEnvelope HandleStatus(string requestId)");
    expect(bridge).toContain("case \"bridge_capabilities\":\n                        return OkResponse(reqId, \"managed_executor_v1,status_cached,set_editor_status_async\");");
    expect(bridge).toContain("return OkStatusResponse(requestId);");
    expect(bridge).not.toContain("case \"status\":\n                        return await HandleStatus(reqId);");
    expect(bridge).toContain("case \"set_editor_status\":\n                        return HandleSetEditorStatus(reqId, msg.message);");
    expect(runStates).toContain("private static PipeEnvelope HandleSetEditorStatus(string requestId, string desiredStatus)");

    const statusHandler = bridge.slice(
      bridge.indexOf("private static PipeEnvelope HandleStatus(string requestId)"),
      bridge.indexOf("private static string BuildCachedEditorStatusMessage()"),
    );
    expect(statusHandler).not.toContain("PostToMainThread");
    expect(statusHandler).not.toContain("TaskCompletionSource");

    const setStatusHandler = runStates.slice(
      runStates.indexOf("private static PipeEnvelope HandleSetEditorStatus(string requestId, string desiredStatus)"),
      runStates.indexOf("private static async Task<PipeEnvelope> HandleCompileRunStates"),
    );
    expect(setStatusHandler).not.toContain("TaskCompletionSource");
    expect(setStatusHandler).not.toContain("await");

    const pausedCaseIndex = runStates.indexOf("case \"playing_paused\":");
    const cacheIndex = runStates.indexOf("_isPaused = true;", pausedCaseIndex);
    const postIndex = runStates.indexOf("PostToMainThread(delegate", pausedCaseIndex);
    const pauseIndex = runStates.indexOf("EditorApplication.isPaused = true;", pausedCaseIndex);
    const ackIndex = runStates.indexOf("return OkResponse(requestId, \"playing_paused_requested\");", pausedCaseIndex);

    expect(pausedCaseIndex).toBeGreaterThan(-1);
    expect(cacheIndex).toBeGreaterThan(pausedCaseIndex);
    expect(postIndex).toBeGreaterThan(cacheIndex);
    expect(pauseIndex).toBeGreaterThan(postIndex);
    expect(ackIndex).toBeGreaterThan(postIndex);
  });
});
