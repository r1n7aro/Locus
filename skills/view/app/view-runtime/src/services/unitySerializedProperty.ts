import { ipcInvoke } from "./ipc";
import type { UnitySerializedPropertySnapshot } from "../components/unity/unitySerializedValue";

export type UnitySerializedPropertyWriteMode = "commit" | "preview";

export interface UnitySerializedPropertyTarget {
  kind: string;
  path?: string | null;
  scenePath?: string | null;
  objectPath?: string | null;
  componentType?: string | null;
  componentIndex?: number | null;
  propertyPath?: string | null;
}

export interface UnitySerializedPropertyReadRequest {
  bindingId?: string | null;
  target: UnitySerializedPropertyTarget;
  maxDepth?: number | null;
  maxArrayItems?: number | null;
}

export interface UnitySerializedPropertyDiscoverRequest {
  bindingId?: string | null;
  target: UnitySerializedPropertyTarget;
  query?: string | null;
  fieldName?: string | null;
  fieldType?: string | null;
  maxDepth?: number | null;
  maxResults?: number | null;
}

export interface UnitySerializedPropertyWriteRequest {
  bindingId?: string | null;
  target: UnitySerializedPropertyTarget;
  value: unknown;
  writeMode?: UnitySerializedPropertyWriteMode | null;
}

export interface UnitySerializedPropertyApplyWrite {
  bindingId?: string | null;
  target: UnitySerializedPropertyTarget;
  value: unknown;
  writeMode?: UnitySerializedPropertyWriteMode | null;
}

export interface UnitySerializedPropertyApplyRequest {
  writes: UnitySerializedPropertyApplyWrite[];
}

export interface UnitySerializedPropertyDiscoverMatch {
  propertyPath: string;
  displayName: string;
  name: string;
  type: string;
  valueType: string;
  fieldTypeFullName: string;
  fieldTypeAssembly: string;
  displayValue: string;
  editable: boolean;
  hasChildren: boolean;
  isArray: boolean;
  isManagedReference: boolean;
  depth: number;
}

export interface UnitySerializedPropertyReadResult extends UnitySerializedPropertySnapshot {
  ok: boolean;
  bindingId?: string | null;
  message: string;
  target: UnitySerializedPropertyTarget;
}

export interface UnitySerializedPropertyDiscoverResult {
  ok: boolean;
  bindingId?: string | null;
  message: string;
  target: UnitySerializedPropertyTarget;
  matches: UnitySerializedPropertyDiscoverMatch[];
}

export interface UnitySerializedPropertyWriteResult extends UnitySerializedPropertyReadResult {
  saved: boolean;
}

export interface UnitySerializedPropertyApplyResult {
  ok: boolean;
  message: string;
  results: UnitySerializedPropertyWriteResult[];
}

export function readUnitySerializedProperty(
  request: UnitySerializedPropertyReadRequest,
): Promise<UnitySerializedPropertyReadResult> {
  return ipcInvoke<UnitySerializedPropertyReadResult>("unity_serialized_property_read", { request });
}

export function discoverUnitySerializedProperties(
  request: UnitySerializedPropertyDiscoverRequest,
): Promise<UnitySerializedPropertyDiscoverResult> {
  return ipcInvoke<UnitySerializedPropertyDiscoverResult>("unity_serialized_property_discover", { request });
}

export function writeUnitySerializedProperty(
  request: UnitySerializedPropertyWriteRequest,
): Promise<UnitySerializedPropertyWriteResult> {
  return ipcInvoke<UnitySerializedPropertyWriteResult>("unity_serialized_property_write", { request });
}

export function applyUnitySerializedProperties(
  request: UnitySerializedPropertyApplyRequest,
): Promise<UnitySerializedPropertyApplyResult> {
  return ipcInvoke<UnitySerializedPropertyApplyResult>("unity_serialized_property_apply", { request });
}
