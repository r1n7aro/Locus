// @vitest-environment jsdom
// Regression assertions for the Unity property editing contract.
import { afterEach, describe, expect, it, vi } from "vitest";
import { createApp, h, nextTick, ref, type App } from "vue";
import { createPinia } from "pinia";
import { createUnityPropertyRuntime } from "../components/unity/unityPropertyBinding";
import { compileViewPackageSource } from "../components/view/viewCompilationCore";
import { createViewRuntimeComponent, type ViewRuntimeApi } from "../components/view/viewRuntime";
import { createViewExecutionScope } from "../components/view/viewExecutionScope";
import SerializedTableView from "../components/table/SerializedTableView.vue";
import type { ViewPackageDetail } from "../services/view";
import type { SerializedTableRow } from "../components/table/serializedTable";
import type { UnityValueEditorCommittedEvent } from "../services/unityValueEditorWindow";
const externalWrites = vi.hoisted(() => ({ listeners: new Set<(event: UnityValueEditorCommittedEvent) => void>() }));
vi.mock("../services/unityValueEditorWindow", async original => ({
  ...await original<typeof import("../services/unityValueEditorWindow")>(),
  listenUnityValueEditorCommitted: async (handler: (event: UnityValueEditorCommittedEvent) => void) => {
    externalWrites.listeners.add(handler); return () => externalWrites.listeners.delete(handler);
  },
}));
vi.mock("@tauri-apps/api/event", async original => ({ ...await original<typeof import("@tauri-apps/api/event")>(), emit: vi.fn(async () => {}) }));

const apps: App[] = [];
const scopes: ReturnType<typeof createViewExecutionScope>[] = [];
afterEach(() => { apps.splice(0).forEach(app => app.unmount()); scopes.splice(0).forEach(scope => scope.dispose()); document.body.innerHTML = ""; });
const target = { kind: "asset", path: "Assets/Probe.asset", propertyPath: "speed" };
const snapshot = (value: number) => ({ ok: true, message: "ok", target, propertyPath: "speed", name: "speed", displayName: "Speed", type: "Float", valueType: "Float", value, displayValue: String(value), editable: true, children: [] });
const flush = async () => { for (let i = 0; i < 20; i++) await Promise.resolve(); await nextTick(); };
function runtime(overrides: Record<string, unknown>) {
  const main = 'import { property, undo } from "@locus/view-runtime"; console.log("review-api", property, undo); export default { render: () => null };';
  const pkg: ViewPackageDetail = {
    summary: { id: "review", packageRoot: "/review", packageRelPath: "", name: "review", apiVersion: "1", version: "1", displayPath: "", manifestPath: "", updatedAt: 0, capabilities: { unity: true }, requirements: { unityConnection: true } },
    manifest: { schema: "locus.view.v1", apiVersion: "1", version: "1", id: "review", name: "review", entry: "main.ts", scripts: [], capabilities: { unity: true }, requirements: { unityConnection: true } },
    files: [{ relPath: "main.ts", content: main, kind: "source", size: main.length, truncated: false }],
  };
  let captured: any[] = [];
  const scope = createViewExecutionScope({ viewId: "review", editorId: "review", workspaceRef: { checkoutId: "review", expectedGeneration: 1 }, active: ref(true), state: new Map(), log: (_level, args) => { if (args[0] === "review-api") captured = args; } });
  scopes.push(scope);
  const api = new Proxy({ workspaceRef: { checkoutId: "review", expectedGeneration: 1 }, ...overrides }, { get: (obj, key) => key in obj ? obj[key as keyof typeof obj] : vi.fn(async () => null) }) as unknown as ViewRuntimeApi;
  const compilation = compileViewPackageSource({ id: 1, key: "review", scopeId: "review", detail: pkg });
  createViewRuntimeComponent({ detail: pkg, api, compilation, scope });
  return { property: captured[1], undo: captured[2] };
}

describe("Unity property editing regressions", () => {
  it("submits one array command per click", async () => {
    let size = 2;
    let writes = 0;
    let reads = 0;
    const arraySnapshot = () => ({ ok: true, message: "ok", target: { ...target, propertyPath: "items" }, propertyPath: "items", type: "Generic", valueType: "Generic", displayName: "Items", editable: true, isArray: true, arraySize: size, children: [] });
    const property = createUnityPropertyRuntime({ read: async () => { reads++; return arraySnapshot(); }, write: async () => { writes++; size++; return { ...arraySnapshot(), saved: true }; }, apply: async () => ({ ok: true, message: "ok", results: [] }) });
    const tree = await property.fromPath({ ...target, propertyPath: "items" });
    const root = document.createElement("div"); document.body.append(root);
    const app = createApp({ render: () => tree.drawDefaultEditor() }); app.use(createPinia()); apps.push(app); app.mount(root);
    (root.querySelector(".array-add-button") as HTMLButtonElement).click();
    await flush();
    expect({ writes, reads, size }).toEqual({ writes: 1, reads: 2, size: 3 });
  });

  it("refreshes bound snapshots after undo and records the next edit from the restored value", async () => {
    let value = 1;
    const write = vi.fn(async (request: any) => { value = request.value?.action === "restoreSnapshot" ? request.value.snapshot.value : request.value; return { ...snapshot(value), saved: true }; });
    const { property, undo } = runtime({ unityPropertyRead: async () => snapshot(value), unityPropertyWrite: write });
    const tree = await property.fromPath(target);
    await tree.require("speed").write(2);
    expect(tree.require("speed").value).toBe(2);
    await undo.undo();
    expect(value).toBe(1);
    expect(tree.require("speed").value).toBe(1);
    await tree.require("speed").write(3);
    await undo.undo();
    expect(value).toBe(1);
  });

  it("records only successful writes and keeps undo available when restoring fails", async () => {
    const calls: any[] = [];
    const good = { ...snapshot(2), saved: true };
    const failed = { ...snapshot(1), target: { ...target, propertyPath: "missing" }, ok: false, saved: false, displayValue: "", value: null };
    const { property, undo } = runtime({ unityPropertyRead: async () => snapshot(1), unityPropertyApply: async (request: any) => { calls.push(request); return { ok: false, message: "Some bindings failed", results: [good, failed] }; } });
    await expect(property.apply([{ target, value: 2 }, { target: failed.target, value: 3 }])).rejects.toThrow();
    await expect(undo.undo()).rejects.toThrow();
    expect(calls[1].writes).toHaveLength(1);
    expect(calls[1].writes[0].target.propertyPath).toBe("speed");
    expect(undo.state.canUndo).toBe(true);
    expect(undo.state.canRedo).toBe(false);
  });

  it("renders every root of a multi-root response", async () => {
    const property = createUnityPropertyRuntime({ read: async () => ({ ...snapshot(1), properties: [snapshot(1), { ...snapshot(2), propertyPath: "other", displayName: "Other" }] }), write: async () => ({ ...snapshot(1), saved: true }), apply: async () => ({ ok: true, message: "ok", results: [] }) });
    const tree = await property.fromPath(target);
    expect(tree.properties).toHaveLength(2);
    const root = document.createElement("div"); document.body.append(root);
    const app = createApp({ render: () => tree.drawDefaultEditor() }); app.use(createPinia()); apps.push(app); app.mount(root);
    expect(root.textContent).toContain("Speed");
    expect(root.textContent).toContain("Other");
    expect(root.querySelectorAll("input")).toHaveLength(2);
  });

  it("routes identical component property paths to the root that emitted the edit", async () => {
    const writes: any[] = [];
    const roots = ["first", "second"].map((id) => ({
      propertyPath: id, displayName: id, valueType: "Object", editable: false,
      bindingTarget: { ...target, globalObjectId: id },
      children: [{ ...snapshot(0), target: undefined, propertyPath: "m_Enabled", valueType: "Boolean", value: false }],
    }));
    const property = createUnityPropertyRuntime({
      read: async () => ({ ...snapshot(1), properties: roots }),
      write: async (request) => { writes.push(request); return { ...snapshot(1), saved: true }; },
      apply: async () => ({ ok: true, message: "ok", results: [] }),
    });
    const tree = await property.fromPath(target);
    const root = document.createElement("div"); document.body.append(root);
    const app = createApp({ render: () => tree.drawDefaultEditor() }); app.use(createPinia()); apps.push(app); app.mount(root);
    const checkbox = root.querySelector<HTMLInputElement>('input[type="checkbox"]')!;
    checkbox.checked = true; checkbox.dispatchEvent(new Event("change", { bubbles: true })); await flush();
    expect(writes).toHaveLength(1);
    expect(writes[0].target.globalObjectId).toBe("first");
  });

  it("records the authoritative before snapshot returned by the write", async () => {
    let value = 2;
    const { property, undo } = runtime({
      unityPropertyRead: async () => snapshot(1),
      unityPropertyWrite: async (request: any) => {
        const beforeSnapshot = snapshot(value);
        value = request.value?.action === "restoreSnapshot" ? request.value.snapshot.value : request.value;
        return { ...snapshot(value), beforeSnapshot, saved: true };
      },
    });
    const bound = await property.readProperty(target);
    await bound.write(3, { refresh: false }); await undo.undo();
    expect(value).toBe(2);
  });

  it("waits for an in-flight commit before undoing the newest change", async () => {
    let value = 1;
    let finish!: () => void;
    const pending = new Promise<void>((resolve) => { finish = resolve; });
    const { property, undo } = runtime({
      unityPropertyRead: async () => snapshot(value),
      unityPropertyWrite: async (request: any) => {
        const beforeSnapshot = snapshot(value);
        if (request.value === 2) await pending;
        value = request.value?.action === "restoreSnapshot" ? request.value.snapshot.value : request.value;
        return { ...snapshot(value), beforeSnapshot, saved: true };
      },
    });
    const change = property.write(target, 2); await flush();
    const revert = undo.undo(); finish(); await change; await revert;
    expect(value).toBe(1);
  });

  it("adds an owned value-editor commit to the View undo history", async () => {
    let value = 2;
    const { undo } = runtime({ unityPropertyWrite: async (request: any) => {
      value = request.value.snapshot.value; return { ...snapshot(value), saved: true };
    } });
    await flush();
    const event: UnityValueEditorCommittedEvent = {
      kind: "curve", workspaceRef: { checkoutId: "review", expectedGeneration: 1 }, historyOwner: "main:review:review",
      target, propertyPath: "speed", value: 2, result: { ...snapshot(2), beforeSnapshot: snapshot(1), saved: true },
    };
    for (const listener of externalWrites.listeners) listener(event);
    await flush(); await undo.undo();
    expect(value).toBe(1); expect(undo.state.canRedo).toBe(true);
  });

  it("offers an explicit continuation for a truncated array", async () => {
    const read = vi.fn(async (request: any) => { const start = request.arrayOffset ?? 0; const count = Math.min(100 - start, request.maxArrayItems ?? 64); return { ...snapshot(0), propertyPath: "items", type: "Generic", valueType: "Generic", displayName: "Items", isArray: true, arraySize: 100, childrenTruncated: start + count < 100, children: Array.from({ length: count }, (_, i) => ({ ...snapshot(i + start), propertyPath: `items.Array.data[${i + start}]` })) }; });
    const property = createUnityPropertyRuntime({ read, write: async () => ({ ...snapshot(1), saved: true }), apply: async () => ({ ok: true, message: "ok", results: [] }) });
    const tree = await property.fromPath(target);
    const root = document.createElement("div"); document.body.append(root);
    const app = createApp({ render: () => tree.drawDefaultEditor() }); app.use(createPinia()); apps.push(app); app.mount(root);
    expect(root.querySelectorAll(".array-item")).toHaveLength(0);
    (root.querySelector(".array-fold-button") as HTMLButtonElement).click(); await flush();
    expect(root.querySelectorAll(".array-item")).toHaveLength(64);
    const more = root.querySelector<HTMLButtonElement>(".property-load-more");
    expect(more).not.toBeNull(); more!.click(); await flush();
    expect(root.querySelectorAll(".array-item")).toHaveLength(100);
    expect(read).toHaveBeenCalledTimes(2);
  });

  it("continues an array beyond the per-request 1024 element budget", async () => {
    const read = vi.fn(async (request: any) => {
      const start = request.arrayOffset ?? 0;
      const count = Math.min(1100 - start, request.maxArrayItems ?? 64, 1024);
      return { ...snapshot(0), propertyPath: "items", valueType: "Generic", isArray: true, arraySize: 1100, childrenTruncated: start + count < 1100,
        children: Array.from({ length: count }, (_, i) => ({ ...snapshot(i + start), propertyPath: `items.Array.data[${i + start}]` })) };
    });
    const property = createUnityPropertyRuntime({ read, write: vi.fn(), apply: vi.fn() });
    const tree = await property.fromPath(target, { maxArrayItems: 1024 });
    await tree.loadChildren(tree.require("items").raw);
    expect(tree.require("items").raw.children).toHaveLength(1100);
    expect(tree.require("items.Array.data[1099]").value).toBe(1099);
    expect(read.mock.calls[1][0].arrayOffset).toBe(1024);
  });

  it("bounds mounted editors for a 200 by 8 serialized table", async () => {
    const columns = Array.from({ length: 8 }, (_, i) => ({ id: `c${i}`, propertyPath: `c${i}`, label: `C${i}` }));
    const rows: SerializedTableRow[] = Array.from({ length: 200 }, (_, i) => ({ id: `r${i}`, sourceKind: "asset", typeName: "ReviewData", label: `R${i}`, assetPath: `Assets/R${i}.asset`, status: "ok", message: "", cells: columns.map(c => ({ ...snapshot(i), columnId: c.id, propertyPath: c.propertyPath, label: c.label, fieldTypeFullName: "System.Single", fieldTypeAssembly: "mscorlib", referenceTypeFullName: "", referenceTypeAssembly: "", hasChildren: false, isArray: false, arraySize: -1, isFlagsEnum: false, enumValueIndex: -1, enumValueFlag: 0, enumOptions: [], isManagedReference: false, managedReferenceFullTypename: "", managedReferenceFieldTypename: "", managedReferenceDisplayName: "", managedReferenceTypes: [] })) }));
    const root = document.createElement("div"); document.body.append(root);
    const app = createApp({ render: () => h(SerializedTableView, { columns, rows }) }); app.use(createPinia()); apps.push(app); app.mount(root);
    await nextTick();
    expect(root.querySelectorAll(".unity-property-editor").length).toBeLessThan(600);
    const scroller = root.querySelector<HTMLElement>(".locus-serialized-table-scroller")!;
    const focused = root.querySelector<HTMLInputElement>("input")!; focused.focus();
    scroller.scrollTop = 12; scroller.dispatchEvent(new Event("scroll")); await flush();
    expect(document.activeElement).toBe(focused);
    scroller.scrollTop = 10000; scroller.dispatchEvent(new Event("scroll")); await flush();
    expect(root.textContent).toContain("R199");
  });
});


