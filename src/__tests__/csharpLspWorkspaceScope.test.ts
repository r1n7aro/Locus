import { beforeEach, describe, expect, it, vi } from "vitest";

const runtime = vi.hoisted(() => ({
  handler: null as ((payload: unknown) => void) | null,
  subscribe: vi.fn(),
}));

vi.mock("../services/ipc", () => ({
  ipcInvoke: vi.fn(),
}));

vi.mock("../services/locusRuntime", () => ({
  getLocusRuntime: () => ({
    kind: "tauri",
    invoke: vi.fn(),
    subscribe: runtime.subscribe,
  }),
}));

import { ipcInvoke } from "../services/ipc";
import {
  csharpLspGetStatus,
  csharpLspRestart,
  csharpLspSetEnabled,
  subscribeCsharpLspStatus,
  subscribeUnityHotReloadSelfTest,
  subscribeUnitySidecarCompilerStatus,
  unityHotReloadSetEnabled,
  unityHotReloadPreflight,
  unitySidecarCompilerGetStatus,
} from "../services/csharpLsp";
import type { CsharpCompileStatus, CsharpLspStatus } from "../types";

const mockedInvoke = vi.mocked(ipcInvoke);
const workspaceRef = {
  checkoutId: "checkout-b",
  expectedGeneration: 23,
};

function status(checkoutId: string, generation: number): CsharpLspStatus {
  return {
    checkoutId,
    workspaceGeneration: generation,
    serviceInstanceId: `lsp-${checkoutId}`,
    serviceGeneration: 4,
    enabled: true,
    supported: true,
    phase: "ready",
    serverVersion: "test",
  };
}

function routedStatus(checkoutId: string, generation: number) {
  const payload = status(checkoutId, generation);
  return {
    eventName: "csharp-lsp-status",
    streamRevision: 1,
    projectId: "project-csharp",
    checkoutId,
    workspaceGeneration: generation,
    serviceInstanceId: payload.serviceInstanceId,
    serviceGeneration: payload.serviceGeneration,
    payload,
  };
}

describe("C# LSP checkout scope", () => {
  beforeEach(() => {
    mockedInvoke.mockReset();
    mockedInvoke.mockResolvedValue(undefined);
    runtime.handler = null;
    runtime.subscribe.mockReset();
    runtime.subscribe.mockImplementation(async (_eventName, handler) => {
      runtime.handler = handler;
      return () => {};
    });
  });

  it("forwards WorkspaceRef through status, lifecycle, and Unity preflight IPC", async () => {
    await csharpLspGetStatus(workspaceRef);
    await csharpLspSetEnabled(true, workspaceRef);
    await csharpLspRestart(workspaceRef);
    await unityHotReloadPreflight(workspaceRef);
    await unitySidecarCompilerGetStatus(workspaceRef);
    await unityHotReloadSetEnabled(true, workspaceRef);

    expect(mockedInvoke).toHaveBeenNthCalledWith(
      1,
      "csharp_lsp_get_status",
      { workspaceRef },
      expect.any(Object),
    );
    expect(mockedInvoke).toHaveBeenNthCalledWith(
      2,
      "csharp_lsp_set_enabled",
      { value: true, workspaceRef },
      expect.any(Object),
    );
    expect(mockedInvoke).toHaveBeenNthCalledWith(
      3,
      "csharp_lsp_restart",
      { workspaceRef },
      expect.any(Object),
    );
    expect(mockedInvoke).toHaveBeenNthCalledWith(
      4,
      "unity_hot_reload_preflight",
      { workspaceRef },
      expect.any(Object),
    );
    expect(mockedInvoke).toHaveBeenNthCalledWith(
      5,
      "unity_sidecar_compiler_get_status",
      { workspaceRef },
      expect.any(Object),
    );
    expect(mockedInvoke).toHaveBeenNthCalledWith(
      6,
      "unity_hot_reload_set_enabled",
      { value: true, workspaceRef },
      expect.any(Object),
    );
  });

  it("drops background checkout and stale-generation status events", async () => {
    const received: CsharpLspStatus[] = [];
    await subscribeCsharpLspStatus(workspaceRef, (payload) => received.push(payload));

    runtime.handler?.(routedStatus("checkout-a", 23));
    runtime.handler?.(routedStatus("checkout-b", 22));
    runtime.handler?.(routedStatus("checkout-b", 23));

    expect(received).toEqual([status("checkout-b", 23)]);
  });

  it("filters sidecar and self-test workspace envelopes by checkout generation", async () => {
    const compilePayload = { enabled: true } as CsharpCompileStatus;
    const compileReceived: CsharpCompileStatus[] = [];
    await subscribeUnitySidecarCompilerStatus(
      workspaceRef,
      (payload) => compileReceived.push(payload),
    );
    runtime.handler?.({
      eventName: "csharp-compile-status",
      checkoutId: "checkout-a",
      workspaceGeneration: 23,
      payload: compilePayload,
    });
    runtime.handler?.({
      eventName: "csharp-compile-status",
      checkoutId: "checkout-b",
      workspaceGeneration: 22,
      payload: compilePayload,
    });
    runtime.handler?.({
      eventName: "csharp-compile-status",
      checkoutId: "checkout-b",
      workspaceGeneration: 23,
      payload: compilePayload,
    });
    expect(compileReceived).toEqual([compilePayload]);

    const selfTestReceived: Array<{ running: boolean }> = [];
    await subscribeUnityHotReloadSelfTest(
      workspaceRef,
      (payload) => selfTestReceived.push(payload),
    );
    runtime.handler?.({
      eventName: "unity-hotreload-selftest",
      checkoutId: "checkout-a",
      workspaceGeneration: 23,
      payload: { running: true, finished: false, passed: 0, failed: 0 },
    });
    runtime.handler?.({
      eventName: "unity-hotreload-selftest",
      checkoutId: "checkout-b",
      workspaceGeneration: 23,
      payload: { running: true, finished: false, passed: 0, failed: 0 },
    });
    expect(selfTestReceived).toHaveLength(1);
    expect(selfTestReceived[0]?.running).toBe(true);
  });
});
