import { ipcInvoke } from "./ipc";
import type {
  UnitySerializedPropertySnapshot as UnitySerializedPropertySnapshotValue,
  UnitySerializedPropertyTargetSnapshot,
} from "../components/unity/unitySerializedValue";
import type { WorkspaceRef } from "./project";
import { assetInteger } from "./unityAssets";

export type UnitySerializedPropertyWriteMode = "commit" | "preview";
/** live uses SerializedObject on Unity's main thread; yaml edits persisted data. */
export type UnitySerializedPropertyBackend = "yaml" | "live";

/** Explicit initialization data; no constructor execution or inferred defaults. */
export interface UnityManagedGraphTemplate {
  rootRid: string;
  entries: { rid: string; type: { class: string; ns: string; asm: string }; data: Record<string, unknown> }[];
}
export type UnityYamlPropertyCommand =
  | { action: "revert" }
  | { action: "applyToSource"; level: number }
  | { action: "createManaged"; template: UnityManagedGraphTemplate }
  | { action: "editObjects"; add?: { id: string; classId: string; rootType: string; data: Record<string, unknown> }[];
      remove?: string[]; updates?: { objectId: string; propertyPath: string; value: unknown }[] };

export type UnitySerializedPropertyTarget = UnitySerializedPropertyTargetSnapshot;
export type UnitySerializedPropertySnapshot = UnitySerializedPropertySnapshotValue;

export interface UnitySerializedPropertyReadRequest {
  arrayOffset?: number | null;
  bindingId?: string | null;
  target: UnitySerializedPropertyTarget;
  maxDepth?: number | null;
  maxArrayItems?: number | null;
  autoExpandCharLimit?: number | null;
}

export interface UnitySerializedPropertyDiscoverRequest {
  bindingId?: string | null;
  target: UnitySerializedPropertyTarget;
  query?: string | null;
  fieldName?: string | null;
  fieldType?: string | null;
  matchFields?: string[] | null;
  maxDepth?: number | null;
  maxResults?: number | null;
  includeAll?: boolean | null;
}

export interface UnitySerializedPropertyWriteRequest {
  expectedRevision?: string;
  expectedDependencies?: Record<string, string>;
  bindingId?: string | null;
  target: UnitySerializedPropertyTarget;
  value: unknown;
  writeMode?: UnitySerializedPropertyWriteMode | null;
}

export interface UnitySerializedPropertyApplyWrite {
  expectedRevision?: string;
  expectedDependencies?: Record<string, string>;
  bindingId?: string | null;
  target: UnitySerializedPropertyTarget;
  value: unknown;
  writeMode?: UnitySerializedPropertyWriteMode | null;
}

export interface UnitySerializedPropertyApplyRequest {
  writes: UnitySerializedPropertyApplyWrite[];
  /** YAML service timings; Editor write/import timings are present when connected. */
  profile?: boolean;
  resultMode?: "full";
}

export interface UnityYamlPropertySummaryRequest extends Omit<UnitySerializedPropertyApplyRequest, "resultMode"> {
  resultMode: "summary";
}
export interface UnityYamlPropertySummaryResult {
  ok: boolean;
  message: string;
  writesApplied: number;
  transactionId: string | null;
  assets: { path: string; revision: string; dependencies: Record<string, string> }[];
  profile?: UnitySerializedPropertyApplyResult["profile"];
}

export interface UnitySerializedPropertyDiscoverMatch {
  target?: UnitySerializedPropertyTarget;
  semanticPath?: string;
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
  managedReferenceId?: string | number;
  referenceTarget?: UnitySerializedPropertyTarget | null;
  depth: number;
}

export interface UnitySerializedPropertyReadResult extends UnitySerializedPropertySnapshot {
  diagnostics?: { code: string; message: string; severity: string; object_id?: string | null; property_path?: string | null }[];
  backend?: UnitySerializedPropertyBackend;
  revision?: string;
  dependencies?: Record<string, string>;
  prefabLayers?: { path: string; instanceId: string; sourceGuid: string; sourceId: string }[];
  ok: boolean;
  bindingId?: string | null;
  message: string;
  target: UnitySerializedPropertyTarget;
  properties?: UnitySerializedPropertySnapshot[];
}

export interface UnitySerializedPropertyDiscoverResult {
  revision?: string;
  dependencies?: Record<string, string>;
  ok: boolean;
  bindingId?: string | null;
  message: string;
  target: UnitySerializedPropertyTarget;
  matches: UnitySerializedPropertyDiscoverMatch[];
  truncated?: boolean;
  scannedObjects?: number;
  scannedProperties?: number;
}

export interface UnitySerializedPropertyWriteResult extends UnitySerializedPropertyReadResult {
  saved: boolean;
  beforeSnapshot?: UnitySerializedPropertySnapshot | null;
}

export interface UnitySerializedPropertyApplyResult {
  profile?: { prepareMs: number; commitMs: number; projectionMs: number; totalMs: number;
    effectiveBuilds?: number; treeBuilds?: number; overrideValidationPasses?: number; changedFiles?: number;
    materializedCompilePasses?: number; readProjections?: number;
    editor?: { preflightMs: number; writeMs: number; importMs: number; changedFiles: number } | null };
  ok: boolean;
  message: string;
  results: UnitySerializedPropertyWriteResult[];
}

/** Safe-number inputs remain compatible. IDs cross frontend IPC as strings. */
export function exactUnityPropertyTarget(target: UnitySerializedPropertyTarget): UnitySerializedPropertyTarget {
  const result = { ...target };
  for (const key of ["objectFileId", "targetFileId"] as const) {
    const id = result[key];
    if (id === undefined || id === null) continue;
    if (typeof id === "number" && !Number.isSafeInteger(id)) {
      throw new Error(`${key} must be an exact decimal string; unsafe numeric IDs are rejected.`);
    }
    result[key] = assetInteger(String(id)).value;
  }
  return result;
}

export async function readUnitySerializedProperty(
  workspaceRef: WorkspaceRef,
  request: UnitySerializedPropertyReadRequest,
): Promise<UnitySerializedPropertyReadResult> {
  return ipcInvoke<UnitySerializedPropertyReadResult>("unity_serialized_property_read", {
    workspaceRef,
    request: { ...request, target: exactUnityPropertyTarget(request.target) },
  });
}

export async function discoverUnitySerializedProperties(
  workspaceRef: WorkspaceRef,
  request: UnitySerializedPropertyDiscoverRequest,
): Promise<UnitySerializedPropertyDiscoverResult> {
  return ipcInvoke<UnitySerializedPropertyDiscoverResult>("unity_serialized_property_discover", {
    workspaceRef,
    request: { ...request, target: exactUnityPropertyTarget(request.target) },
  });
}

export async function writeUnitySerializedProperty(
  workspaceRef: WorkspaceRef,
  request: UnitySerializedPropertyWriteRequest,
): Promise<UnitySerializedPropertyWriteResult> {
  return ipcInvoke<UnitySerializedPropertyWriteResult>("unity_serialized_property_write", {
    workspaceRef,
    request: { ...request, target: exactUnityPropertyTarget(request.target) },
  });
}

export async function applyUnitySerializedProperties(
  workspaceRef: WorkspaceRef,
  request: UnitySerializedPropertyApplyRequest,
): Promise<UnitySerializedPropertyApplyResult> {
  return ipcInvoke<UnitySerializedPropertyApplyResult>("unity_serialized_property_apply", {
    workspaceRef,
    request: { ...request, writes: request.writes.map((write) => ({ ...write, target: exactUnityPropertyTarget(write.target) })) },
  });
}
