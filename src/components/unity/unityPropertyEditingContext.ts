import { inject, type InjectionKey } from "vue";
import type { WorkspaceRef } from "../../services/project";
import type { UnityBoundPropertyRuntimeAdapter } from "./unityPropertyBinding";

export interface UnityPropertyEditingContext {
  workspaceRef: WorkspaceRef;
  historyOwner: string;
  adapter: UnityBoundPropertyRuntimeAdapter;
}
export const UNITY_PROPERTY_EDITING: InjectionKey<UnityPropertyEditingContext> = Symbol("UnityPropertyEditing");
export const UNITY_PROPERTY_WORKSPACE: InjectionKey<() => WorkspaceRef> = Symbol("UnityPropertyWorkspace");
export function useUnityPropertyEditingContext() { return inject(UNITY_PROPERTY_EDITING, null); }
export function useUnityPropertyWorkspace() { return inject(UNITY_PROPERTY_WORKSPACE, null); }
export function propertyEditingMatchesWorkspace(context: UnityPropertyEditingContext | null, workspace: WorkspaceRef) {
  return !!context && context.workspaceRef.checkoutId === workspace.checkoutId
    && context.workspaceRef.expectedGeneration === workspace.expectedGeneration
    && context.workspaceRef.expectedMaterializationEpoch === workspace.expectedMaterializationEpoch;
}
