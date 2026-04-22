import { beforeEach, describe, expect, it, vi } from "vitest";
import { createPinia, setActivePinia } from "pinia";
import { useChatStore } from "../stores/chat";
import { useNotificationStore } from "../stores/notification";

const permissionServiceMocks = vi.hoisted(() => ({
  getToolPermissionMode: vi.fn(),
  saveToolPermissionMode: vi.fn(),
}));

vi.mock("../services/permissions", () => permissionServiceMocks);

describe("chat store tool permission mode", () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    vi.clearAllMocks();
    permissionServiceMocks.getToolPermissionMode.mockResolvedValue("auto");
    permissionServiceMocks.saveToolPermissionMode.mockResolvedValue(undefined);
  });

  it("loads the persisted global tool permission mode", async () => {
    permissionServiceMocks.getToolPermissionMode.mockResolvedValue("ask");

    const chatStore = useChatStore();

    await chatStore.loadToolPermissionMode();

    expect(chatStore.toolPermissionMode).toBe("ask");
  });

  it("reverts the mode and shows a notice when saving fails", async () => {
    permissionServiceMocks.saveToolPermissionMode.mockRejectedValue(new Error("disk full"));

    const chatStore = useChatStore();
    const notificationStore = useNotificationStore();

    await chatStore.setToolPermissionMode("ask");

    expect(chatStore.toolPermissionMode).toBe("auto");
    expect(notificationStore.notices).toEqual([
      expect.objectContaining({
        level: "error",
        operation: "saveToolPermissionMode",
        message: expect.stringContaining("disk full"),
      }),
    ]);
  });
});
