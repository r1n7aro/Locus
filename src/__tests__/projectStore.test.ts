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
});
