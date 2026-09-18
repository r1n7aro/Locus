import type { WorkspaceRef } from "../services/project";

/** Include checkout lifetime as well as resource identity; never key drafts by title alone. */
export function documentSessionKey(
  workspaceRef: WorkspaceRef | null | undefined,
  resource: readonly (string | number | null)[],
): string {
  return JSON.stringify([
    workspaceRef ? [
      "workspace",
      workspaceRef.checkoutId,
      workspaceRef.expectedGeneration ?? "current",
      workspaceRef.expectedMaterializationEpoch ?? "current",
    ] : ["workspace:unbound"],
    ...resource,
  ]);
}
