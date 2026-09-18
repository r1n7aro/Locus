// @vitest-environment jsdom
import { createApp, nextTick } from "vue";
import { afterEach, describe, expect, it, vi } from "vitest";
import ToolCallBlock from "../components/ToolCallBlock.vue";
import { WORKBENCH_FILE_OPEN_KEY } from "../services/workbenchFile";

const mocks = vi.hoisted(() => ({
  notice: vi.fn(),
  fallbackOpen: vi.fn(),
  focusedWorkspaceRef: { checkoutId: "other-checkout", expectedGeneration: 9 },
}));
vi.mock("../stores/project", () => ({
  useProjectStore: () => ({ workingDir: "F:/Game", extraWorkdirs: {} }),
}));
vi.mock("../stores/workspaceContext", () => ({
  useWorkspaceContextStore: () => ({ focusedWorkspaceRef: mocks.focusedWorkspaceRef }),
}));
vi.mock("../stores/notification", () => ({
  useNotificationStore: () => ({ addNotice: mocks.notice }),
}));
vi.mock("../services/workbenchFile", () => ({
  WORKBENCH_FILE_OPEN_KEY: Symbol("workbench-file-open"),
  openWorkbenchFileTab: mocks.fallbackOpen,
}));
vi.mock("../components/MarkdownRenderer.vue", () => ({ default: { render: () => null } }));
vi.mock("../components/diff/FileDiffViewer.vue", () => ({ default: { render: () => null } }));
vi.mock("../components/tool-block-overrides/toolBlockOverrides", () => ({
  resolveToolBlockOverride: () => null,
}));

afterEach(() => { vi.clearAllMocks(); });

describe("tool file Workbench actions", () => {
  it.each(["read", "edit", "write"])("opens %s in its containing Workbench using the session checkout", async (name) => {
    const workspaceRef = { checkoutId: "session-checkout", expectedGeneration: 7, expectedMaterializationEpoch: 3 };
    const open = vi.fn().mockResolvedValue(undefined);
    const app = createApp(ToolCallBlock, {
      toolCall: { id: "file-call", name, arguments: JSON.stringify({ filePath: "Assets/Player.cs" }), status: "done" },
      workspaceRef,
    });
    app.provide(WORKBENCH_FILE_OPEN_KEY, open);
    const host = document.createElement("div");
    app.mount(host);
    try {
      host.querySelector<HTMLButtonElement>(".tool-file-preview-action")!.click();
      await nextTick();
      expect(open).toHaveBeenCalledWith(expect.objectContaining({ filePath: "Assets/Player.cs", workspaceRef }));
      expect(mocks.fallbackOpen).not.toHaveBeenCalled();
      expect(mocks.notice).not.toHaveBeenCalled();
    } finally { app.unmount(); }
  });

  it("does not open a different checkout when the session workspace is unavailable", async () => {
    const open = vi.fn();
    const app = createApp(ToolCallBlock, {
      toolCall: { id: "read-call", name: "read", arguments: '{"filePath":"Assets/Player.cs"}', status: "done" },
      workspaceRef: null,
    });
    app.provide(WORKBENCH_FILE_OPEN_KEY, open);
    const host = document.createElement("div");
    app.mount(host);
    try {
      host.querySelector<HTMLButtonElement>(".tool-file-preview-action")!.click();
      await nextTick();
      expect(open).not.toHaveBeenCalled();
      expect(mocks.notice).toHaveBeenCalledWith("error", expect.any(String), expect.objectContaining({ operation: "openWorkbenchFileTab" }));
    } finally { app.unmount(); }
  });
});
