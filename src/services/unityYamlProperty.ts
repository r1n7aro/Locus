/** Transport only: Rust owns YAML addresses, graph traversal, projection and writes. */
import { ipcInvoke } from "./ipc";
import { encodeAssetValue } from "./unityAssets";
import { exactUnityPropertyTarget, type UnitySerializedPropertyApplyRequest, type UnitySerializedPropertyApplyResult,
  type UnitySerializedPropertyReadRequest, type UnitySerializedPropertyReadResult } from "./unitySerializedProperty";
import type { UnityYamlPropertySummaryRequest, UnityYamlPropertySummaryResult } from "./unitySerializedProperty";
import type { WorkspaceRef } from "./project";
import type { UnitySerializedPropertyDiscoverRequest, UnitySerializedPropertyDiscoverResult } from "./unitySerializedProperty";

export async function discoverYamlProperties(workspaceRef: WorkspaceRef, request: UnitySerializedPropertyDiscoverRequest): Promise<UnitySerializedPropertyDiscoverResult> {
  return ipcInvoke("unity_assets_execute", { workspaceRef, request: { ...request, target: exactUnityPropertyTarget(request.target), action: "discover_property", backend: "yaml" } });
}

export async function readYamlProperty(workspaceRef: WorkspaceRef, request: UnitySerializedPropertyReadRequest): Promise<UnitySerializedPropertyReadResult> {
  return ipcInvoke("unity_assets_execute", { workspaceRef, request: {
    ...request, target: exactUnityPropertyTarget(request.target), action: "read_property", backend: "yaml",
  } });
}

export async function applyYamlProperties(workspaceRef: WorkspaceRef, request: UnitySerializedPropertyApplyRequest, signal?: AbortSignal): Promise<UnitySerializedPropertyApplyResult> {
  return sendYamlApply(workspaceRef, request, signal);
}
export async function applyYamlPropertySummary(workspaceRef: WorkspaceRef, request: UnityYamlPropertySummaryRequest, signal?: AbortSignal): Promise<UnityYamlPropertySummaryResult> {
  return sendYamlApply(workspaceRef, request, signal);
}
async function sendYamlApply<T>(workspaceRef: WorkspaceRef, request: UnitySerializedPropertyApplyRequest | UnityYamlPropertySummaryRequest, signal?: AbortSignal): Promise<T> {
  if (signal?.aborted) throw new Error("Frontend execution was cancelled.");
  const writes = request.writes.map((write) => ({ ...write, target: exactUnityPropertyTarget(write.target), value: encodeAssetValue(write.value) }));
  return ipcInvoke("unity_assets_execute", { workspaceRef, request: { action: "apply_properties", backend: "yaml", writes, ...(request.profile === undefined ? {} : { profile: request.profile }), ...(request.resultMode === undefined ? {} : { resultMode: request.resultMode }) } });
}
