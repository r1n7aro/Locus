// @vitest-environment jsdom
import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { createPinia } from "pinia";
import { createApp, nextTick } from "vue";
import { describe, expect, it } from "vitest";
import ToolCallBlock from "../components/ToolCallBlock.vue";

const root = process.cwd();
const read = (path: string) => readFileSync(resolve(root, path), "utf8");

describe("Unity Test Framework tools", () => {
  it("uses the official TestRunnerApi and preserves asynchronous UnityTest execution", () => {
    const service = read("locus_unity/Editor/Testing/LocusUnityTestService.cs");

    expect(service).toContain("TestRunnerApi");
    expect(service).toContain("Api.RetrieveTestList");
    expect(service).toContain("Api.Execute(new ExecutionSettings(filter))");
    expect(service).toContain("Api.RegisterCallbacks");
    expect(service).not.toContain("runSynchronously = true");
    expect(service).not.toContain("System.Reflection");
  });

  it("exposes a typed UnityTestApi for unity_execute without reflection", () => {
    const api = read("locus_unity/Editor/Testing/UnityTestApi.cs");
    const service = read("locus_unity/Editor/Testing/LocusUnityTestService.cs");
    const executeDefinition = JSON.parse(read("tools/unity_execute.json"));

    expect(api).toContain("public static class UnityTestApi");
    expect(api).toContain("Task<UnityTestListResult> ListAsync");
    expect(api).toContain("UnityTestRunSnapshot Start");
    expect(api).toContain("UnityTestRunSnapshot Status");
    expect(api).toContain("UnityTestRunSnapshot Cancel");
    expect(api).toContain("LocusUnityTestService.ListAsync");
    expect(api).toContain("LocusUnityTestService.Start");
    expect(api).not.toContain("System.Reflection");

    expect(service).toContain("internal static async Task<UnityTestListDto> ListAsync");
    expect(service).toContain("internal static UnityTestRunSnapshotDto Start");
    expect(service).toContain("internal static UnityTestRunSnapshotDto Status");
    expect(service).toContain("internal static UnityTestRunSnapshotDto Cancel");
    expect(executeDefinition.description).toContain("UnityTestApi.ListAsync");
    expect(executeDefinition.description).toContain("UnityTestApi.Start");
    expect(executeDefinition.description).toContain("Status(runId)");
    expect(executeDefinition.description).toContain("com.unity.test-framework");
  });

  it("compiles the adapter only when com.unity.test-framework is installed", () => {
    const asmdef = JSON.parse(
      read("locus_unity/Editor/Testing/Locus.UnityTesting.Editor.asmdef"),
    );
    const bridge = read("locus_unity/Editor/LocusBridge.Extensions.cs");

    expect(asmdef.references).toEqual(
      expect.arrayContaining(["Locus.Editor", "UnityEngine.TestRunner", "UnityEditor.TestRunner"]),
    );
    expect(asmdef.defineConstraints).toContain("LOCUS_HAS_UNITY_TEST_FRAMEWORK");
    expect(asmdef.versionDefines).toContainEqual(
      expect.objectContaining({
        name: "com.unity.test-framework",
        expression: "1.4.0",
        define: "LOCUS_HAS_UNITY_TEST_FRAMEWORK",
      }),
    );
    expect(bridge).toContain("RegisterExtensionMessageHandler");
  });

  it("requires workspace opt-in and the installed package across agent and MCP surfaces", () => {
    const workspace = read("src-tauri/src/workspace.rs");
    const agent = read("src-tauri/src/agent/instance/mod.rs");
    const mcp = read("src-tauri/src/mcp/server/tools.rs");
    const settings = read("src/components/settings/UnityConnectionSettings.vue");

    expect(workspace).toContain("enabled && package_installed && package_supported");
    expect(workspace).toContain("UNITY_TEST_FRAMEWORK_MIN_VERSION");
    expect(agent).toContain('"unity_test_list" | "unity_test_run"');
    expect(agent).toContain("unity_test_tools_available(&self.working_dir)");
    expect(mcp).toContain("unity_test_tools_workspace_status(working_dir)");
    expect(settings).toContain("status.packageSupported");
    expect(settings).toContain("unityTestPackageUnsupported");
  });

  it("holds discovery and execution until edited tests converge through a domain reload", () => {
    const workspace = read("src-tauri/src/workspace.rs");
    const filesystem = read("src-tauri/src/tool/builtins/filesystem.rs");
    const bridge = read("src-tauri/src/unity_bridge/mod.rs");
    const listDefinition = JSON.parse(read("tools/unity_test_list.json"));
    const runDefinition = JSON.parse(read("tools/unity_test_run.json"));

    expect(filesystem).toContain("note_unity_test_source_written");
    expect(workspace).toContain("unity_test_sources_pending");
    expect(bridge).toContain("require_unity_test_sources_converged");
    expect(bridge).toContain("clear_unity_test_pending_sources_through");
    expect(listDefinition.description).toContain("complete unity_recompile before discovery");
    expect(runDefinition.description).toContain("complete unity_recompile before running");
  });

  it("makes filters optional and accepts both Unity Test modes in one request", () => {
    const listDefinition = JSON.parse(read("tools/unity_test_list.json"));
    const runDefinition = JSON.parse(read("tools/unity_test_run.json"));
    const service = read("locus_unity/Editor/Testing/LocusUnityTestService.cs");
    const api = read("locus_unity/Editor/Testing/UnityTestApi.cs");

    for (const definition of [listDefinition, runDefinition]) {
      expect(definition.parameters.required).toEqual([]);
      expect(definition.parameters.properties.mode.enum).toEqual([
        "edit",
        "play",
        "edit|play",
      ]);
      expect(definition.parameters.properties.mode.default).toBe("edit|play");
      expect(definition.description).toMatch(/omit unused filter arrays/i);
    }
    expect(service).toContain("value.Split('|')");
    expect(service).toContain("TestMode.EditMode | TestMode.PlayMode");
    expect(service).toContain('return "edit|play";');
    expect(api).toContain("[Flags]");
    expect(api).toContain("EditAndPlay = Edit | Play");
    expect(api).toContain("UnityTestMode Mode = UnityTestMode.EditAndPlay");
  });

  it("saves every dirty persisted scene before starting Unity tests", () => {
    const service = read("locus_unity/Editor/Testing/LocusUnityTestService.cs");

    expect(service).toContain("SaveDirtyScenesBeforeRun();");
    expect(service).toContain("EditorSceneManager.sceneCount");
    expect(service).toContain("EditorSceneManager.GetSceneAt(index)");
    expect(service).toContain("scene.isDirty");
    expect(service).toContain("EditorSceneManager.IsPreviewScene(scene)");
    expect(service).toContain("!EditorSceneManager.SaveScene(scene) || scene.isDirty");
    expect(service).toContain("Save the untitled scene before starting Unity Tests");
    expect(service).not.toContain("SaveCurrentModifiedScenesIfUserWantsTo");
    expect(service.indexOf("SaveDirtyScenesBeforeRun();")).toBeLessThan(
      service.indexOf("state.Begin("),
    );
  });

  it("cancels the active Unity test whenever tool waiting is interrupted", () => {
    const agent = read("src-tauri/src/agent/instance/mod.rs");
    const bridge = read("src-tauri/src/unity_bridge/mod.rs");
    const dialog = read("src-tauri/src/unity_bridge/dialog.rs");
    const service = read("locus_unity/Editor/Testing/LocusUnityTestService.cs");
    const liveness = read("locus_unity/Editor/Testing/UnityTestRunLiveness.cs");
    const testingAssembly = JSON.parse(
      read("locus_unity/Editor/Testing/Locus.UnityTesting.Editor.asmdef"),
    );
    const runDefinition = JSON.parse(read("tools/unity_test_run.json"));

    expect(runDefinition.parameters.properties.resume_run_id).toBeUndefined();
    expect(runDefinition.description).toContain("Cancellation/host timeout cancels the run");
    expect(agent).toContain('if tc.name == "unity_test_run"');
    expect(agent).toContain("unity_test_cancellation_failed");
    expect(bridge).toContain("let mut dialog_events = dialog::subscribe()");
    expect(bridge).toContain("wait_for_unity_test_poll_wake");
    expect(bridge).toContain("dialog_events.has_changed()");
    expect(bridge).toContain("dispatch_unity_test_cancel");
    expect(bridge).toContain("cancel_unity_test_run");
    expect(bridge).toContain("unity_test_abort_error");
    expect(bridge).toContain("Err(error) if dialog::is_unity_modal_dialog_blocked_error(&error)");
    expect(bridge).toContain("object.insert(");
    expect(bridge).toContain('"run_id".to_string()');
    expect(dialog).toContain('"test_run_cancel_queued"');
    expect(dialog).toContain("取消请求已提交");
    expect(dialog).not.toContain("resume_run_id");
    expect(service).toContain("public string run_id;");
    expect(service).toContain("request.run_id");
    expect(service).toContain("state.cancellation_requested = true;");
    expect(service).toContain("ReconcileCancellation");
    expect(service).toContain("TryCancelActiveRun");
    expect(service).toContain("TryFinishCancellationFromUtfState");
    expect(service).toContain("FinishCancelledLocally");
    expect(service).toContain("CancellationObservationTimeoutSeconds");
    expect(service).toContain("ReportCancellationObservationError");
    expect(service).not.toContain("CancellationAcceptanceTimeoutSeconds");
    expect(service).not.toContain("CancellationSettleTimeoutSeconds");
    expect(liveness).toContain('GetMethod(\n            "IsRunning"');
    expect(liveness).toContain('GetProperty(\n            "m_testJobDataHolder"');
    expect(liveness).toContain('GetField("TestRuns"');
    expect(liveness).toContain('GetField("guid"');
    expect(liveness).toContain('GetField("isRunning"');
    expect(service).toContain(
      "It does not mean that the cancellation API is absent.",
    );
    expect(service).toContain(
      "Unity Test cancellation requires com.unity.test-framework 1.4.0 or newer.",
    );
    expect(service).not.toContain(
      "The installed Unity Test Framework does not expose test cancellation.",
    );
    expect(testingAssembly.versionDefines).toContainEqual({
      name: "com.unity.test-framework",
      expression: "1.4.0",
      define: "LOCUS_HAS_UNITY_TEST_CANCEL",
    });
  });

  it("preserves the Unity suite path for tree output", () => {
    const service = read("locus_unity/Editor/Testing/LocusUnityTestService.cs");
    const formatter = read("src-tauri/src/tool/builtins/unity.rs");

    expect(service).toContain("public string[] path;");
    expect(service).toContain("await CollectModeAsync(");
    expect(service).toMatch(/TestMode\.EditMode,\s+"edit"/);
    expect(service).toMatch(/TestMode\.PlayMode,\s+"play"/);
    expect(service).toContain("mode = modeName");
    expect(service).not.toContain("ModeName(node.TestMode)");
    expect(service).toContain("path = testPath.ToArray()");
    expect(formatter).toContain("render_unity_test_tree");
    expect(formatter).toContain('let branch = if is_last { "└─ " } else { "├─ " };');
    expect(formatter).toContain('format!("{} :: {metadata}", test.label)');
  });

  it("hides empty optional filters from Unity Test tool details", async () => {
    const host = document.createElement("div");
    const app = createApp(ToolCallBlock, {
      toolCall: {
        id: "unity-test-list",
        name: "unity_test_list",
        arguments: JSON.stringify({
          mode: "edit|play",
          assemblies: [],
          tests: [],
          groups: [],
          categories: [],
          max_results: 500,
        }),
        status: "done",
        output: "Unity tests: mode=\"edit|play\" matched=0 shown=0 truncated=false\n└─ <empty>",
      },
    });
    app.use(createPinia());
    app.mount(host);
    host.querySelector<HTMLButtonElement>(".tool-call-header")?.click();
    await nextTick();

    const keys = [...host.querySelectorAll<HTMLElement>(".tool-arg-key")]
      .map((element) => element.textContent);
    expect(keys).toEqual(["mode", "max results"]);
    expect(host.querySelector(".tool-call-pre")?.textContent).toContain("└─ <empty>");

    app.unmount();
  });

  it("exposes a workspace setting with package-aware status", () => {
    const settings = read("src/components/settings/UnityConnectionSettings.vue");
    const service = read("src/services/unity.ts");

    expect(settings).toContain("status.packageInstalled");
    expect(settings).toContain("setUnityTestToolsWorkspaceEnabled");
    expect(service).toContain('"get_unity_test_tools_workspace_status"');
    expect(service).toContain('"set_unity_test_tools_workspace_enabled"');
  });

  it("provides a CLI integration suite that lists and runs through the same host path", () => {
    const driver = read("src-tauri/src/cli_driver.rs");

    expect(driver).toContain("CliDriverSuite::UnityTest");
    expect(driver).toContain("run_unity_test_suite");
    expect(driver).toContain("unity_bridge::unity_test_list");
    expect(driver).toContain("unity_bridge::unity_test_run");
    expect(driver).toContain('let list_request = json!({ "max_results": 50 });');
    expect(driver).toContain('json!({ "mode": "edit|play", "result_detail": "failures" })');
    expect(driver).toContain('list_mode != "edit|play"');
  });
});
