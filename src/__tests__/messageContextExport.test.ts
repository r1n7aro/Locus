import { describe, expect, it, vi } from "vitest";
import { exportSessionContext } from "../services/session";
import { ipcInvoke } from "../services/ipc";

vi.mock("../services/ipc", () => ({ ipcInvoke: vi.fn() }));

describe("message context export IPC", () => {
  it("passes the selected message without relying on the active session", async () => {
    await exportSessionContext("source-session", null, "selected-message");
    expect(ipcInvoke).toHaveBeenLastCalledWith("export_session_context", {
      sessionId: "source-session", filePath: null, messageId: "selected-message",
    });
  });

  it("keeps whole-session exports unscoped", async () => {
    await exportSessionContext("source-session", "F:/Temp/context.yaml");
    expect(ipcInvoke).toHaveBeenLastCalledWith("export_session_context", {
      sessionId: "source-session", filePath: "F:/Temp/context.yaml", messageId: null,
    });
  });
});
