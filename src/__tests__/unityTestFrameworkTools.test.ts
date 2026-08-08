import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

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
  });
});
