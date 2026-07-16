import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const read = (path: string) => readFileSync(resolve(process.cwd(), path), "utf8");

describe("Unity Test Framework core loop wiring", () => {
  it("registers the agent tools and prompts", () => {
    const promptSource = read("src-tauri/src/prompt.rs");
    const builtinsSource = read("src-tauri/src/tool/builtins/mod.rs");
    const agentSource = read("src-tauri/src/agent/instance/mod.rs");
    const devAgent = JSON.parse(read("agent/dev/config.json"));
    const runtimeDebuggerAgent = JSON.parse(read("agent/runtime_debugger/config.json"));

    expect(promptSource).toContain("UNITY_TEST_FIND");
    expect(promptSource).toContain("UNITY_TEST_RUN");
    expect(promptSource).toContain('("unity_test_find", tools::UNITY_TEST_FIND)');
    expect(promptSource).toContain('("unity_test_run", tools::UNITY_TEST_RUN)');
    expect(builtinsSource).toContain("unity::unity_test_find()");
    expect(builtinsSource).toContain("unity::unity_test_run()");
    expect(agentSource).toContain('tc.name == "unity_test_run"');
    expect(agentSource).toContain("execute_unity_test_run");
    expect(devAgent.tools).toContain("unity_test_find");
    expect(devAgent.tools).toContain("unity_test_run");
    expect(runtimeDebuggerAgent.tools).toContain("unity_test_find");
    expect(runtimeDebuggerAgent.tools).toContain("unity_test_run");
  });

  it("keeps run results visible through the frontend override and latest snapshot command", () => {
    expect(read("src/components/tool-block-overrides/toolBlockOverrides.ts")).toContain(
      "unity_test_run: UnityTestToolBlock",
    );
    expect(read("src-tauri/src/commands/mod.rs")).toContain("mod unity_test");
    expect(read("src-tauri/src/lib.rs")).toContain("commands::unity_test_latest_snapshot");
    expect(read("src/services/unityTest.ts")).toContain("unity_test_latest_snapshot");
    expect(read("src/types.ts")).toContain("export interface UnityTestSnapshot");
  });

  it("defaults test runs to confirmation because they write latest snapshot state", () => {
    const settingsSource = read("src/composables/useSettingsState.ts");
    const configRegistrySource = read("src-tauri/src/config_registry.rs");
    const runTool = JSON.parse(read("tools/unity_test_run.json"));

    expect(settingsSource).toContain('name: "unity_test_run"');
    expect(settingsSource).toMatch(/name:\s*"unity_test_run"[\s\S]*defaultMode:\s*"ask"\s+as const/);
    expect(configRegistrySource).toContain('("unity_test_find", "Find Unity Test Framework tests")');
    expect(configRegistrySource).toContain('("unity_test_run", "Run Unity Test Framework tests")');
    expect(runTool.description).toContain("Run Unity Test Framework tests");
    expect(runTool.parameters.properties.search.description).toContain("resolve search by discovery");
  });

  it("does not require Test Framework assemblies at compile time", () => {
    const asmdef = JSON.parse(read("locus_unity/Editor/Locus.Editor.asmdef"));
    const testRunnerSource = read("locus_unity/Editor/LocusBridge.TestRunner.cs");

    expect(asmdef.references).not.toContain("UnityEditor.TestRunner");
    expect(asmdef.references).not.toContain("UnityEngine.TestRunner");
    expect(testRunnerSource).not.toContain("using UnityEditor.TestTools.TestRunner.Api");
    expect(testRunnerSource).toContain("test_framework_missing");
    expect(testRunnerSource).toContain("DispatchProxy");
  });
});
