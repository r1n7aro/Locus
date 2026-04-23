import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createPinia, setActivePinia } from "pinia";

vi.mock("../services/appUpdate", async () => {
  const actual = await vi.importActual<typeof import("../services/appUpdate")>(
    "../services/appUpdate",
  );
  return {
    ...actual,
    fetchAppUpdateManifest: vi.fn(),
  };
});
vi.mock("../services/appVersion", () => ({
  getAppRuntimeVersion: vi.fn(),
}));

import * as appUpdateService from "../services/appUpdate";
import * as appVersionService from "../services/appVersion";
import { useAppUpdateStore } from "../stores/appUpdate";

function createLocalStorageMock() {
  let storage = new Map<string, string>();
  return {
    getItem: vi.fn((key: string) => storage.get(key) ?? null),
    setItem: vi.fn((key: string, value: string) => {
      storage.set(key, String(value));
    }),
    removeItem: vi.fn((key: string) => {
      storage.delete(key);
    }),
    clear: vi.fn(() => {
      storage = new Map<string, string>();
    }),
  };
}

describe("app update store", () => {
  let localStorageMock: ReturnType<typeof createLocalStorageMock>;
  const fetchAppUpdateManifestMock = vi.mocked(appUpdateService.fetchAppUpdateManifest);
  const getAppRuntimeVersionMock = vi.mocked(appVersionService.getAppRuntimeVersion);

  beforeEach(() => {
    setActivePinia(createPinia());
    vi.clearAllMocks();
    localStorageMock = createLocalStorageMock();
    vi.stubGlobal("localStorage", localStorageMock as unknown as Storage);
    localStorageMock.clear();
    getAppRuntimeVersionMock.mockResolvedValue("0.1.0");
  });

  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it("stores the last checked timestamp and resolves update info on a successful check", async () => {
    fetchAppUpdateManifestMock.mockResolvedValue({
      manifest: {
        version: "0.2.0",
        releasedAt: "2026-04-23",
        channel: "stable",
        locales: {
          zh: {
            title: "发现新版本",
            summary: "Locus 0.2.0 已发布。",
            changelogUrl: "/overview/latest-version",
            changes: [
              {
                title: "新增",
                items: ["新增手动检查更新"],
              },
            ],
          },
        },
      },
      sourceKind: "local",
      sourceBaseUrl: "http://localhost:3002",
    });

    const store = useAppUpdateStore();
    const info = await store.checkForUpdates({ silent: true });

    expect(store.currentVersion).toBe("0.1.0");
    expect(store.lastCheckedAt).not.toBeNull();
    expect(localStorageMock.setItem).toHaveBeenCalledWith(
      "locus-app-update-last-checked-at",
      expect.any(String),
    );
    expect(info?.latestVersion).toBe("0.2.0");
    expect(store.hasUpdate).toBe(true);
    expect(store.sourceKind).toBe("local");
    expect(store.sourceBaseUrl).toBe("http://localhost:3002");
    expect(store.sourceLabel).toBe("本地服务器 (localhost:3002)");
  });

  it("records the last checked timestamp even when the request fails", async () => {
    fetchAppUpdateManifestMock.mockRejectedValue(new Error("network failed"));

    const store = useAppUpdateStore();
    const info = await store.checkForUpdates({ silent: true });

    expect(info).toBeNull();
    expect(store.lastCheckedAt).not.toBeNull();
    expect(store.lastError).toBe("network failed");
  });

  it("does not reopen the update dialog for build-metadata-only differences", async () => {
    getAppRuntimeVersionMock.mockResolvedValue("0.2.0+build.9");
    fetchAppUpdateManifestMock.mockResolvedValue({
      manifest: {
        version: "0.2.0",
        releasedAt: "2026-04-23",
        channel: "stable",
        locales: {
          zh: {
            title: "发现新版本",
            summary: "Locus 0.2.0 已发布。",
            changelogUrl: "/overview/latest-version",
            changes: [
              {
                title: "新增",
                items: ["新增手动检查更新"],
              },
            ],
          },
        },
      },
      sourceKind: "remote",
      sourceBaseUrl: "https://unity.farlocus.com",
    });

    const store = useAppUpdateStore();
    const info = await store.checkForUpdates({ silent: true });

    expect(info).toBeNull();
    expect(store.hasUpdate).toBe(false);
  });
});
