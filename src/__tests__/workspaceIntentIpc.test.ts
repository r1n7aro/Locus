import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("../services/ipc", () => ({
  ipcInvoke: vi.fn(),
}));

import { ipcInvoke } from "../services/ipc";
import {
  detachWorkspacePane,
  detachWorkspaceWindow,
  focusWorkspace,
  setActiveWorkspaceSession,
} from "../services/project";

const mockedInvoke = vi.mocked(ipcInvoke);

describe("workspace window intent IPC", () => {
  beforeEach(() => {
    mockedInvoke.mockReset();
  });

  it("forwards the monotonic epoch for every pane mutation", async () => {
    await focusWorkspace("window-a", "pane-a", {
      checkoutId: "checkout-b",
      expectedGeneration: 7,
    }, 11);
    await setActiveWorkspaceSession("window-a", "pane-a", "session-b", 12);
    await detachWorkspacePane("window-a", "pane-a", 13);

    expect(mockedInvoke).toHaveBeenNthCalledWith(1, "focus_workspace", {
      windowId: "window-a",
      paneId: "pane-a",
      workspaceRef: {
        checkoutId: "checkout-b",
        expectedGeneration: 7,
      },
      intentEpoch: 11,
    });
    expect(mockedInvoke).toHaveBeenNthCalledWith(2, "set_active_session", {
      windowId: "window-a",
      paneId: "pane-a",
      activeSessionId: "session-b",
      intentEpoch: 12,
    });
    expect(mockedInvoke).toHaveBeenNthCalledWith(3, "detach_workspace_pane", {
      windowId: "window-a",
      paneId: "pane-a",
      intentEpoch: 13,
    });
  });

  it("forwards the window tombstone epoch", async () => {
    await detachWorkspaceWindow("window-a", 21);

    expect(mockedInvoke).toHaveBeenCalledWith("detach_workspace_window", {
      windowId: "window-a",
      intentEpoch: 21,
    });
  });
});
