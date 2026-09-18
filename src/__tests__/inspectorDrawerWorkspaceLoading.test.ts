import { describe, expect, it, vi } from "vitest";
import { effectScope, nextTick, reactive } from "vue";
import type { WorkspaceRef } from "../services/project";

const mocks = vi.hoisted(() => ({ invoke: vi.fn().mockResolvedValue([]) }));
const workspace = reactive<{ focusedWorkspaceRef: WorkspaceRef | null }>({ focusedWorkspaceRef: null });
vi.mock("../stores/workspaceContext", () => ({ useWorkspaceContextStore: () => workspace }));
vi.mock("../services/ipc", () => ({ ipcInvoke: mocks.invoke }));
vi.mock("../services/tauriRuntime", () => ({ hasTauriWindowRuntime: () => true }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn().mockResolvedValue(() => {}) }));
vi.mock("../services/workspaceEventHub", () => ({ listenWorkspaceEvent: vi.fn().mockResolvedValue(() => {}) }));
vi.mock("../services/propertyTree", () => ({
  defineInspectorPropertyDrawers: vi.fn(), pluginInspectorPropertyDrawerLibrary: {},
}));
vi.mock("../services/unityObjectDrawer", () => ({
  defineUnityObjectDrawers: vi.fn(), pluginUnityObjectDrawerLibrary: {},
}));

import { bootstrapPluginInspectorDrawers } from "../services/inspectorDrawerExtensions";

describe("Inspector drawer workspace loading", () => {
  it("loads app drawers, then follows restored and switched workspaces without a plugin event", async () => {
    const scope = effectScope();
    try {
      scope.run(() => bootstrapPluginInspectorDrawers());
      expect(mocks.invoke).toHaveBeenLastCalledWith("plugin_inspector_drawer_packages", { workspaceRef: null });
      workspace.focusedWorkspaceRef = { checkoutId: "first", expectedGeneration: 1, expectedMaterializationEpoch: 1 };
      await nextTick();
      expect(mocks.invoke).toHaveBeenLastCalledWith("plugin_inspector_drawer_packages", { workspaceRef: workspace.focusedWorkspaceRef });
      workspace.focusedWorkspaceRef = { checkoutId: "second", expectedGeneration: 2, expectedMaterializationEpoch: 1 };
      await nextTick();
      expect(mocks.invoke).toHaveBeenLastCalledWith("plugin_inspector_drawer_packages", { workspaceRef: workspace.focusedWorkspaceRef });
      workspace.focusedWorkspaceRef = { ...workspace.focusedWorkspaceRef, expectedMaterializationEpoch: 2 };
      await nextTick();
      expect(mocks.invoke).toHaveBeenCalledTimes(4);
      expect(mocks.invoke).toHaveBeenLastCalledWith("plugin_inspector_drawer_packages", { workspaceRef: workspace.focusedWorkspaceRef });
      workspace.focusedWorkspaceRef = null;
      await nextTick();
      expect(mocks.invoke).toHaveBeenLastCalledWith("plugin_inspector_drawer_packages", { workspaceRef: null });
    } finally {
      scope.stop();
    }
  });
});
