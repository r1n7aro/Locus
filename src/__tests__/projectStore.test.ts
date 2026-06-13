import { beforeEach, describe, expect, it, vi } from "vitest";
import { createPinia, setActivePinia } from "pinia";
import { useProjectStore } from "../stores/project";

const projectServiceMocks = vi.hoisted(() => ({
  getWorkingDir: vi.fn(),
  setWorkingDir: vi.fn(),
  listRecentDirs: vi.fn(),
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

describe("project store asset scan state", () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    vi.clearAllMocks();
    assetServiceMocks.assetDbScanStart.mockResolvedValue({
      started: true,
      alreadyRunning: false,
    });
  });

  it("allows a new scan after switching workspaces while a background scan is running", async () => {
    const store = useProjectStore();

    await store.startScan();
    expect(assetServiceMocks.assetDbScanStart).toHaveBeenCalledTimes(1);

    projectServiceMocks.setWorkingDir.mockResolvedValue("F:/project-b");
    await store.setWorkingDir("F:/project-b");
    await store.startScan();

    expect(assetServiceMocks.assetDbScanStart).toHaveBeenCalledTimes(2);
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

    expect(unityServiceMocks.checkUnityConnectionStatus).toHaveBeenCalledTimes(1);
    resolveStatus(unityConnectionStatus(true));
    await Promise.all([first, second]);
    expect(store.unityConnected).toBe(true);

    unityServiceMocks.checkUnityConnectionStatus.mockResolvedValueOnce(unityConnectionStatus(false));
    await store.checkUnityConnection();

    expect(unityServiceMocks.checkUnityConnectionStatus).toHaveBeenCalledTimes(2);
    expect(store.unityConnected).toBe(false);
  });
});
