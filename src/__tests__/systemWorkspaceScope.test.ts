import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("../services/ipc", () => ({
  ipcInvoke: vi.fn(),
}));

import { ipcInvoke } from "../services/ipc";
import {
  unityNativeBridgeSelfTestRun,
  unityNativeBridgeSetEnabled,
  unityNativeBrokerGetStatus,
  unitySemanticStateGet,
  unityStateProbeGetStatus,
  unityStateProbeSelfTestRun,
  unityStateProbeSetEnabled,
} from "../services/csharpLsp";
import { runUnityIntegrationTests } from "../services/integrationTests";
import {
  getUnityBackgroundHookStatus,
  setUnityBackgroundHookEnabled,
  setUnityExternalEditorDefaultEnabled,
} from "../services/system";

const mockedInvoke = vi.mocked(ipcInvoke);
const workspaceRef = {
  checkoutId: "checkout-system",
  expectedGeneration: 31,
};

describe("system Unity checkout scope", () => {
  beforeEach(() => {
    mockedInvoke.mockReset();
    mockedInvoke.mockResolvedValue(undefined);
  });

  it("forwards scope through settings and runtime status commands", async () => {
    await setUnityBackgroundHookEnabled(true, workspaceRef);
    await setUnityExternalEditorDefaultEnabled(true, workspaceRef);
    await unityNativeBridgeSetEnabled(true, workspaceRef);
    await unityNativeBrokerGetStatus(workspaceRef);
    await unitySemanticStateGet(workspaceRef);

    expect(mockedInvoke).toHaveBeenNthCalledWith(1, "set_unity_background_hook_enabled", {
      value: true,
      workspaceRef,
    });
    expect(mockedInvoke).toHaveBeenNthCalledWith(2, "set_unity_external_editor_default_enabled", {
      value: true,
      workspaceRef,
    });
    expect(mockedInvoke).toHaveBeenNthCalledWith(
      3,
      "set_unity_native_bridge_enabled",
      { value: true, workspaceRef },
      expect.any(Object),
    );
    expect(mockedInvoke).toHaveBeenNthCalledWith(
      4,
      "get_unity_native_broker_status",
      { workspaceRef },
      expect.any(Object),
    );
    expect(mockedInvoke).toHaveBeenNthCalledWith(
      5,
      "get_unity_semantic_state",
      { workspaceRef },
      expect.any(Object),
    );
  });

  it("forwards scope through Ready-gated diagnostics", async () => {
    await unityStateProbeSelfTestRun(workspaceRef);
    await unityNativeBridgeSelfTestRun(workspaceRef);
    await runUnityIntegrationTests({ suites: ["connect"] }, workspaceRef);

    expect(mockedInvoke).toHaveBeenNthCalledWith(
      1,
      "unity_state_probe_selftest_run",
      { workspaceRef },
      expect.any(Object),
    );
    expect(mockedInvoke).toHaveBeenNthCalledWith(
      2,
      "unity_native_bridge_selftest_run",
      { workspaceRef },
      expect.any(Object),
    );
    expect(mockedInvoke).toHaveBeenNthCalledWith(
      3,
      "unity_integration_test_run",
      { request: { suites: ["connect"] }, workspaceRef },
      expect.any(Object),
    );
  });

  it("requires scope for checkout-observable probe and hook status", async () => {
    await getUnityBackgroundHookStatus(workspaceRef);
    await unityStateProbeGetStatus(workspaceRef);
    await unityStateProbeSetEnabled(true, workspaceRef);

    expect(mockedInvoke).toHaveBeenNthCalledWith(1, "get_unity_background_hook_status", {
      workspaceRef,
    });
    expect(mockedInvoke).toHaveBeenNthCalledWith(
      2,
      "get_unity_state_probe_status",
      { workspaceRef },
      expect.any(Object),
    );
    expect(mockedInvoke).toHaveBeenNthCalledWith(
      3,
      "set_unity_state_probe_enabled",
      { value: true, workspaceRef },
      expect.any(Object),
    );
  });
});
