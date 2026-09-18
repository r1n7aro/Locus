import { describe, expect, it } from "vitest";
import { viewWorkspaceDragReference } from "../components/view/viewWorkspaceDrag";
import type { ViewPackageSummary } from "../services/view";

const view = { id: "combat-view", name: "Combat", icon: "eye", packageRoot: "F:/Project/Locus/views/combat-view.vue" } as ViewPackageSummary;

describe("view workspace drag", () => {
  it("captures the source checkout and materialization for a persistent view shortcut", () => {
    const workspaceRef = { checkoutId: "checkout-a", expectedGeneration: 3, expectedMaterializationEpoch: 7 };
    const reference = viewWorkspaceDragReference("project-a", workspaceRef, view);
    workspaceRef.expectedMaterializationEpoch = 8;
    expect(reference).toEqual({
      projectId: "project-a",
      workspaceRef: { checkoutId: "checkout-a", expectedGeneration: 3, expectedMaterializationEpoch: 7 },
      view,
    });
  });

  it("does not treat folders or unscoped views as transferable view shortcuts", () => {
    const scope = { checkoutId: "checkout-a" };
    expect(viewWorkspaceDragReference("project-a", scope, undefined)).toBeNull();
    expect(viewWorkspaceDragReference(undefined, scope, view)).toBeNull();
    expect(viewWorkspaceDragReference("project-a", null, view)).toBeNull();
  });
});
