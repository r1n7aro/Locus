import { beforeEach, describe, expect, it, vi } from "vitest";
import { createPinia, setActivePinia } from "pinia";
import { useProjectStore } from "../stores/project";
import { useWorkspaceContextStore } from "../stores/workspaceContext";
import { useNotificationStore } from "../stores/notification";

const projectServiceMocks = vi.hoisted(() => ({
  listRecentDirs: vi.fn(),
  startWorkspaceUnityService: vi.fn(),
}));

const unityServiceMocks = vi.hoisted(() => ({
  checkUnityConnection: vi.fn(),
  checkUnityConnectionStatus: vi.fn(),
  checkUnityPlugin: vi.fn(),
  installUnityPlugin: vi.fn(),
  launchUnityProject: vi.fn(),
}));

const assetServiceMocks = vi.hoisted(() => ({
  assetDbLightStatus: vi.fn(),
  assetDbScanStart: vi.fn(),
}));

vi.mock("../services/project", () => projectServiceMocks);
vi.mock("../services/unity", () => unityServiceMocks);
vi.mock("../services/asset", () => assetServiceMocks);

function unityConnectionStatus(connected: boolean) {
  return {
    connected,
    editorStatus: connected ? "editing" : "disconnected",
    controlChannelState: connected ? "ready" : "disconnected",
    editorProcessState: connected ? "running" : "unknown",
    pipeName: "\\\\.\\pipe\\locus_unity_native_test",
    reconnectAttempts: 0,
    backgroundHook: {
      enabled: false,
      supported: true,
      state: "disabled",
      patched: false,
      symbolCount: 0,
      updatedAtMs: 1,
    },
    checkedAtMs: 1,
  };
}

function scanStats(nodesAdded: number) {
  return {
    dirsScanned: 1,
    metaFilesFound: nodesAdded,
    yamlAssetsFound: 0,
    nodesAdded,
    edgesAdded: 0,
    nodesUpdated: 0,
    nodesDeleted: 0,
    parseFailures: 0,
    elapsedMs: 1,
    duplicateGuids: {
      groupCount: 0,
      pathCount: 0,
      assetsOnlyGroups: 0,
      packagesOnlyGroups: 0,
      crossRootGroups: 0,
    },
  };
}

function focusTestWorkspace(
  checkoutId = "checkout-test",
  workspaceGeneration = 7,
  root = "F:/project-test",
  detectedServices: string[] = ["unity"],
) {
  const workspaceContext = useWorkspaceContextStore();
  workspaceContext.checkoutsById[checkoutId] = {
    checkoutId,
    projectId: "project-test",
    root,
    normalizedRoot: root.toLowerCase(),
    lastOpenedAt: 1,
    runtime: {
      projectId: "project-test",
      checkoutId,
      root,
      workspaceGeneration,
      leaseCount: 1,
      detectedServices,
    },
  };
  workspaceContext.paneContexts["main\u0000main"] = {
    windowId: "main",
    paneId: "main",
    focusedCheckoutId: checkoutId,
    workspaceGeneration,
    intentEpoch: workspaceContext.paneContexts["main\u0000main"]?.intentEpoch + 1 || 1,
    revision: workspaceContext.paneContexts["main\u0000main"]?.revision + 1 || 1,
  };
}

describe("project store asset scan state", () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    vi.clearAllMocks();
    focusTestWorkspace();
    projectServiceMocks.startWorkspaceUnityService.mockResolvedValue({
      serviceKind: "unity",
      serviceInstanceId: "unity-test",
      runtimeGeneration: 1,
    });
    assetServiceMocks.assetDbScanStart.mockResolvedValue({
      started: true,
      alreadyRunning: false,
    });
  });

  it("allows a new scan after switching workspaces while a background scan is running", async () => {
    const store = useProjectStore();

    await store.startScan();
    expect(assetServiceMocks.assetDbScanStart).toHaveBeenCalledTimes(1);

    focusTestWorkspace("checkout-b", 8, "F:/project-b");
    await store.startScan();

    expect(assetServiceMocks.assetDbScanStart).toHaveBeenCalledTimes(2);
  });

  it("skips Unity-only workspace services for a generic project", async () => {
    focusTestWorkspace("checkout-generic", 8, "F:/generic-project", []);
    const store = useProjectStore();
    const notifications = useNotificationStore();
    notifications.addNotice("error", "Not a Unity project (Assets/ not found)", {
      operation: "ref_graph_scan_start",
      skipConsoleLog: true,
    });

    await Promise.all([
      store.checkUnityConnection(),
      store.checkUnityPlugin(),
      store.loadAssetDbStatus(),
      store.startScan(),
    ]);

    expect(store.isUnityProject).toBe(false);
    expect(projectServiceMocks.startWorkspaceUnityService).not.toHaveBeenCalled();
    expect(unityServiceMocks.checkUnityConnectionStatus).not.toHaveBeenCalled();
    expect(unityServiceMocks.checkUnityPlugin).not.toHaveBeenCalled();
    expect(assetServiceMocks.assetDbLightStatus).not.toHaveBeenCalled();
    expect(assetServiceMocks.assetDbScanStart).not.toHaveBeenCalled();
    expect(notifications.notices.some((notice) => (
      notice.operation === "ref_graph_scan_start" || notice.operation === "ref_graph_scan"
    ))).toBe(false);
  });

  it("deduplicates concurrent Unity connection checks", async () => {
    const store = useProjectStore();
    let resolveStatus!: (value: ReturnType<typeof unityConnectionStatus>) => void;
    const pendingStatus = new Promise<ReturnType<typeof unityConnectionStatus>>((resolve) => {
      resolveStatus = resolve;
    });
    unityServiceMocks.checkUnityConnectionStatus.mockReturnValueOnce(pendingStatus);

    const first = store.checkUnityConnection();
    const second = store.checkUnityConnection();

    expect(projectServiceMocks.startWorkspaceUnityService).toHaveBeenCalledTimes(1);
    await Promise.resolve();
    expect(unityServiceMocks.checkUnityConnectionStatus).toHaveBeenCalledTimes(1);
    resolveStatus(unityConnectionStatus(true));
    await Promise.all([first, second]);
    expect(store.unityConnected).toBe(true);

    unityServiceMocks.checkUnityConnectionStatus.mockResolvedValueOnce(unityConnectionStatus(false));
    await store.checkUnityConnection();

    expect(unityServiceMocks.checkUnityConnectionStatus).toHaveBeenCalledTimes(2);
    expect(store.unityConnected).toBe(false);
  });

  it("does not project a late Unity status from a previously focused checkout", async () => {
    const store = useProjectStore();
    let resolveCheckoutA!: (value: ReturnType<typeof unityConnectionStatus>) => void;
    const checkoutAStatus = new Promise<ReturnType<typeof unityConnectionStatus>>((resolve) => {
      resolveCheckoutA = resolve;
    });
    unityServiceMocks.checkUnityConnectionStatus.mockImplementation(
      (workspaceRef: { checkoutId: string }) => workspaceRef.checkoutId === "checkout-test"
        ? checkoutAStatus
        : Promise.resolve(unityConnectionStatus(false)),
    );

    const checkoutARequest = store.checkUnityConnection();
    focusTestWorkspace("checkout-b", 9, "F:/project-b");
    await store.checkUnityConnection();
    expect(store.unityConnected).toBe(false);

    resolveCheckoutA(unityConnectionStatus(true));
    await checkoutARequest;

    expect(store.unityConnected).toBe(false);
    expect(unityServiceMocks.checkUnityConnectionStatus).toHaveBeenCalledWith({
      checkoutId: "checkout-test",
      expectedGeneration: 7,
    });
    expect(unityServiceMocks.checkUnityConnectionStatus).toHaveBeenCalledWith({
      checkoutId: "checkout-b",
      expectedGeneration: 9,
    });
  });

  it("does not project a late Asset status from a previously focused checkout", async () => {
    const store = useProjectStore();
    let resolveCheckoutA!: (value: any) => void;
    const checkoutAStatus = new Promise((resolve) => {
      resolveCheckoutA = resolve;
    });
    assetServiceMocks.assetDbLightStatus.mockImplementation(
      (workspaceRef: { checkoutId: string }) => workspaceRef.checkoutId === "checkout-test"
        ? checkoutAStatus
        : Promise.resolve({
            status: "indexed",
            currentScanPhase: null,
            lastScanStats: scanStats(2),
          }),
    );

    const checkoutARequest = store.loadAssetDbStatus();
    focusTestWorkspace("checkout-b", 9, "F:/project-b");
    await store.loadAssetDbStatus();
    expect(store.lastScanStats?.nodesAdded).toBe(2);

    resolveCheckoutA({
      status: "indexed",
      currentScanPhase: { phase: "dirScan" },
      lastScanStats: scanStats(99),
    });
    await checkoutARequest;

    expect(store.lastScanStats?.nodesAdded).toBe(2);
    expect(store.scanPhase).toBeNull();
  });
});
