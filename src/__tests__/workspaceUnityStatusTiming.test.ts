import { effectScope, nextTick, ref, watch } from "vue";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { RoutedWorkspaceEvent, WorkspaceRef } from "../services/project";
import type { UnityConnectionStatus } from "../types";

const mocks = vi.hoisted(() => ({ subscribe: vi.fn(), ipcInvoke: vi.fn() }));
vi.mock("../services/locusRuntime", () => ({
  getLocusRuntime: () => ({ kind: "tauri", invoke: vi.fn(), subscribe: mocks.subscribe }),
}));
vi.mock("../services/ipc", () => ({ ipcInvoke: mocks.ipcInvoke }));
vi.mock("@tauri-apps/plugin-dialog", () => ({ confirm: vi.fn() }));

import {
  sameUnityConnectionStatusIgnoringTiming,
  useWorkspaceUnityStatus,
} from "../composables/useWorkspaceUnityStatus";

const workspaceRef: WorkspaceRef = { checkoutId: "checkout-a", expectedGeneration: 1 };

function status(overrides: Partial<UnityConnectionStatus> = {}): UnityConnectionStatus {
  return {
    connected: true,
    editorStatus: "ready",
    controlChannelState: "ready",
    editorProcessState: "running",
    editorProcessId: 7,
    headless: false,
    processCheckedAtMs: 100,
    pipeName: "pipe",
    latencyMs: 3,
    reconnectAttempts: 0,
    backgroundHook: {
      enabled: false,
      supported: false,
      state: "inactive",
      patched: false,
      processId: null,
      editorProcessPath: null,
      symbolCount: 0,
      error: null,
      updatedAtMs: 100,
    },
    checkedAtMs: 100,
    ...overrides,
  } as UnityConnectionStatus;
}

async function flush() {
  for (let index = 0; index < 5; index += 1) await nextTick();
}

describe("workspace Unity connection status timing", () => {
  let handler: ((event: RoutedWorkspaceEvent) => void) | null;

  beforeEach(() => {
    vi.clearAllMocks();
    handler = null;
    mocks.subscribe.mockImplementation(async (_event: string, next: typeof handler) => {
      handler = next;
      return () => {};
    });
    mocks.ipcInvoke.mockImplementation(async (command: string) => {
      if (command === "check_unity_connection_status") return { connected: true };
      if (command === "check_unity_plugin") return { status: "upToDate" };
      throw new Error(`Unexpected command: ${command}`);
    });
  });

  function emit(payload: UnityConnectionStatus) {
    handler!({
      checkoutId: workspaceRef.checkoutId,
      workspaceGeneration: 1,
      eventName: "unity-connection-status-detail",
      payload,
    } as RoutedWorkspaceEvent);
  }

  it("treats statuses that differ only in check timing as equal", () => {
    const later = status({
      checkedAtMs: 900,
      processCheckedAtMs: 900,
      latencyMs: 12,
      backgroundHook: { ...status().backgroundHook, updatedAtMs: 900 },
    });
    expect(sameUnityConnectionStatusIgnoringTiming(status(), later)).toBe(true);
    expect(sameUnityConnectionStatusIgnoringTiming(status(), status({ editorStatus: "compiling" }))).toBe(false);
  });

  it("patches timing in place so consumers of the status object do not re-run", async () => {
    const scope = effectScope();
    const state = scope.run(() => useWorkspaceUnityStatus({ workspaceRef: ref(workspaceRef) }))!;
    await flush();
    emit(status());
    const first = state.connectionStatus.value;
    const replacements = vi.fn();
    const checkedAt = vi.fn();
    scope.run(() => {
      watch(state.connectionStatus, replacements);
      watch(() => state.connectionStatus.value?.checkedAtMs, checkedAt);
    });

    emit(status({ checkedAtMs: 400, latencyMs: 9 }));
    await flush();
    expect(state.connectionStatus.value).toBe(first);
    expect(state.connectionStatus.value?.checkedAtMs).toBe(400);
    expect(state.connectionStatus.value?.latencyMs).toBe(9);
    expect(replacements).not.toHaveBeenCalled();
    expect(checkedAt).toHaveBeenCalledTimes(1);

    emit(status({ checkedAtMs: 700, editorStatus: "compiling" }));
    await flush();
    expect(state.connectionStatus.value).not.toBe(first);
    expect(state.connectionStatus.value?.editorStatus).toBe("compiling");
    expect(replacements).toHaveBeenCalledTimes(1);
    scope.stop();
  });
});
