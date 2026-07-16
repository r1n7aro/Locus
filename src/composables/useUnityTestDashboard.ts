import {
  computed,
  onMounted,
  onUnmounted,
  ref,
  toValue,
  watch,
  type MaybeRefOrGetter,
} from "vue";
import type { UnlistenFn } from "@tauri-apps/api/event";
import { useNotificationStore } from "../stores/notification";
import { t } from "../i18n";
import type {
  UnityTestMode,
  UnityTestProgress,
  UnityTestResult,
  UnityTestRunRequest,
  UnityTestRunSource,
  UnityTestSnapshot,
} from "../types";
import {
  cancelUnityTestsFromDashboard,
  discoverUnityTests,
  getUnityTestActiveProgress,
  getUnityTestLatestSnapshot,
  listenUnityTestProgress,
  listenUnityTestSnapshotChanged,
  openUnityTestSource,
  runUnityTestsFromDashboard,
} from "../services/unityTest";
import {
  broadUnityTestRunRequest,
  buildUnityTestRunRequest,
  filterUnityTestTree,
  groupUnityTestTreeByMode,
  indexUnityTestDiscovery,
  mapUnityTestResults,
  selectedState,
  unityTestAssemblyKey,
  unityTestFixtureKey,
  unityTestModeKey,
  type UnityTestLeaf,
  type UnityTestStatusFilter,
} from "../utils/unityTestDashboard";

function normalizedPath(path: string): string {
  return path.replace(/\\/g, "/").replace(/\/$/, "").toLocaleLowerCase();
}

function errorMessage(error: unknown): string {
  if (typeof error === "string") return error;
  if (error instanceof Error) return error.message;
  if (error && typeof error === "object" && "message" in error) {
    const message = Reflect.get(error, "message");
    const detail = Reflect.get(error, "detail");
    if (typeof message === "string" && typeof detail === "string" && detail.trim()) {
      return `${message}: ${detail}`;
    }
    if (typeof message === "string") return message;
  }
  return String(error ?? "Unknown Unity test error");
}

export function useUnityTestDashboard(workingDir: MaybeRefOrGetter<string>) {
  const notifications = useNotificationStore();
  const discovery = ref<Awaited<ReturnType<typeof discoverUnityTests>> | null>(null);
  const snapshot = ref<UnityTestSnapshot | null>(null);
  const activeProgress = ref<UnityTestProgress | null>(null);
  const runSource = ref<UnityTestRunSource | null>(null);
  const loading = ref(false);
  const running = ref(false);
  const cancelling = ref(false);
  const error = ref<string | null>(null);
  const search = ref("");
  const modeFilter = ref<UnityTestMode>("all");
  const statusFilter = ref<UnityTestStatusFilter>("all");
  const checkedKeys = ref<Set<string>>(new Set());
  const inspectedKey = ref<string | null>(null);
  const expandedKeys = ref<Set<string>>(new Set());
  const detailTab = ref<"latest" | "detail">("latest");
  let loadGeneration = 0;
  let unlistenProgress: UnlistenFn | null = null;
  let unlistenSnapshot: UnlistenFn | null = null;

  const testIndex = computed(() => indexUnityTestDiscovery(discovery.value));
  const resultByKey = computed(() => mapUnityTestResults(snapshot.value, testIndex.value));
  const filteredTree = computed(() => filterUnityTestTree(
    discovery.value,
    search.value,
    modeFilter.value,
    statusFilter.value,
    resultByKey.value,
  ));
  const filteredModeTree = computed(() => groupUnityTestTreeByMode(filteredTree.value));
  const inspectedTest = computed(() => inspectedKey.value
    ? testIndex.value.get(inspectedKey.value) ?? null
    : null);
  const inspectedResult = computed<UnityTestResult | null>(() => inspectedKey.value
    ? resultByKey.value.get(inspectedKey.value) ?? null
    : null);
  const checkedTests = computed(() => [...checkedKeys.value]
    .map((key) => testIndex.value.get(key))
    .filter((leaf): leaf is UnityTestLeaf => Boolean(leaf)));
  const totalTests = computed(() => testIndex.value.size);
  const isActive = computed(() => running.value || activeProgress.value?.active === true);

  function isCurrentWorkspace(path: string): boolean {
    return normalizedPath(path) === normalizedPath(toValue(workingDir));
  }

  function reset() {
    loadGeneration += 1;
    discovery.value = null;
    snapshot.value = null;
    activeProgress.value = null;
    runSource.value = null;
    error.value = null;
    checkedKeys.value = new Set();
    inspectedKey.value = null;
    expandedKeys.value = new Set();
    detailTab.value = "latest";
  }

  function reconcileSelection() {
    checkedKeys.value = new Set([...checkedKeys.value].filter((key) => testIndex.value.has(key)));
    if (inspectedKey.value && !testIndex.value.has(inspectedKey.value)) {
      inspectedKey.value = null;
      detailTab.value = "latest";
    }
  }

  function expandDiscoveredBranches() {
    const next = new Set(expandedKeys.value);
    for (const assembly of discovery.value?.assemblies ?? []) {
      next.add(unityTestModeKey(assembly.testMode));
      next.add(unityTestAssemblyKey(assembly));
      for (const fixture of assembly.fixtures) {
        next.add(unityTestFixtureKey(assembly.testMode, assembly.name, fixture.name));
      }
    }
    expandedKeys.value = next;
  }

  async function refresh() {
    const project = toValue(workingDir).trim();
    if (!project) {
      reset();
      return;
    }
    const generation = ++loadGeneration;
    loading.value = true;
    error.value = null;
    try {
      const [next, nextSnapshot] = await Promise.all([
        discoverUnityTests({ testMode: "all" }),
        getUnityTestLatestSnapshot(),
      ]);
      if (generation !== loadGeneration) return;
      discovery.value = next;
      snapshot.value = nextSnapshot;
      reconcileSelection();
      expandDiscoveredBranches();
    } catch (nextError) {
      if (generation !== loadGeneration) return;
      error.value = errorMessage(nextError);
    } finally {
      if (generation === loadGeneration) loading.value = false;
    }
  }

  async function loadLatest() {
    const generation = loadGeneration;
    try {
      const next = await getUnityTestLatestSnapshot();
      if (generation !== loadGeneration) return;
      snapshot.value = next;
    } catch (nextError) {
      if (generation === loadGeneration) error.value = errorMessage(nextError);
    }
  }

  async function loadActive() {
    const generation = loadGeneration;
    try {
      const next = await getUnityTestActiveProgress();
      if (generation !== loadGeneration) return;
      activeProgress.value = next?.active ? next : null;
    } catch {
      if (generation === loadGeneration) activeProgress.value = null;
    }
  }

  async function loadWorkspace() {
    reset();
    if (!toValue(workingDir).trim()) return;
    await Promise.all([refresh(), loadActive()]);
  }

  function toggleExpanded(key: string) {
    const next = new Set(expandedKeys.value);
    if (next.has(key)) next.delete(key);
    else next.add(key);
    expandedKeys.value = next;
  }

  function toggleChecked(key: string) {
    const next = new Set(checkedKeys.value);
    if (next.has(key)) next.delete(key);
    else next.add(key);
    checkedKeys.value = next;
  }

  function setBranchChecked(keys: readonly string[], checked: boolean) {
    const next = new Set(checkedKeys.value);
    for (const key of keys) {
      if (checked) next.add(key);
      else next.delete(key);
    }
    checkedKeys.value = next;
  }

  function inspect(key: string) {
    inspectedKey.value = key;
    detailTab.value = "detail";
  }

  async function runRequest(request: UnityTestRunRequest) {
    running.value = true;
    runSource.value = "dashboard";
    error.value = null;
    detailTab.value = "latest";
    try {
      snapshot.value = await runUnityTestsFromDashboard(request);
    } catch (nextError) {
      error.value = errorMessage(nextError);
      await loadLatest();
    } finally {
      running.value = false;
      activeProgress.value = null;
      await loadLatest();
    }
  }

  async function runSelected() {
    if (!checkedTests.value.length) return;
    await runRequest(buildUnityTestRunRequest(checkedTests.value));
  }

  async function runBroad(testMode: UnityTestMode) {
    await runRequest(broadUnityTestRunRequest(testMode));
  }

  async function cancel() {
    cancelling.value = true;
    try {
      await cancelUnityTestsFromDashboard();
    } catch (nextError) {
      notifications.addNotice("error", errorMessage(nextError), { operation: "unityTestCancel" });
    } finally {
      cancelling.value = false;
    }
  }

  async function openSource(leaf: UnityTestLeaf, result?: UnityTestResult | null) {
    const path = result?.sourcePath ?? leaf.test.sourcePath;
    const line = result?.line ?? leaf.test.line;
    if (!path) return;
    try {
      const navigation = await openUnityTestSource(path, line);
      if (line && !navigation.positioned) {
        notifications.addNotice("info", t("unityTest.sourceOpenedWithoutLine"), {
          operation: "unityTestOpenSource",
        });
      }
    } catch (nextError) {
      notifications.addNotice("error", errorMessage(nextError), { operation: "unityTestOpenSource" });
    }
  }

  function branchState(keys: readonly string[]) {
    return selectedState(keys, checkedKeys.value);
  }

  onMounted(async () => {
    [unlistenProgress, unlistenSnapshot] = await Promise.all([
      listenUnityTestProgress((event) => {
        if (!isCurrentWorkspace(event.workingDir)) return;
        activeProgress.value = event.progress.active ? event.progress : null;
        runSource.value = event.source;
      }),
      listenUnityTestSnapshotChanged((event) => {
        if (!isCurrentWorkspace(event.workingDir)) return;
        activeProgress.value = null;
        runSource.value = event.source;
        detailTab.value = "latest";
        void loadLatest();
      }),
    ]);
    await loadWorkspace();
  });

  watch(() => toValue(workingDir), () => {
    void loadWorkspace();
  });

  onUnmounted(() => {
    loadGeneration += 1;
    unlistenProgress?.();
    unlistenSnapshot?.();
  });

  return {
    discovery,
    snapshot,
    activeProgress,
    runSource,
    loading,
    running,
    cancelling,
    error,
    search,
    modeFilter,
    statusFilter,
    checkedKeys,
    inspectedKey,
    expandedKeys,
    detailTab,
    testIndex,
    resultByKey,
    filteredTree,
    filteredModeTree,
    inspectedTest,
    inspectedResult,
    checkedTests,
    totalTests,
    isActive,
    refresh,
    toggleExpanded,
    toggleChecked,
    setBranchChecked,
    inspect,
    runSelected,
    runBroad,
    cancel,
    openSource,
    branchState,
  };
}
