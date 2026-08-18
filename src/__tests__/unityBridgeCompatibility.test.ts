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

  it("acknowledges recompile only after Unity starts the requested epoch", () => {
    const bridge = read("locus_unity/Editor/LocusBridge.cs");
    const requestHandler = bridge.slice(
      bridge.indexOf('case "request_recompile":'),
      bridge.indexOf('case "begin_edit_session":'),
    );
    const compilationStarted = bridge.slice(
      bridge.indexOf("private static void OnCompilationStarted"),
      bridge.indexOf("private static void OnAssemblyCompilationFinished"),
    );

    expect(requestHandler).toContain("return await startCompletion.Task.ConfigureAwait(false);");
    expect(requestHandler).not.toContain('return OkResponse(reqId, "recompile_started")');
    expect(compilationStarted).toContain('CompleteRecompileStartResponse();');
    expect(compilationStarted).toContain('SetCompileResult("pending");');
    expect(bridge).toContain('OkResponse(requestId, "recompile_started")');
    expect(bridge).toContain('SetCompileResult("starting")');
    expect(bridge).toContain("Unity 没有开始编译。");
    expect(bridge).toContain("Unity 没有开始编译。未找到活动重编译请求。");
    expect(bridge).toContain("RecompileStartIdleTimeoutSeconds = 30.0");

    const targetPersisted = requestHandler.indexOf(
      "SessionState.SetInt(SessionKey_RecompileTargetEpoch, targetEpoch)",
    );
    expect(targetPersisted).toBeGreaterThan(-1);
    expect(targetPersisted).toBeLessThan(requestHandler.indexOf("ReleaseAllEditSessions();"));
    expect(targetPersisted).toBeLessThan(requestHandler.indexOf("AssetDatabase.Refresh();"));
    expect(targetPersisted).toBeLessThan(
      requestHandler.indexOf("CompilationPipeline.RequestScriptCompilation();"),
    );
    expect(bridge).not.toContain("RecompileCheckDelayFrames");
    expect(bridge).not.toContain("_recompileCheckFrames");
  });

  it("rechecks persisted compile state after a bridge reconnect", () => {
    const bridge = read("src-tauri/src/unity_bridge/mod.rs");
    const reconnectBranch = bridge.slice(
      bridge.indexOf("if disconnected {", bridge.indexOf("async fn recompile_and_wait_inner")),
      bridge.indexOf('"get_compile_result"', bridge.indexOf("async fn recompile_and_wait_inner")),
    );

    expect(reconnectBranch).toContain("disconnected = false;");
    expect(reconnectBranch).not.toContain("finish_recompile_success");
    expect(bridge).toContain('"starting" | "pending" => Ok(RecompilePollState::Waiting)');
    expect(bridge).toContain('"ok" => Ok(RecompilePollState::Completed)');
    expect(bridge).toContain("RECOMPILE_TOTAL_TIMEOUT");
    expect(bridge).toContain("RECOMPILE_START_CONFIRM_TIMEOUT: Duration = Duration::from_secs(90)");
    expect(bridge).toContain("RECOMPILE_TOTAL_TIMEOUT: Duration = Duration::from_secs(300)");
    expect(bridge).toContain("send_message_without_timeout_with_acceptance(");
    expect(bridge).toContain("state_probe::semantic_state_for_project(project_path).await");
    expect(bridge).toContain("Native Broker 已接收请求");
    expect(bridge).toContain("main_thread=");
    expect(bridge).toContain("RecompileStartAck::Unconfirmed");
    expect(bridge).toContain("recompile_timeout_reason(&state)");
    expect(bridge).not.toContain("Unity 最终状态：");
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
