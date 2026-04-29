import { beforeEach, describe, expect, it, vi } from "vitest";

const ipcInvokeMock = vi.hoisted(() => vi.fn());

vi.mock("../services/ipc", () => ({
  ipcInvoke: ipcInvokeMock,
}));

import {
  getCachedDebugMode,
  getDebugMode,
  saveToolPermissionMode,
  setDebugMode,
} from "../services/permissions";

describe("permissions service", () => {
  beforeEach(() => {
    ipcInvokeMock.mockReset();
    ipcInvokeMock.mockResolvedValue(undefined);
  });

  it("saves the global tool permission mode with the backend value field", async () => {
    await saveToolPermissionMode("ask");

    expect(ipcInvokeMock).toHaveBeenCalledWith("save_tool_permission_mode", {
      value: "ask",
    });
  });

  it("caches loaded debug mode for remounted settings panels", async () => {
    ipcInvokeMock.mockResolvedValueOnce(true);

    await expect(getDebugMode()).resolves.toBe(true);
    await expect(getDebugMode()).resolves.toBe(true);

    expect(getCachedDebugMode()).toBe(true);
    expect(ipcInvokeMock).toHaveBeenCalledTimes(1);
    expect(ipcInvokeMock).toHaveBeenCalledWith("get_debug_mode");
  });

  it("updates the cached debug mode after saving", async () => {
    await setDebugMode(false);

    expect(getCachedDebugMode()).toBe(false);
    await expect(getDebugMode()).resolves.toBe(false);
    expect(ipcInvokeMock).toHaveBeenCalledTimes(1);
    expect(ipcInvokeMock).toHaveBeenCalledWith("set_debug_mode", {
      value: false,
    });
  });
});
