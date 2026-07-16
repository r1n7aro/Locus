import type {
  UnityTestAssembly,
  UnityTestDiscovery,
  UnityTestMethod,
  UnityTestMode,
  UnityTestResult,
  UnityTestRunRequest,
  UnityTestSnapshot,
} from "../types";

export type UnityTestStatusFilter = "all" | "failed" | "skipped" | "not_run";

export interface UnityTestLeaf {
  key: string;
  testMode: Exclude<UnityTestMode, "all">;
  assemblyName: string;
  fixtureName: string;
  test: UnityTestMethod;
}

export interface UnityTestFixtureView {
  key: string;
  name: string;
  tests: UnityTestLeaf[];
}

export interface UnityTestAssemblyView {
  key: string;
  name: string;
  testMode: Exclude<UnityTestMode, "all">;
  fixtures: UnityTestFixtureView[];
}

export interface UnityTestModeView {
  key: string;
  name: "EditMode" | "PlayMode";
  testMode: Exclude<UnityTestMode, "all">;
  assemblies: UnityTestAssemblyView[];
}

export type SelectionState = "none" | "some" | "all";

function normalizedMode(mode: string): Exclude<UnityTestMode, "all"> {
  return mode.toLowerCase() === "playmode" ? "playmode" : "editmode";
}

export function unityTestKey(
  testMode: string,
  assemblyName: string,
  fixtureName: string,
  fullName: string,
  testName = "",
): string {
  return JSON.stringify([
    normalizedMode(testMode),
    assemblyName.trim(),
    fixtureName.trim(),
    (fullName.trim() || testName.trim()),
  ]);
}

export function unityTestAssemblyKey(assembly: Pick<UnityTestAssembly, "name" | "testMode">): string {
  return JSON.stringify([normalizedMode(assembly.testMode), assembly.name]);
}

export function unityTestModeKey(testMode: string): string {
  return JSON.stringify([normalizedMode(testMode)]);
}

export function unityTestFixtureKey(
  testMode: string,
  assemblyName: string,
  fixtureName: string,
): string {
  return JSON.stringify([normalizedMode(testMode), assemblyName, fixtureName]);
}

export function indexUnityTestDiscovery(discovery: UnityTestDiscovery | null): Map<string, UnityTestLeaf> {
  const index = new Map<string, UnityTestLeaf>();
  for (const assembly of discovery?.assemblies ?? []) {
    const testMode = normalizedMode(assembly.testMode);
    for (const fixture of assembly.fixtures) {
      for (const test of fixture.tests) {
        const key = unityTestKey(testMode, assembly.name, fixture.name, test.fullName, test.name);
        index.set(key, {
          key,
          testMode,
          assemblyName: assembly.name,
          fixtureName: fixture.name,
          test,
        });
      }
    }
  }
  return index;
}

function resultKey(testMode: string, result: UnityTestResult): string | null {
  if (!result.assemblyName.trim() || !result.fixtureName.trim()) return null;
  if (!result.fullName.trim() && !result.testName.trim()) return null;
  return unityTestKey(
    testMode,
    result.assemblyName,
    result.fixtureName,
    result.fullName,
    result.testName,
  );
}

function matchedDiscoveryKey(
  testMode: string,
  result: UnityTestResult,
  testIndex?: ReadonlyMap<string, UnityTestLeaf>,
): string | null {
  const normalizedRequestedMode = testMode.trim().toLowerCase();
  const hasSpecificMode = normalizedRequestedMode === "editmode" || normalizedRequestedMode === "playmode";
  const direct = hasSpecificMode ? resultKey(testMode, result) : null;
  if (!testIndex) return direct;
  if (direct && testIndex.has(direct)) return direct;

  const normalizedFixture = result.fixtureName.trim().toLocaleLowerCase();
  const normalizedFullName = result.fullName.trim().toLocaleLowerCase();
  const normalizedTestName = result.testName.trim().toLocaleLowerCase();
  const candidates = [...testIndex.values()].filter((leaf) => {
    if (hasSpecificMode && leaf.testMode !== normalizedMode(testMode)) return false;
    if (normalizedFixture && leaf.fixtureName.toLocaleLowerCase() !== normalizedFixture) return false;
    if (normalizedFullName) {
      return leaf.test.fullName.trim().toLocaleLowerCase() === normalizedFullName;
    }
    return Boolean(normalizedTestName)
      && leaf.test.name.trim().toLocaleLowerCase() === normalizedTestName;
  });
  return candidates.length === 1 ? candidates[0].key : null;
}

export function mapUnityTestResults(
  snapshot: UnityTestSnapshot | null,
  testIndex?: ReadonlyMap<string, UnityTestLeaf>,
): Map<string, UnityTestResult> {
  const results = new Map<string, UnityTestResult>();
  if (!snapshot) return results;
  for (const result of snapshot.results) {
    const key = matchedDiscoveryKey(result.testMode ?? snapshot.requestedScope.testMode, result, testIndex);
    if (key) results.set(key, result);
  }
  for (const phase of snapshot.phaseSummaries) {
    for (const result of phase.results) {
      const key = matchedDiscoveryKey(phase.testMode, result, testIndex);
      if (key) results.set(key, result);
    }
  }
  return results;
}

function matchesStatus(
  leaf: UnityTestLeaf,
  status: UnityTestStatusFilter,
  resultByKey: ReadonlyMap<string, UnityTestResult>,
): boolean {
  if (status === "all") return true;
  const result = resultByKey.get(leaf.key);
  if (status === "not_run") return !result;
  return result?.outcome.toLowerCase() === status;
}

function matchesSearch(leaf: UnityTestLeaf, rawSearch: string): boolean {
  const search = rawSearch.trim().toLocaleLowerCase();
  if (!search) return true;
  return [
    leaf.assemblyName,
    leaf.fixtureName,
    leaf.test.name,
    leaf.test.fullName,
    leaf.test.sourcePath ?? "",
  ].some((value) => value.toLocaleLowerCase().includes(search));
}

export function filterUnityTestTree(
  discovery: UnityTestDiscovery | null,
  search: string,
  mode: UnityTestMode,
  status: UnityTestStatusFilter,
  resultByKey: ReadonlyMap<string, UnityTestResult>,
): UnityTestAssemblyView[] {
  const tree: UnityTestAssemblyView[] = [];
  for (const assembly of discovery?.assemblies ?? []) {
    const testMode = normalizedMode(assembly.testMode);
    if (mode !== "all" && mode !== testMode) continue;
    const fixtures: UnityTestFixtureView[] = [];
    for (const fixture of assembly.fixtures) {
      const tests = fixture.tests
        .map((test): UnityTestLeaf => {
          const key = unityTestKey(testMode, assembly.name, fixture.name, test.fullName, test.name);
          return { key, testMode, assemblyName: assembly.name, fixtureName: fixture.name, test };
        })
        .filter((leaf) => matchesSearch(leaf, search) && matchesStatus(leaf, status, resultByKey));
      if (tests.length) {
        fixtures.push({
          key: unityTestFixtureKey(testMode, assembly.name, fixture.name),
          name: fixture.name,
          tests,
        });
      }
    }
    if (fixtures.length) {
      tree.push({
        key: unityTestAssemblyKey(assembly),
        name: assembly.name,
        testMode,
        fixtures,
      });
    }
  }
  return tree;
}

export function groupUnityTestTreeByMode(
  assemblies: readonly UnityTestAssemblyView[],
): UnityTestModeView[] {
  return (["editmode", "playmode"] as const).flatMap((testMode) => {
    const matching = assemblies.filter((assembly) => assembly.testMode === testMode);
    if (!matching.length) return [];
    return [{
      key: unityTestModeKey(testMode),
      name: testMode === "editmode" ? "EditMode" : "PlayMode",
      testMode,
      assemblies: matching,
    }];
  });
}

export function selectedState(keys: readonly string[], checked: ReadonlySet<string>): SelectionState {
  const selected = keys.reduce((count, key) => count + (checked.has(key) ? 1 : 0), 0);
  if (selected === 0) return "none";
  return selected === keys.length ? "all" : "some";
}

export function buildUnityTestRunRequest(leaves: readonly UnityTestLeaf[]): UnityTestRunRequest {
  const modes = new Set(leaves.map((leaf) => leaf.testMode));
  const testMode: UnityTestMode = modes.size === 1 ? leaves[0]?.testMode ?? "all" : "all";
  return {
    testMode,
    tests: leaves.map((leaf) => ({
      assemblyName: leaf.assemblyName,
      fixtureName: leaf.fixtureName,
      testName: leaf.test.fullName || leaf.test.name,
    })),
  };
}

export function broadUnityTestRunRequest(testMode: UnityTestMode): UnityTestRunRequest {
  return { testMode };
}
