import { effectScope, nextTick, ref } from "vue";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { useWorkspaceAssetDbStatus } from "../composables/useWorkspaceAssetDbStatus";
import type { RoutedWorkspaceEvent, WorkspaceRef } from "../services/project";
import type { AssetDbScanEvent } from "../types";

const runtimeMocks = vi.hoisted(() => ({
  subscribe: vi.fn(),
}));

const ipcMocks = vi.hoisted(() => ({
  ipcInvoke: vi.fn(),
}));

vi.mock("../services/locusRuntime", () => ({
  getLocusRuntime: () => ({
    kind: "tauri",
    invoke: vi.fn(),
    subscribe: runtimeMocks.subscribe,
  }),
}));

vi.mock("../services/ipc", () => ({
  ipcInvoke: ipcMocks.ipcInvoke,
}));

async function flushAsyncWork() {
  await nextTick();
  await Promise.resolve();
  await Promise.resolve();
}

function lightStatus(status: "indexed" | "scanning" | "none" = "none") {
  return {
    status,
    nodes: status === "indexed" ? 12 : 0,
    edges: status === "indexed" ? 7 : 0,
  };
}

describe("workspace-scoped asset database status", () => {
  let workspaceEventHandler: ((event: RoutedWorkspaceEvent<AssetDbScanEvent>) => void) | null;
  let release: ReturnType<typeof vi.fn>;

  beforeEach(() => {
    vi.clearAllMocks();
    workspaceEventHandler = null;
    release = vi.fn();
    runtimeMocks.subscribe.mockImplementation(async (_eventName, handler) => {
      workspaceEventHandler = handler;
      return release;
    });
    ipcMocks.ipcInvoke.mockImplementation(async (command: string) => {
      if (command === "asset_db_light_status") return lightStatus();
      if (command === "ref_graph_scan_start") {
        return { started: true, alreadyRunning: false };
      }
      throw new Error(`Unexpected command: ${command}`);
    });
  });

  it("starts and tracks a scan for the editor workspace instead of global focus", async () => {
    const workspaceRef = ref<WorkspaceRef | null>({
      checkoutId: "editor-checkout",
      expectedGeneration: 9,
    });
    const scanErrors: string[] = [];
    const scope = effectScope();
    const state = scope.run(() => useWorkspaceAssetDbStatus({
      workspaceRef,
      enabled: true,
      onScanError: (error) => scanErrors.push(error.message),
    }))!;
    await flushAsyncWork();

    expect(ipcMocks.ipcInvoke).toHaveBeenCalledWith("asset_db_light_status", {
      workspaceRef: { checkoutId: "editor-checkout", expectedGeneration: 9 },
    });

    await expect(state.startScan()).resolves.toBe(true);
    expect(ipcMocks.ipcInvoke).toHaveBeenCalledWith("ref_graph_scan_start", {
      workspaceRef: { checkoutId: "editor-checkout", expectedGeneration: 9 },
    });
    expect(state.scanPhase.value).toEqual({ phase: "dirScan" });

    workspaceEventHandler?.({
      eventName: "ref-graph-scan",
      streamRevision: 1,
      projectId: "project-test",
      checkoutId: "other-checkout",
      workspaceGeneration: 9,
      payload: { phase: "error", error: { code: "wrong", message: "wrong workspace", severity: "error", retryable: false } },
    });
    expect(state.scanPhase.value).toEqual({ phase: "dirScan" });

    workspaceEventHandler?.({
      eventName: "ref-graph-scan",
      streamRevision: 2,
      projectId: "project-test",
      checkoutId: "editor-checkout",
      workspaceGeneration: 9,
      payload: {
        phase: "done",
        stats: {
          dirsScanned: 3,
          metaFilesFound: 12,
          yamlAssetsFound: 6,
          nodesAdded: 12,
          edgesAdded: 7,
          nodesUpdated: 0,
          nodesDeleted: 0,
          parseFailures: 0,
          elapsedMs: 25,
          duplicateGuids: {
            groupCount: 0,
            pathCount: 0,
            assetsOnlyGroups: 0,
            packagesOnlyGroups: 0,
            crossRootGroups: 0,
          },
        },
      },
    });
    expect(state.lastScanStats.value?.nodesAdded).toBe(12);
    expect(state.scanPhase.value?.phase).toBe("done");
    expect(scanErrors).toEqual([]);

    scope.stop();
    expect(release).toHaveBeenCalledTimes(1);
  });

  it("rebinds status and scan actions when the editor checkout changes", async () => {
    const workspaceRef = ref<WorkspaceRef | null>({ checkoutId: "checkout-a", expectedGeneration: 1 });
    const scope = effectScope();
    const state = scope.run(() => useWorkspaceAssetDbStatus({ workspaceRef }))!;
    await flushAsyncWork();

    workspaceRef.value = { checkoutId: "checkout-b", expectedGeneration: 2 };
    await flushAsyncWork();
    expect(release).toHaveBeenCalledTimes(1);

    await state.startScan();
    expect(ipcMocks.ipcInvoke).toHaveBeenLastCalledWith("ref_graph_scan_start", {
      workspaceRef: { checkoutId: "checkout-b", expectedGeneration: 2 },
    });

    scope.stop();
    expect(release).toHaveBeenCalledTimes(2);
  });
});
