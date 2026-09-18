import { ipcInvoke } from "./ipc";
import type { WorkspaceRef } from "./project";
import { createUnityAssetBatch, type UnityAssetBatch } from "./unityAssetBatch";
export type { UnityAssetBatch } from "./unityAssetBatch";

export type AssetBackend = "yaml" | "live";
export interface AssetInteger { kind: "int64"; value: string; }
export interface AssetUnsignedInteger { kind: "uint64"; value: string; }
export type AssetValue = null | boolean | number | string | AssetInteger | AssetUnsignedInteger | AssetValue[] | { [key: string]: AssetValue };
/** bigint is accepted as input and encoded before crossing JSON IPC. */
export type AssetValueInput = AssetValue | bigint | AssetValueInput[] | { [key: string]: AssetValueInput };
export interface AssetField { property_path: string; kind: string; value: AssetValue; type_hint?: string; }
export interface AssetObject { object_id: string; class_id: string | null; root_type: string; fields: AssetField[]; }
export interface AssetDiagnostic { code?: string; message: string; severity?: string; [key: string]: unknown; }
export interface AssetSnapshot { revision: string; objects: AssetObject[]; diagnostics: AssetDiagnostic[]; backend?: AssetBackend; path?: string; }
export interface AssetFieldRef { object_id: string; property_path: string; }
export type AssetOperation = AssetFieldRef & (
  | { op: "set"; value: AssetValueInput }
  | { op: "array_insert"; index: number; value: AssetValueInput }
  | { op: "array_remove"; index: number }
  | { op: "array_move"; index: number; to_index: number }
  | { op: "array_resize"; size: number; value?: AssetValueInput }
);
export interface AssetReadOptions { object_id?: string; property_path?: string; }
export interface AssetDiscoverOptions extends AssetReadOptions { query?: string; offset?: number; limit?: number; }
export interface AssetDiscovery {
  revision: string;
  matches: (AssetField & { object_id: string })[];
  truncated: boolean;
  total: number;
  next_offset?: number | null;
}
export interface AssetPreviewOptions { expected_revision?: string; persist?: "disk"; }
export interface AssetApplyOptions { expected_revision: string; persist?: "disk"; }
export interface AssetEditResult {
  applied: boolean;
  persisted: boolean;
  snapshot: AssetSnapshot;
  operations_count: number;
  diagnostics: AssetDiagnostic[];
  transaction_id?: string | null;
}
export interface AssetBatchEntry { path: string; operations: AssetOperation[]; expected_revision: string; }
export interface AssetPreviewBatchEntry { path: string; operations: AssetOperation[]; expected_revision?: string; }
export interface AssetBatchResult {
  applied: boolean;
  persisted: boolean;
  results: (AssetEditResult & { path: string })[];
  transaction_id?: string | null;
}
export interface AssetRecovery { transaction_id: string; state: string; backend?: AssetBackend; }
export interface AssetCapabilities {
  backend: AssetBackend;
  supported_operations: AssetOperation["op"][];
  atomicity: "rollback_on_failure";
  multi_file_atomic_visibility: false;
  crash_recovery: boolean;
  durability: "journaled_per_file_replacement" | "editor_save_with_undo";
  persist_modes: "disk"[];
  starts_editor: boolean;
  read_source: "disk" | "editor";
  supported_extensions: string[];
  [key: string]: unknown;
}
export interface UnityAssets {
  batch(): UnityAssetBatch;
  backend(name: AssetBackend): UnityAssets;
  integer(value: string | bigint): AssetInteger;
  unsignedInteger(value: string | bigint): AssetUnsignedInteger;
  capabilities(): Promise<AssetCapabilities>;
  recover(transaction_id: string): Promise<AssetRecovery>;
  read(path: string, filters?: AssetReadOptions): Promise<AssetSnapshot>;
  discover(path: string, filters?: AssetDiscoverOptions): Promise<AssetDiscovery>;
  preview(path: string, operations: AssetOperation[], options?: AssetPreviewOptions): Promise<AssetEditResult>;
  apply(path: string, operations: AssetOperation[], options: AssetApplyOptions): Promise<AssetEditResult>;
  preview_batch(entries: AssetPreviewBatchEntry[]): Promise<AssetBatchResult>;
  apply_batch(entries: AssetBatchEntry[]): Promise<AssetBatchResult>;
}

const decimal = /^-?(?:0|[1-9][0-9]*)$/;

export function assetInteger(value: string | bigint): AssetInteger {
  const text = String(value);
  if ((typeof value !== "string" && typeof value !== "bigint") || !decimal.test(text) || text === "-0") {
    throw new Error("An asset integer must be a canonical decimal string or bigint.");
  }
  const integer = BigInt(text);
  if (integer < -(1n << 63n) || integer >= 1n << 63n) throw new Error("An asset integer must fit signed 64-bit range.");
  return { kind: "int64", value: text };
}

export function assetUnsignedInteger(value: string | bigint): AssetUnsignedInteger {
  const text = String(value);
  if ((typeof value !== "string" && typeof value !== "bigint") || !/^(?:0|[1-9][0-9]*)$/.test(text)) {
    throw new Error("An unsigned asset integer must be a canonical non-negative decimal string or bigint.");
  }
  if (BigInt(text) >= 1n << 64n) throw new Error("An unsigned asset integer must fit unsigned 64-bit range.");
  return { kind: "uint64", value: text };
}

function encodeValue(value: unknown, ancestors = new Set<object>()): AssetValue {
  if (value === null || typeof value === "string" || typeof value === "boolean") return value;
  if (typeof value === "bigint") return assetInteger(value);
  if (typeof value === "number") {
    if (!Number.isFinite(value)) throw new Error("Asset values cannot contain NaN or infinity.");
    if (Number.isInteger(value) && !Number.isSafeInteger(value)) throw new Error("Unsafe numeric integer; use assets.integer(decimalString) or bigint.");
    return value;
  }
  if (typeof value !== "object" || value === undefined) throw new Error("Asset values must be JSON-compatible values or bigint.");
  if (ancestors.has(value)) throw new Error("Asset values cannot contain cycles.");
  if (!Array.isArray(value) && Object.getPrototypeOf(value) !== Object.prototype && Object.getPrototypeOf(value) !== null) {
    throw new Error("Asset value objects must be plain objects.");
  }
  ancestors.add(value);
  try {
    if (Array.isArray(value)) return Array.from(value, (item) => encodeValue(item, ancestors));
    const object = value as Record<string, unknown>;
    if (object.kind === "int64" || object.kind === "uint64") {
      if (Object.keys(object).length !== 2 || typeof object.value !== "string") throw new Error("An integer value must contain only kind and a decimal string value.");
      return object.kind === "uint64" ? assetUnsignedInteger(object.value) : assetInteger(object.value);
    }
    const keys = Object.keys(object);
    const pointerKey = keys.includes("fileID") && keys.every((key) => ["fileID", "guid", "type"].includes(key)) ? "fileID"
      : keys.length === 1 && keys[0] === "rid" ? "rid" : null;
    if (pointerKey) {
      let id = object[pointerKey];
      if (typeof id === "number") {
        if (!Number.isSafeInteger(id)) throw new Error("Reference IDs must be exact decimal strings or bigint.");
        id = String(id);
      } else if (id && typeof id === "object" && "kind" in id && id.kind === "int64") {
        id = (encodeValue(id, ancestors) as AssetInteger).value;
      }
      if (typeof id !== "string" && typeof id !== "bigint") throw new Error("Reference IDs must be exact decimal strings or bigint.");
      const encoded = assetInteger(id).value;
      return Object.fromEntries(Object.entries(object).map(([key, item]) => [key, key === pointerKey ? encoded : encodeValue(item, ancestors)]));
    }
    return Object.fromEntries(Object.entries(object).map(([key, item]) => [key, encodeValue(item, ancestors)]));
  } finally { ancestors.delete(value); }
}

/** Shared exact JSON wire encoding; it does not interpret property semantics. */
export const encodeAssetValue = (value: unknown): AssetValue => encodeValue(value);

function assertRef(ref: AssetReadOptions) {
  if (ref.object_id !== undefined) {
    if (typeof ref.object_id !== "string") throw new Error("object_id must be an exact decimal string.");
    assetInteger(ref.object_id);
  }
  if (ref.property_path !== undefined && (typeof ref.property_path !== "string" || !ref.property_path.startsWith("/") || /~(?![01])/.test(ref.property_path))) {
    throw new Error("property_path must be a root-inclusive RFC 6901 pointer.");
  }
}

function encodeOperations(operations: AssetOperation[]): Record<string, unknown>[] {
  if (!Array.isArray(operations) || !operations.length) throw new Error("operations must be a non-empty array.");
  return operations.map((operation) => {
    if (!operation || typeof operation !== "object") throw new Error("Each asset operation must be an object.");
    const required: Record<string, string[]> = { set: ["value"], array_insert: ["index", "value"], array_remove: ["index"], array_move: ["index", "to_index"], array_resize: ["size"] };
    const fields = required[operation.op];
    const common = ["op", "object_id", "property_path"];
    const allowed = new Set([...common, ...(fields ?? []), ...(operation.op === "array_resize" ? ["value"] : [])]);
    if (!fields || [...common, ...fields].some((key) => !Object.prototype.hasOwnProperty.call(operation, key)) || Object.keys(operation).some((key) => !allowed.has(key))) {
      throw new Error(`Invalid fields for asset operation ${operation.op}.`);
    }
    if (typeof operation.object_id !== "string" || typeof operation.property_path !== "string") throw new Error("Operations require exact string object_id and property_path.");
    assertRef(operation);
    const source = operation as unknown as Record<string, unknown>;
    for (const key of fields.filter((field) => ["index", "to_index", "size"].includes(field))) {
      const value = source[key];
      if (typeof value !== "number" || !Number.isSafeInteger(value) || value < 0) throw new Error(`${key} must be a non-negative safe integer.`);
    }
    return Object.fromEntries(Object.entries(operation).map(([key, value]) => [key, key === "value" ? encodeValue(value) : value]));
  });
}

function assertRevision(revision: unknown) {
  if (typeof revision !== "string" || !revision.trim()) throw new Error("apply requires expected_revision from a current snapshot.");
}

function assertPath(path: unknown) {
  if (typeof path !== "string" || !path.trim()) throw new Error("path must name a project-relative Unity asset.");
}

/** Backend selection is immutable; every operation retains the caller's checkout
 * and lifecycle. Cancellation prevents new calls; it does not undo a dispatched write. */
export function createUnityAssets(workspaceRef: WorkspaceRef, options: { signal?: AbortSignal; backend?: AssetBackend } = {}): UnityAssets {
  const workspace = Object.freeze({ ...workspaceRef });
  const selected = options.backend ?? "yaml";
  if (selected !== "yaml" && selected !== "live") throw new Error("backend must be yaml or live.");
  function assertActive() {
    if (options.signal?.aborted) throw new Error("Frontend execution was cancelled.");
  }
  async function call<T>(action: string, request: Record<string, unknown> = {}): Promise<T> {
    assertActive();
    return ipcInvoke<T>("unity_assets_execute", { workspaceRef: workspace, request: { ...request, action, backend: selected } });
  }
  function batch(entries: AssetPreviewBatchEntry[], applying: boolean) {
    if (!Array.isArray(entries) || !entries.length) throw new Error("entries must be a non-empty array.");
    return entries.map((entry) => {
      assertPath(entry.path);
      if (applying) assertRevision(entry.expected_revision);
      return { path: entry.path, expected_revision: entry.expected_revision, operations: encodeOperations(entry.operations) };
    });
  }
  return {
    batch: () => createUnityAssetBatch(
      (entry) => batch([entry], true)[0] as unknown as AssetBatchEntry,
      (entries) => call<AssetBatchResult>("apply_batch", { entries, persist: "disk" }),
      assertActive,
    ),
    backend: (name: AssetBackend): UnityAssets => createUnityAssets(workspace, { ...options, backend: name }),
    integer: assetInteger,
    unsignedInteger: assetUnsignedInteger,
    capabilities: () => call<AssetCapabilities>("capabilities"),
    async recover(transaction_id: string): Promise<AssetRecovery> {
      if (typeof transaction_id !== "string" || !transaction_id.trim()) throw new Error("transaction_id must name a recorded asset transaction.");
      return call("recover", { transaction_id });
    },
    async read(path: string, filters: AssetReadOptions = {}): Promise<AssetSnapshot> {
      assertPath(path); assertRef(filters);
      return call("read", { path, object_id: filters.object_id, property_path: filters.property_path });
    },
    async discover(path: string, filters: AssetDiscoverOptions = {}): Promise<AssetDiscovery> {
      assertPath(path); assertRef(filters);
      const offset = filters.offset ?? 0; const limit = filters.limit ?? 100;
      if (!Number.isSafeInteger(offset) || offset < 0 || !Number.isSafeInteger(limit) || limit < 1) throw new Error("offset must be a non-negative integer and limit a positive integer.");
      return call("discover", { path, object_id: filters.object_id, property_path: filters.property_path, query: filters.query, offset, limit });
    },
    async preview(path: string, operations: AssetOperation[], editOptions: AssetPreviewOptions = {}): Promise<AssetEditResult> {
      assertPath(path);
      if (editOptions.persist !== undefined && editOptions.persist !== "disk") throw new Error("persist must be disk.");
      return call("preview", { path, expected_revision: editOptions.expected_revision, persist: "disk", operations: encodeOperations(operations) });
    },
    async apply(path: string, operations: AssetOperation[], editOptions: AssetApplyOptions): Promise<AssetEditResult> {
      assertPath(path); assertRevision(editOptions?.expected_revision);
      if (editOptions.persist !== undefined && editOptions.persist !== "disk") throw new Error("persist must be disk.");
      return call("apply", { path, expected_revision: editOptions.expected_revision, persist: "disk", operations: encodeOperations(operations) });
    },
    async preview_batch(entries: AssetPreviewBatchEntry[]): Promise<AssetBatchResult> {
      return call("preview_batch", { entries: batch(entries, false), persist: "disk" });
    },
    async apply_batch(entries: AssetBatchEntry[]): Promise<AssetBatchResult> {
      return call("apply_batch", { entries: batch(entries, true), persist: "disk" });
    },
  };
}
