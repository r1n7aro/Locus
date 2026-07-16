import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import type { UnityTestDiscovery, UnityTestSnapshot } from "../types";
import {
  buildUnityTestRunRequest,
  filterUnityTestTree,
  groupUnityTestTreeByMode,
  indexUnityTestDiscovery,
  mapUnityTestResults,
  selectedState,
  unityTestKey,
} from "../utils/unityTestDashboard";

const discovery: UnityTestDiscovery = {
  assemblies: [
    {
      name: "Game.EditMode.Tests",
      testMode: "editmode",
      fixtures: [{
        name: "Game.Tests.InventoryTests",
        tests: [
          { name: "AddsItem(1)", fullName: "Game.Tests.InventoryTests.AddsItem(1)", attributes: [], sourcePath: "Assets/Tests/InventoryTests.cs", line: 12 },
          { name: "AddsItem(2)", fullName: "Game.Tests.InventoryTests.AddsItem(2)", attributes: [], sourcePath: "Assets/Tests/InventoryTests.cs", line: 20 },
        ],
      }],
    },
    {
      name: "Game.PlayMode.Tests",
      testMode: "playmode",
      fixtures: [{
        name: "Game.Tests.InventoryTests",
        tests: [{ name: "AddsItem(1)", fullName: "Game.Tests.InventoryTests.AddsItem(1)", attributes: [] }],
      }],
    },
  ],
};

function snapshot(): UnityTestSnapshot {
  const result = {
    assemblyName: "Game.EditMode.Tests",
    fixtureName: "Game.Tests.InventoryTests",
    testName: "AddsItem(1)",
    fullName: "Game.Tests.InventoryTests.AddsItem(1)",
    outcome: "failed",
    duration: 0.25,
    message: "Expected one item",
    stackTrace: "at InventoryTests.AddsItem",
  };
  return {
    runId: "run-1",
    startedAt: "2026-07-10T00:00:00Z",
    finishedAt: "2026-07-10T00:00:01Z",
    terminalStatus: "completed_failed",
    preparation: { method: "hot_reload", status: "ok" },
    requestedScope: { testMode: "editmode" },
    phaseSummaries: [{
      runId: "run-1",
      testMode: "editmode",
      status: "failed",
      total: 1,
      passed: 0,
      failed: 1,
      skipped: 0,
      duration: 0.25,
      results: [result],
    }],
    totalSummary: { total: 1, passed: 0, failed: 1, skipped: 0, duration: 0.25 },
    results: [result],
  };
}

describe("Unity test dashboard projections", () => {
  it("uses mode, assembly, fixture, and full name in stable keys", () => {
    const edit = unityTestKey("editmode", "Asm", "Fixture", "Fixture.Test(1)");
    const play = unityTestKey("playmode", "Asm", "Fixture", "Fixture.Test(1)");
    const parameter = unityTestKey("editmode", "Asm", "Fixture", "Fixture.Test(2)");
    expect(new Set([edit, play, parameter])).toHaveLength(3);
  });

  it("indexes parameterized and same-named tests without collisions", () => {
    const index = indexUnityTestDiscovery(discovery);
    expect(index.size).toBe(3);
  });

  it("keeps test modes as stable top-level nodes above real assemblies", () => {
    const modes = groupUnityTestTreeByMode(filterUnityTestTree(discovery, "", "all", "all", new Map()));

    expect(modes.map((mode) => mode.name)).toEqual(["EditMode", "PlayMode"]);
    expect(modes[0].assemblies.map((assembly) => assembly.name)).toEqual(["Game.EditMode.Tests"]);
    expect(modes[1].assemblies.map((assembly) => assembly.name)).toEqual(["Game.PlayMode.Tests"]);
  });

  it("maps results through phase mode and combines filters with AND", () => {
    const results = mapUnityTestResults(snapshot(), indexUnityTestDiscovery(discovery));
    const failedEdit = filterUnityTestTree(discovery, "Inventory", "editmode", "failed", results);
    expect(failedEdit).toHaveLength(1);
    expect(failedEdit[0].fixtures[0].tests.map((leaf) => leaf.test.name)).toEqual(["AddsItem(1)"]);
    expect(filterUnityTestTree(discovery, "PlayMode", "editmode", "failed", results)).toEqual([]);
  });

  it("safely maps legacy results whose assembly is only the mode label", () => {
    const legacy = snapshot();
    legacy.phaseSummaries[0].results[0].assemblyName = "EditMode";
    legacy.results[0].assemblyName = "EditMode";
    const index = indexUnityTestDiscovery(discovery);
    const results = mapUnityTestResults(legacy, index);

    expect(results.size).toBe(1);
    expect(results.get([...index.keys()][0])?.outcome).toBe("failed");
  });

  it("does not treat Unity's empty root suite as a completed test", () => {
    const emptyRoot = snapshot();
    const rootResult = emptyRoot.phaseSummaries[0].results[0];
    rootResult.assemblyName = "EditMode";
    rootResult.fixtureName = "";
    rootResult.testName = "a_bite_of_the_solar_system";
    rootResult.fullName = "a_bite_of_the_solar_system";
    rootResult.outcome = "passed";
    emptyRoot.results = [rootResult];

    expect(mapUnityTestResults(emptyRoot, indexUnityTestDiscovery(discovery))).toEqual(new Map());
  });

  it("maps cumulative results while the phase summary contains only the current run", () => {
    const cumulative = snapshot();
    cumulative.results.push({
      testMode: "editmode",
      assemblyName: "Game.EditMode.Tests",
      fixtureName: "Game.Tests.InventoryTests",
      testName: "AddsItem(2)",
      fullName: "Game.Tests.InventoryTests.AddsItem(2)",
      outcome: "passed",
      duration: 0.1,
      message: "",
      stackTrace: "",
    });
    const results = mapUnityTestResults(cumulative, indexUnityTestDiscovery(discovery));

    expect(results.size).toBe(2);
    expect([...results.values()].map((result) => result.outcome).sort()).toEqual(["failed", "passed"]);
    expect(cumulative.phaseSummaries[0].results).toHaveLength(1);
  });

  it("keeps checked state independent from visible filtering", () => {
    const keys = [...indexUnityTestDiscovery(discovery).keys()];
    const checked = new Set(keys.slice(0, 2));
    expect(selectedState(keys, checked)).toBe("some");
    expect(filterUnityTestTree(discovery, "no match", "all", "all", new Map())).toEqual([]);
    expect(checked.size).toBe(2);
  });

  it("builds exact requests and narrows homogeneous modes", () => {
    const leaves = [...indexUnityTestDiscovery(discovery).values()];
    const editRequest = buildUnityTestRunRequest(leaves.filter((leaf) => leaf.testMode === "editmode"));
    expect(editRequest.testMode).toBe("editmode");
    expect(editRequest.tests).toHaveLength(2);
    expect(editRequest.tests?.[0].testName).toBe("Game.Tests.InventoryTests.AddsItem(1)");
    expect(buildUnityTestRunRequest(leaves).testMode).toBe("all");
  });
});

describe("Unity test dashboard registration", () => {
  it("registers commands, events, lazy tab, and bilingual labels", () => {
    const read = (path: string) => readFileSync(path, "utf8");
    const lib = read("src-tauri/src/lib.rs");
    const command = read("src-tauri/src/commands/unity_test.rs");
    const app = read("src/App.vue");
    const en = read("src/language/en.json");
    const zh = read("src/language/zh.json");
    for (const name of [
      "unity_test_discover",
      "unity_test_run_dashboard",
      "unity_test_cancel_dashboard",
      "unity_test_active_progress",
      "unity_test_open_source",
    ]) expect(lib).toContain(`commands::${name}`);
    expect(command).toContain('"unity-test-progress"');
    expect(command).toContain('"unity-test-snapshot-changed"');
    expect(app).toContain('import("./components/UnityTestDashboardView.vue")');
    expect(app).toContain('id: "tests"');
    expect(en).toContain('"app.tab.tests": "Tests"');
    expect(zh).toContain('"app.tab.tests": "测试"');
  });

  it("extracts authoritative leaf results and real Unity assembly names", () => {
    const bridge = readFileSync("locus_unity/Editor/LocusBridge.TestRunner.cs", "utf8");
    expect(bridge).toContain("EnumerateLeafResults(result)");
    expect(bridge).toContain("ResolveAssemblyName(test, info.AssemblyName)");
    expect(bridge).toContain('errorCode = noTestsExecuted ? "no_tests_executed" : ""');
  });

  it("localizes terminal states and exposes a resizable polished result layout", () => {
    const component = readFileSync("src/components/UnityTestDashboardView.vue", "utf8");
    const zh = readFileSync("src/language/zh.json", "utf8");
    expect(component).toContain("terminalStatusLabel");
    expect(component).toContain('class="pane-resize-handle"');
    expect(component).toContain('class="running-card"');
    expect(component).toContain('class="result-item-header"');
    expect(component).toContain(".summary-grid .passed strong { color:");
    expect(component).toContain(".outcome-pill { flex: none;");
    expect(zh).toContain('"unityTest.terminal.completedFailed": "运行完成，有测试失败"');
  });
});
