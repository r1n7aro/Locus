// @vitest-environment jsdom
import { createPinia, setActivePinia } from "pinia";
import { beforeEach, describe, expect, it } from "vitest";
import { useAgentWorkspaceScope } from "../composables/useAgentWorkspaceScope";
import { useDisplaySettings } from "../composables/useDisplaySettings";
import type { WorkspaceCheckoutDescriptor } from "../services/project";
import { useWorkbenchStore } from "../stores/workbench";
import { useWorkspaceContextStore } from "../stores/workspaceContext";

function checkout(checkoutId: string, services: string[] = []): WorkspaceCheckoutDescriptor {
  const projectId = `project-${checkoutId}`;
  const root = `F:/projects/${checkoutId}`;
  return {
    checkoutId, projectId, root, normalizedRoot: root.toLowerCase(), lastOpenedAt: 1,
    runtime: {
      checkoutId, projectId, root, detectedServices: services,
      workspaceGeneration: 7, leaseCount: 1,
    },
  };
}

function bindPane(windowId: string, paneId: string, checkoutId: string) {
  useWorkspaceContextStore().paneContexts[`${windowId}\u0000${paneId}`] = {
    windowId, paneId, focusedCheckoutId: checkoutId, workspaceGeneration: 7,
    intentEpoch: 1, revision: 1,
  };
}

describe("main Agent workspace scope", () => {
  beforeEach(() => {
    localStorage.clear();
    setActivePinia(createPinia());
    useDisplaySettings().state.workspaceDisplayMode = "single";
    useWorkspaceContextStore().checkoutsById = {
      unity: checkout("unity", ["unity"]),
      general: checkout("general"),
    };
  });

  it("uses the single-workspace tree even while another pane owns the global focus", () => {
    const contexts = useWorkspaceContextStore();
    useWorkbenchStore().switchWorkspaceScope("main", "unity");
    bindPane("auxiliary", "other", "general");
    contexts.activatePane("auxiliary", "other");

    const scope = useAgentWorkspaceScope();
    expect(contexts.focusedRuntime?.detectedServices).toEqual([]);
    expect(scope.workspaceRef.value).toEqual({ checkoutId: "unity", expectedGeneration: 7 });
    expect(scope.workingDir.value).toBe("F:/projects/unity");
    expect(scope.runtime.value?.detectedServices).toEqual(["unity"]);
  });

  it("retains the tree workspace without an active conversation or pane context", () => {
    useWorkbenchStore().switchWorkspaceScope("main", "unity");
    const scope = useAgentWorkspaceScope();
    expect(useWorkspaceContextStore().focusedWorkspaceRef).toBeNull();
    expect(scope.workspaceRef.value?.checkoutId).toBe("unity");
  });

  it("reacts to workspace switches and runtime generation updates", () => {
    const workbench = useWorkbenchStore();
    workbench.switchWorkspaceScope("main", "unity");
    const scope = useAgentWorkspaceScope();
    expect(scope.runtime.value?.detectedServices).toEqual(["unity"]);

    workbench.switchWorkspaceScope("main", "general");
    expect(scope.workingDir.value).toBe("F:/projects/general");
    expect(scope.runtime.value?.detectedServices).toEqual([]);
    useWorkspaceContextStore().checkoutsById.general!.runtime!.workspaceGeneration = 8;
    expect(scope.workspaceRef.value).toEqual({ checkoutId: "general", expectedGeneration: 8 });
  });

  it("uses the restored main pane before the tree has adopted a workspace scope", () => {
    bindPane("main", "main", "unity");
    bindPane("auxiliary", "other", "general");
    useWorkspaceContextStore().activatePane("auxiliary", "other");
    expect(useAgentWorkspaceScope().workspaceRef.value?.checkoutId).toBe("unity");
  });

  it("preserves focused-checkout behavior in multi-workspace mode", () => {
    useWorkbenchStore().switchWorkspaceScope("main", "unity");
    bindPane("main", "main", "general");
    const settings = useDisplaySettings().state;
    const scope = useAgentWorkspaceScope();
    expect(scope.workspaceRef.value?.checkoutId).toBe("unity");
    settings.workspaceDisplayMode = "multi";
    expect(scope.workspaceRef.value?.checkoutId).toBe("general");
    settings.workspaceDisplayMode = "single";
    expect(scope.workspaceRef.value?.checkoutId).toBe("unity");
  });

  it("does not substitute a different focused workspace for a missing scoped runtime", () => {
    bindPane("main", "main", "general");
    useWorkbenchStore().switchWorkspaceScope("main", "unity");
    useWorkspaceContextStore().checkoutsById.unity!.runtime = null;
    const scope = useAgentWorkspaceScope();
    expect(scope.workspaceRef.value).toBeNull();
    expect(scope.workingDir.value).toBe("F:/projects/unity");
    delete useWorkspaceContextStore().checkoutsById.unity;
    expect(scope.workspaceRef.value).toBeNull();
    expect(scope.workingDir.value).toBe("");
  });
});
