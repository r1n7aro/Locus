import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const cwd = process.cwd();

function read(relPath: string) {
  return readFileSync(resolve(cwd, relPath), "utf8");
}

describe("unityBridgeCompatibility", () => {
  it("does not start the legacy managed command pipe", () => {
    const bridge = read("locus_unity/Editor/LocusBridge.cs");

    expect(bridge).toContain("NativeStartIfEnabled();");
    expect(bridge).toContain("Native broker bridge is required but did not start.");
    expect(bridge).not.toContain("NamedPipeServerStream");
    expect(bridge).not.toContain("WaitForConnectionCompat");
    expect(bridge).not.toContain("ServerPipeOptions");
    expect(bridge).not.toContain("SendEnvelopeAsync");
  });

  it("keeps the Unity bridge connection stable after recompilation", () => {
    const bridge = read("src-tauri/src/unity_bridge/mod.rs");
    const transport = read("src-tauri/src/unity_bridge/transport.rs");

    expect(bridge).toContain("wait_for_unity_bridge_ready_after_recompile");
    expect(bridge).toContain("refresh_unity_type_index_after_recompile");
    expect(bridge).toContain("Unity reconnected after domain reload");
    expect(bridge).not.toContain("Unity recompile completed");
    expect(transport).toContain(".filter(|value| !value.is_empty())");
  });

  it("samples Unity editor state only for confirmed bridge work or outbound updates", () => {
    const bridge = read("locus_unity/Editor/LocusBridge.cs");
    const pump = bridge.slice(
      bridge.indexOf("private static void PumpMainThreadQueue()"),
      bridge.indexOf("private static bool HasAnyDesktopConnection()"),
    );
    const editorUpdate = bridge.slice(
      bridge.indexOf("private static void MaybeSendEditorUpdateEvent()"),
      bridge.indexOf("private static EditorSelectionSnapshot BuildEditorSelectionSnapshot"),
    );
    const statusHandler = bridge.slice(
      bridge.indexOf("private static PipeEnvelope HandleStatus"),
      bridge.indexOf("private static string BuildCachedEditorStatusMessage"),
    );

    expect(bridge).not.toContain("_desktopPipeConnected");
    expect(bridge).not.toContain("_currentServer");
    expect(pump).toContain("NativePump();");
    expect(pump).toContain("bool desktopConnected = HasAnyDesktopConnection();");
    expect(pump).toContain("bool hasRuntimeWork = HasMainThreadRuntimeWork();");
    expect(pump).toContain("if (hasRuntimeWork)");
    expect(pump).toContain("RefreshCachedEditorState();");
    expect(pump).toMatch(/if \(_activeRunStatesSession != null\)\s+PumpRunStates\(\);/);
    expect(pump).toMatch(/if \(HasActiveExecuteCodeAsyncRuntime\(\)\)\s+PumpExecuteCodeAsyncRuntime\(\);/);
    expect(pump).toMatch(/if \(desktopConnected\)\s+MaybeSendEditorUpdateEvent\(\);/);
    expect(editorUpdate).toContain("int selectionInstanceId = LocusObjectIdentity.InstanceId(selection);");
    expect(editorUpdate).toContain("RefreshCachedEditorState();");
    expect(bridge).toContain("private static bool HasAnyDesktopConnection()");
    expect(bridge).toContain("return IsNativeBridgeActive;");
    expect(bridge).toContain('case "status":');
    expect(bridge).toContain("return HandleStatus(reqId);");
    expect(statusHandler).not.toContain("PostToMainThread(delegate");
    expect(statusHandler).not.toContain("RefreshCachedEditorState();");
    expect(statusHandler).toContain("OkStatusResponse(requestId)");
    expect(bridge).toContain("private static PipeEnvelope OkStatusResponse(string replyTo)");
    expect(bridge).toContain("OkResponse(replyTo, BuildCachedEditorStatusMessage())");
    expect(bridge).toContain("response.processId = _editorProcessId;");
    expect(bridge).toContain("response.processPath = _editorProcessPath;");
  });

  it("keeps transient View assemblies out of the Unity type index", () => {
    const typeIndex = read("locus_unity/Editor/LocusBridge.TypeIndex.cs");
    const viewScripts = read("locus_unity/Editor/LocusBridge.ViewScripts.cs");
    const bridge = read("locus_unity/Editor/LocusBridge.cs");

    expect(typeIndex).toContain('assemblyName.StartsWith("__LocusView_"');
    expect(typeIndex).toContain("IsInactiveSkillPackageAssemblyName(assemblyName)");
    expect(viewScripts).toContain("PreviousAssemblyId");
    expect(viewScripts).toContain("FindActiveSkillPackageAssembly");
    expect(viewScripts).toContain('\\"previousAssemblyId\\"');
    expect(viewScripts).toContain("HandleInvokeSkillPackage");
    expect(bridge).toContain("preprocessorSymbols: SnippetPreprocessorSymbols");
    expect(bridge).toContain("AddUnityVersionPreprocessorSymbols");
  });

  it("keeps the cached Unity pipe connection after a response timeout", () => {
    const transport = read("src-tauri/src/unity_bridge/transport.rs");
    const responseTimeoutBranch = transport.slice(
      transport.indexOf('let err = "Unity response timed out".to_string();'),
      transport.indexOf("} else {\n            match rx.await"),
    );

    expect(transport).toContain('let err = "Unity response timed out".to_string();');
    expect(responseTimeoutBranch).toContain("pending.remove(&request_id);");
    expect(responseTimeoutBranch).not.toContain("remove_connection_if_same");
    expect(responseTimeoutBranch).not.toContain("close_connection");
  });
});
