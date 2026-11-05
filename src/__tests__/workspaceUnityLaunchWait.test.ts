import { effectScope, nextTick, ref } from "vue";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { RoutedWorkspaceEvent, WorkspaceRef } from "../services/project";

const mocks = vi.hoisted(() => ({ subscribe: vi.fn(), ipcInvoke: vi.fn() }));
vi.mock("../services/locusRuntime", () => ({
  getLocusRuntime: () => ({ kind: "tauri", invoke: vi.fn(), subscribe: mocks.subscribe }),
}));
vi.mock("../services/ipc", () => ({ ipcInvoke: mocks.ipcInvoke }));
vi.mock("@tauri-apps/plugin-dialog", () => ({ confirm: vi.fn() }));

import { useWorkspaceUnityStatus } from "../composables/useWorkspaceUnityStatus";

const workspaceRef: WorkspaceRef = { checkoutId: "launch-a", expectedGeneration: 1 };

async function flush() {
  for (let index = 0; index < 8; index += 1) await nextTick();
}

function connectionChecks(): number {
  return mocks.ipcInvoke.mock.calls.filter(([command]) => command === "check_unity_connection_status").length;
}

describe("workspace Unity launch wait", () => {
  let handler: ((event: RoutedWorkspaceEvent) => void) | null;

  beforeEach(() => {
    vi.useFakeTimers();
    vi.clearAllMocks();
    handler = null;
    mocks.subscribe.mockImplementation(async (_event: string, next: typeof handler) => {
      handler = next;
      return () => {};
    });
    mocks.ipcInvoke.mockImplementation(async (command: string) => {
      if (command === "check_unity_connection_status") return { connected: false };
      if (command === "check_unity_plugin") return { status: "upToDate" };
      if (command === "launch_unity_project") {
        return { editorPath: "Unity.exe", projectPath: "F:/Game", projectVersion: "6000.5", processId: 9, mode: "editor" };
      }
      throw new Error(`Unexpected command: ${command}`);
    });
  });
  afterEach(() => vi.useRealTimers());

  it("waits for the pushed connection instead of polling", async () => {
    const scope = effectScope();
    const state = scope.run(() => useWorkspaceUnityStatus({ workspaceRef: ref(workspaceRef) }))!;
    await flush();
    const afterBind = connectionChecks();
    expect(await state.launch()).toBe(true);
    expect(state.launchState.value).toBe("waitingConnection");

    await vi.advanceTimersByTimeAsync(30_000);
    await flush();
    expect(connectionChecks()).toBe(afterBind);
    expect(state.launchState.value).toBe("waitingConnection");

    handler!({
      checkoutId: workspaceRef.checkoutId,
      workspaceGeneration: 1,
      eventName: "unity-connection-status",
      payload: true,
    } as RoutedWorkspaceEvent);
    await flush();
    expect(state.connected.value).toBe(true);
    expect(state.launchState.value).toBe("idle");
    scope.stop();
  });

  it("gives up after the launch deadline without a connection", async () => {
    const scope = effectScope();
    const state = scope.run(() => useWorkspaceUnityStatus({ workspaceRef: ref(workspaceRef) }))!;
    await flush();
    await state.launch();
    await vi.advanceTimersByTimeAsync(120_000);
    await flush();
    expect(state.launchState.value).toBe("idle");
    expect(state.connected.value).toBe(false);
    scope.stop();
  });
});
