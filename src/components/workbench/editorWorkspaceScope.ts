import { workspaceMaterializationMatches, type WorkspaceRef, type WorkspaceRuntimeDescriptor } from "../../services/project";
import type { EditorCheckoutBinding } from "../../types/workbench";

/** Refresh a process incarnation only after the durable assignment matches.
 * Never replace a persisted epoch with the currently occupying pool slot. */
export function workspaceRefForEditorBinding(binding: EditorCheckoutBinding | null | undefined, runtime?: WorkspaceRuntimeDescriptor | null): WorkspaceRef | null {
  if (!binding?.checkoutId) return null;
  const sameAssignment = runtime?.checkoutId === binding.checkoutId
    && workspaceMaterializationMatches(binding.expectedMaterializationEpoch, runtime.materializationEpoch);
  return {
    checkoutId: binding.checkoutId,
    expectedGeneration: sameAssignment ? runtime.workspaceGeneration : binding.expectedGeneration ?? undefined,
    expectedMaterializationEpoch: binding.expectedMaterializationEpoch ?? undefined,
  };
}

export function editorBindingMatchesRuntime(binding: EditorCheckoutBinding | null | undefined, runtime?: WorkspaceRuntimeDescriptor | null): boolean {
  return !!binding?.checkoutId && !!runtime && binding.checkoutId === runtime.checkoutId
    && workspaceMaterializationMatches(binding.expectedMaterializationEpoch, runtime.materializationEpoch);
}
