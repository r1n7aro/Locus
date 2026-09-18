import { toRaw } from "vue";
import { createUnityPropertyRuntime, type UnityBoundPropertyRuntimeOptions, type UnityBoundPropertyTree, type UnityPropertyPathInput } from "../components/unity/unityPropertyBinding";
import { applyYamlProperties, applyYamlPropertySummary, readYamlProperty, discoverYamlProperties } from "./unityYamlProperty";
import { applyUnitySerializedProperties, discoverUnitySerializedProperties, readUnitySerializedProperty, writeUnitySerializedProperty,
  type UnitySerializedPropertyBackend, type UnitySerializedPropertyReadRequest, type UnitySerializedPropertyReadResult,
  type UnitySerializedPropertyDiscoverRequest, type UnitySerializedPropertyDiscoverResult, type UnitySerializedPropertyWriteRequest,
  type UnitySerializedPropertyWriteResult, type UnitySerializedPropertyApplyRequest, type UnitySerializedPropertyApplyResult,
  type UnitySerializedPropertyApplyWrite } from "./unitySerializedProperty";
import type { UnityYamlPropertySummaryRequest, UnityYamlPropertySummaryResult } from "./unitySerializedProperty";
import type { WorkspaceRef } from "./project";

export interface UnityPropertyBatch {
  readonly size: number;
  readonly state: "collecting" | "flushing" | "failed";
  enqueue(write: UnitySerializedPropertyApplyWrite): void;
  flush(): Promise<UnitySerializedPropertyApplyResult | null>;
  clear(): void;
}

export interface UnityPropertyApi {
  readonly mode: UnitySerializedPropertyBackend;
  backend(mode: UnitySerializedPropertyBackend): UnityPropertyApi;
  read(request: UnitySerializedPropertyReadRequest): Promise<UnitySerializedPropertyReadResult>;
  discover(request: UnitySerializedPropertyDiscoverRequest): Promise<UnitySerializedPropertyDiscoverResult>;
  write(request: UnitySerializedPropertyWriteRequest): Promise<UnitySerializedPropertyWriteResult>;
  apply(request: UnitySerializedPropertyApplyRequest): Promise<UnitySerializedPropertyApplyResult>;
  apply(request: UnityYamlPropertySummaryRequest): Promise<UnityYamlPropertySummaryResult>;
  readTree(target: UnityPropertyPathInput, options?: UnityBoundPropertyRuntimeOptions): Promise<UnityBoundPropertyTree>;
  batch(): UnityPropertyBatch;
}

/** Backend is selected once by the View author. Trees and batches retain it. */
export function createUnityPropertyApi(workspaceRef: WorkspaceRef, options: { backend?: UnitySerializedPropertyBackend; signal?: AbortSignal } = {}): UnityPropertyApi {
  const workspace = Object.freeze({ ...workspaceRef });
  const mode = options.backend ?? "live";
  if (mode !== "yaml" && mode !== "live") throw new Error("Property backend must be yaml or live.");
  function active() {
    if (options.signal?.aborted) throw new Error("Frontend execution was cancelled.");
  }
  function apply(request: UnitySerializedPropertyApplyRequest): Promise<UnitySerializedPropertyApplyResult>;
  function apply(request: UnityYamlPropertySummaryRequest): Promise<UnityYamlPropertySummaryResult>;
  async function apply(request: UnitySerializedPropertyApplyRequest | UnityYamlPropertySummaryRequest): Promise<UnitySerializedPropertyApplyResult | UnityYamlPropertySummaryResult> {
    active();
    if (request.resultMode === "summary") {
      if (mode !== "yaml") throw new Error("Summary results require the YAML property backend.");
      return applyYamlPropertySummary(workspace, request, options.signal);
    }
    return mode === "yaml" ? applyYamlProperties(workspace, request, options.signal) : applyUnitySerializedProperties(workspace, request);
  }
  const api: UnityPropertyApi = {
    mode,
    backend: (backend) => createUnityPropertyApi(workspace, { ...options, backend }),
    async read(request) {
      active();
      return mode === "yaml" ? readYamlProperty(workspace, request) : readUnitySerializedProperty(workspace, request);
    },
    async discover(request) {
      active();
      if (mode === "yaml") return discoverYamlProperties(workspace, request);
      return discoverUnitySerializedProperties(workspace, request);
    },
    async write(request) {
      active();
      if (mode === "live") return writeUnitySerializedProperty(workspace, request);
      return (await applyYamlProperties(workspace, { writes: [request] }, options.signal)).results[0]!;
    },
    apply,
    async readTree(target, treeOptions) {
      active();
      // Per-tree revision ownership: reading another tree cannot make stale
      // data in this tree appear current. Revisions travel with every commit.
      let revision: string | undefined;
      let dependencies: Record<string, string> | undefined;
      let rootPath: string | undefined;
      const runtime = createUnityPropertyRuntime({
        read: async (request) => {
          const result = await api.read(request);
          const path = request.target.propertyPath ?? "";
          const dependencyChanged = dependencies && (Object.keys(dependencies).length !== Object.keys(result.dependencies ?? {}).length
            || Object.entries(dependencies).some(([path, version]) => result.dependencies?.[path] !== version));
          if (revision && (result.revision !== revision || dependencyChanged) && (path !== rootPath || (request.arrayOffset ?? 0) > 0)) {
            throw new Error("assets.stale_revision: refresh the tree before loading more children.");
          }
          rootPath ??= path;
          revision = result.revision;
          dependencies = result.dependencies;
          return result;
        },
        write: async (request) => {
          active();
          if (mode === "yaml" && request.writeMode === "preview") {
            // The number editor already renders its drag value locally.
            return { ok: true, saved: false, message: "", target: request.target, propertyPath: request.target.propertyPath ?? "", value: request.value };
          }
          const result = await api.write({ ...request, expectedRevision: revision, expectedDependencies: dependencies });
          if (result.ok && result.saved) { revision = result.revision; dependencies = result.dependencies; }
          return result;
        },
        apply: async (request) => {
          const result = await api.apply({ writes: request.writes.map((write) => ({ ...write, expectedRevision: revision, expectedDependencies: dependencies })) });
          if (result.ok) { revision = result.results[0]?.revision ?? revision; dependencies = result.results[0]?.dependencies ?? dependencies; }
          return result;
        },
      });
      return runtime.readTree(target, treeOptions);
    },
    batch: () => createPropertyBatch(api, active),
  };
  return api;
}

function copyRequest<T>(value: T, ancestors = new Set<object>()): T {
  if (typeof value === "function" || typeof value === "symbol") throw new Error("Property requests must contain serializable values.");
  if (value === null || typeof value !== "object") return value;
  // Values and targets commonly come from Vue refs; structuredClone rejects proxies.
  const raw = toRaw(value);
  if (ancestors.has(raw)) throw new Error("Property requests cannot contain cycles.");
  if (!Array.isArray(raw) && ![Object.prototype, null].includes(Object.getPrototypeOf(raw))) throw new Error("Property request objects must be plain objects.");
  ancestors.add(raw);
  try {
    return (Array.isArray(raw) ? raw.map((item) => copyRequest(item, ancestors))
      : Object.fromEntries(Object.entries(raw).map(([key, item]) => [key, copyRequest(item, ancestors)]))) as T;
  } finally { ancestors.delete(raw); }
}

function createPropertyBatch(api: UnityPropertyApi, active: () => void): UnityPropertyBatch {
  const writes: UnitySerializedPropertyApplyWrite[] = [];
  let state: UnityPropertyBatch["state"] = "collecting";
  let pending: Promise<UnitySerializedPropertyApplyResult | null> | null = null;
  function collecting() {
    active();
    if (state !== "collecting") throw new Error(`Property batch is ${state}; ${state === "failed" ? "inspect the outcome, then clear and rebuild from fresh reads" : "wait for flush"}.`);
  }
  return {
    get size() { return writes.length; },
    get state() { return state; },
    enqueue(write) {
      collecting();
      if (writes.length >= 10_000) throw new Error("A property batch supports at most 10000 writes.");
      if (write.writeMode && write.writeMode !== "commit") throw new Error("Batches accept committed edits only; keep previews local.");
      if (api.mode === "yaml" && !write.expectedRevision?.trim()) throw new Error("YAML writes require expectedRevision from a YAML read.");
      writes.push(copyRequest(write));
    },
    flush() {
      if (pending) return pending;
      try { collecting(); } catch (error) { return Promise.reject(error); }
      if (!writes.length) return Promise.resolve(null);
      state = "flushing";
      pending = Promise.resolve().then(() => api.apply({ writes: [...writes] })).then((result) => {
        if (!result.ok || result.results.length !== writes.length || result.results.some((item) => !item.ok || !item.saved)) throw new Error(result.message || "Some property writes failed; inspect the outcome before retrying.");
        writes.length = 0; state = "collecting";
        return result;
      }).catch((error) => { state = "failed"; throw error; }).finally(() => { pending = null; });
      return pending;
    },
    clear() {
      if (state === "flushing") throw new Error("Cannot clear an in-flight property batch.");
      writes.length = 0; state = "collecting";
    },
  };
}
