import { effectScope, nextTick, ref } from "vue";
import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { useWorkspaceUnityStatus } from "../composables/useWorkspaceUnityStatus";
import type { RoutedWorkspaceEvent, WorkspaceRef } from "../services/project";
import type { UnityConnectionStatus } from "../types";

const runtimeMocks = vi.hoisted(() => ({
  subscribe: vi.fn(),
}));

const ipcMocks = vi.hoisted(() => ({
  ipcInvoke: vi.fn(),
}));

vi.mock("@tauri-apps/plugin-dialog", () => ({
  confirm: vi.fn(async () => true),
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

function connectionDetail(connected: boolean): UnityConnectionStatus {
  return {
    connected,
    editorStatus: connected ? "editing" : "disconnected",
    controlChannelState: connected ? "ready" : "disconnected",
    editorProcessState: connected ? "running" : "not_running",
    headless: false,
    pipeName: "test-pipe",
    reconnectAttempts: 0,
    backgroundHook: {
      enabled: false,
      supported: false,
      state: "inactive",
      patched: false,
      symbolCount: 0,
      updatedAtMs: 1,
    },
    checkedAtMs: 1,
  };
}

describe("workbench status icon scoping", () => {
  let workspaceEventHandler: ((event: RoutedWorkspaceEvent) => void) | null;
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
      if (command === "check_unity_connection_status") {
        return {
          checkoutId: "editor-checkout",
          workspaceGeneration: 4,
          connected: false,
          ready: false,
        };
      }
      if (command === "check_unity_plugin") return { status: "upToDate" };
      if (command === "launch_unity_project") {
        return {
          editorPath: "C:/Unity/Editor.exe",
          projectPath: "F:/editor-project",
          projectVersion: "6000.0",
          processId: 42,
          mode: "interactive",
        };
      }
      throw new Error(`Unexpected command: ${command}`);
    });
  });

  it("loads, launches, and listens against the editor checkout", async () => {
    const workspaceRef = ref<WorkspaceRef | null>({
      checkoutId: "editor-checkout",
      expectedGeneration: 4,
    });
    const scope = effectScope();
    const state = scope.run(() => useWorkspaceUnityStatus({ workspaceRef, enabled: true }))!;
    await flushAsyncWork();

    expect(ipcMocks.ipcInvoke).toHaveBeenCalledWith("check_unity_connection_status", {
      workspaceRef: { checkoutId: "editor-checkout", expectedGeneration: 4 },
    });
    expect(ipcMocks.ipcInvoke).toHaveBeenCalledWith("check_unity_plugin", {
      workspaceRef: { checkoutId: "editor-checkout", expectedGeneration: 4 },
    });

    await expect(state.launch()).resolves.toBe(true);
    expect(ipcMocks.ipcInvoke).toHaveBeenCalledWith("launch_unity_project", {
      workspaceRef: { checkoutId: "editor-checkout", expectedGeneration: 4 },
    });
    expect(state.launchState.value).toBe("waitingConnection");
    expect(state.connectionStatus.value?.editorProcessId).toBe(42);

    workspaceEventHandler?.({
      eventName: "unity-connection-status-detail",
      streamRevision: 1,
      projectId: "other-project",
      checkoutId: "other-checkout",
      workspaceGeneration: 4,
      payload: connectionDetail(true),
    });
    expect(state.connected.value).toBe(false);

    workspaceEventHandler?.({
      eventName: "unity-connection-status-detail",
      streamRevision: 2,
      projectId: "editor-project",
      checkoutId: "editor-checkout",
      workspaceGeneration: 4,
      payload: connectionDetail(true),
    });
    expect(state.connected.value).toBe(true);
    expect(state.launchState.value).toBe("idle");

    scope.stop();
    expect(release).toHaveBeenCalledTimes(1);
  });

  it("rebinds Unity actions when an editor moves to another checkout", async () => {
    const workspaceRef = ref<WorkspaceRef | null>({ checkoutId: "checkout-a", expectedGeneration: 1 });
    const scope = effectScope();
    const state = scope.run(() => useWorkspaceUnityStatus({ workspaceRef }))!;
    await flushAsyncWork();

    workspaceRef.value = { checkoutId: "checkout-b", expectedGeneration: 2 };
    await flushAsyncWork();
    await state.launch();

    expect(release).toHaveBeenCalledTimes(1);
    expect(ipcMocks.ipcInvoke).toHaveBeenLastCalledWith("launch_unity_project", {
      workspaceRef: { checkoutId: "checkout-b", expectedGeneration: 2 },
    });

    scope.stop();
    expect(release).toHaveBeenCalledTimes(2);
  });

  it("keeps every checkout-level icon path off the global project store", () => {
    const read = (path: string) => readFileSync(resolve(process.cwd(), path), "utf8");
    const editor = read("src/components/workbench/WorkbenchSessionEditor.vue");
    const indicators = read("src/components/chat/ChatStatusIndicators.vue");

    expect(editor).toContain("useWorkspaceAssetDbStatus");
    expect(editor).toContain("useWorkspaceUnityStatus");
    expect(editor).toContain('@start-scan="startWorkspaceAssetScan"');
    expect(editor).toContain('@install-plugin="installWorkspaceUnityPlugin"');
    expect(editor).toContain('@launch-unity-project="launchWorkspaceUnity"');
    expect(editor).not.toContain("useProjectStore");
    expect(editor).not.toContain("projectStore.");

    expect(indicators).toContain("unitySemanticStateGet(workspaceRef)");
    expect(indicators).toContain("csharpLspGetStatus(scope)");
    expect(indicators).toContain("csharpLspRestart(scope)");
    expect(indicators).toContain("unitySidecarCompilerGetStatus(scope)");
    expect(indicators).toContain("unityRecompileRun(scope)");
    expect(indicators).toContain("knowledgeGetOverview(scope)");
    expect(indicators).toContain("listWorkspaceAgentInjectedItems(scope, agentId, knowledgeMode.value)");

    // MCP servers and the knowledge access preference are application-level by design.
    expect(indicators).toContain("mcpGetStatus()");
    expect(indicators).toContain("mcpServerSetEnabled(server.id, next)");
  });
});
