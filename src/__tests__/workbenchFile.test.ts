import { beforeEach, describe, expect, it, vi } from "vitest";

const mocks = vi.hoisted(() => ({
  emitTo: vi.fn(),
  label: "main",
  available: true,
}));

vi.mock("@tauri-apps/api/event", () => ({ emitTo: mocks.emitTo }));
vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: () => ({ label: mocks.label }),
}));
vi.mock("../services/tauriRuntime", () => ({
  hasTauriWindowRuntime: () => mocks.available,
}));
vi.mock("../services/workbenchWindow", () => ({
  isWorkbenchWindowLabel: (label: string) => label === "main" || label.startsWith("workbench-"),
}));

import { openWorkbenchFileTab, WORKBENCH_FILE_OPEN_EVENT } from "../services/workbenchFile";

describe("Workbench file routing", () => {
  const request = {
    filePath: "Assets/Scripts/Player.cs",
    workspaceRef: { checkoutId: "session-checkout", expectedGeneration: 7, expectedMaterializationEpoch: 2 },
    highlight: { mode: "all" as const },
  };

  beforeEach(() => {
    mocks.emitTo.mockReset().mockResolvedValue(undefined);
    mocks.available = true;
    mocks.label = "main";
  });

  it.each([
    ["main", "main"],
    ["workbench-code", "workbench-code"],
    ["chat-session-window", "main"],
  ])("routes %s to %s with the source checkout and location", async (source, target) => {
    mocks.label = source;
    await openWorkbenchFileTab(request);
    expect(mocks.emitTo).toHaveBeenCalledWith(target, WORKBENCH_FILE_OPEN_EVENT, {
      ...request,
      targetLabel: target,
    });
  });

  it("propagates routing failures to the tool action", async () => {
    mocks.emitTo.mockRejectedValue(new Error("Window closed"));
    await expect(openWorkbenchFileTab(request)).rejects.toThrow("Window closed");
    mocks.available = false;
    await expect(openWorkbenchFileTab(request)).rejects.toThrow("Workbench window is unavailable");
  });
});
