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
    expect(read("agent/dev/rule/tool_usage_strategy.md")).toContain(
      "UnityTestApi.ListAsync",
    );
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
        expression: "1.1.0",
        define: "LOCUS_HAS_UNITY_TEST_FRAMEWORK",
      }),
    );
    expect(bridge).toContain("RegisterExtensionMessageHandler");
  });

  it("requires workspace opt-in and the installed package across agent and MCP surfaces", () => {
    const workspace = read("src-tauri/src/workspace.rs");
    const agent = read("src-tauri/src/agent/instance/mod.rs");
    const mcp = read("src-tauri/src/mcp/server/tools.rs");

    expect(workspace).toContain("enabled && package_installed");
    expect(workspace).toContain('contains_key("com.unity.test-framework")');
    expect(agent).toContain('"unity_test_list" | "unity_test_run"');
    expect(agent).toContain("unity_test_tools_available(&self.working_dir)");
    expect(mcp).toContain("unity_test_tools_workspace_status(working_dir)");
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
    expect(listDefinition.description).toContain("call unity_recompile before listing");
    expect(runDefinition.description).toContain("call unity_recompile before running");
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
      expect(definition.description).toContain("do not send empty arrays");
    }
    expect(service).toContain("value.Split('|')");
    expect(service).toContain("TestMode.EditMode | TestMode.PlayMode");
    expect(service).toContain('return "edit|play";');
    expect(api).toContain("[Flags]");
    expect(api).toContain("EditAndPlay = Edit | Play");
    expect(api).toContain("UnityTestMode Mode = UnityTestMode.EditAndPlay");
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
