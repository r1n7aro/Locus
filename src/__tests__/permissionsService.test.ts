import { beforeEach, describe, expect, it, vi } from "vitest";

const ipcInvokeMock = vi.hoisted(() => vi.fn());

vi.mock("../services/ipc", () => ({
  ipcInvoke: ipcInvokeMock,
}));

import { saveToolPermissionMode } from "../services/permissions";

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
});
