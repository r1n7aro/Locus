import { beforeEach, describe, expect, it, vi } from "vitest";
import { reactive } from "vue";
vi.mock("../services/ipc", () => ({ ipcInvoke: vi.fn() }));
import { ipcInvoke } from "../services/ipc";
import { createUnityPropertyApi } from "../services/unityPropertyApi";
import { createUnityAssets } from "../services/unityAssets";

const invoke = vi.mocked(ipcInvoke);
const workspace = { checkoutId: "property-backends", expectedGeneration: 4, expectedMaterializationEpoch: 8 };
const target = { kind: "asset", path: "Assets/Data.asset", targetFileId: "9007199254740993", propertyPath: "amount" };
function result(value = 7, revision = "r1", propertyPath = "amount") {
  return { ok: true, saved: true, message: "", backend: "yaml", revision, target: { ...target, propertyPath },
    bindingTarget: { ...target, propertyPath }, propertyPath, name: propertyPath, type: "Integer", valueType: "Integer", value, editable: true, children: [] };
}
function root(revision = "r1", amount = 7) {
  return { ...result(0, revision, ""), type: "Object", valueType: "Object", hasChildren: true, children: [result(amount, revision),
    { ...result(0, revision, "nested"), type: "Generic", valueType: "Generic", hasChildren: true, childrenTruncated: true }] };
}
const applied = (values = [9]) => ({ ok: true, message: "", results: values.map((value) => result(value, "r2")) });
beforeEach(() => { invoke.mockReset(); });

describe("Property API transport and backend ownership", () => {
  it("supports compact YAML commit receipts without weakening exact IDs or revisions", async () => {
    const summary = { ok: true, message: "", writesApplied: 1, transactionId: "tx", assets: [{ path: "Assets/Data.asset", revision: "r2", dependencies: { "Assets/Data.asset": "r2" } }] };
    invoke.mockResolvedValue(summary);
    const yaml = createUnityPropertyApi(workspace).backend("yaml");
    expect(await yaml.apply({ writes: [{ target, value: 9, expectedRevision: "r1" }], resultMode: "summary", profile: true })).toBe(summary);
    expect((invoke.mock.calls[0]![1] as any).request).toMatchObject({ resultMode: "summary", profile: true, writes: [{ target, expectedRevision: "r1" }] });
    await expect(yaml.backend("live").apply({ writes: [], resultMode: "summary" })).rejects.toThrow(/YAML/);
    expect(invoke).toHaveBeenCalledTimes(1);
  });
  it("passes optional YAML profiling through without changing snapshots", async () => {
    const profile = { prepareMs: 10, commitMs: 20, projectionMs: 3, totalMs: 33, overrideValidationPasses: 1 };
    invoke.mockResolvedValue({ ...applied(), profile });
    const result = await createUnityPropertyApi(workspace).backend("yaml").apply({ writes: [{ target, value: 9, expectedRevision: "r1" }], profile: true });
    expect(result.profile).toBe(profile);
    expect((invoke.mock.calls[0]![1] as any).request.profile).toBe(true);
  });
  it("selects once on an immutable API instance and retains workspace scope", async () => {
    const input = { ...workspace };
    const live = createUnityPropertyApi(input);
    const yaml = live.backend("yaml"); input.checkoutId = "changed";
    invoke.mockResolvedValue(result());
    expect(live.mode).toBe("live"); expect(yaml.mode).toBe("yaml");
    await yaml.read({ target });
    expect(invoke).toHaveBeenLastCalledWith("unity_assets_execute", { workspaceRef: workspace, request: { target, action: "read_property", backend: "yaml" } });
    await live.read({ target });
    expect(invoke).toHaveBeenLastCalledWith("unity_serialized_property_read", { workspaceRef: workspace, request: { target } });
    expect(() => live.backend("automatic" as never)).toThrow(/backend/);
  });

  it("forwards budgets and shared graph snapshots without frontend projection", async () => {
    const snapshot = { ...result(), value: { rid: "9223372036854775807" }, canonicalPath: "Assets/Data.asset/node", children: [result()] };
    invoke.mockResolvedValue(snapshot);
    const request = { target, arrayOffset: 1024, maxArrayItems: 64, maxDepth: 4 };
    expect(await createUnityPropertyApi(workspace).backend("yaml").read(request)).toBe(snapshot);
    expect(invoke).toHaveBeenCalledWith("unity_assets_execute", { workspaceRef: workspace, request: { ...request, action: "read_property", backend: "yaml" } });
  });

  it("discovers exact inherited targets through the shared backend", async () => {
    const response = { ok: true, message: "", target, matches: [{ target }], dependencies: { "Assets/Base.prefab": "r1" } };
    invoke.mockResolvedValue(response);
    expect(await createUnityPropertyApi(workspace).backend("yaml").discover({ target, query: "amount" })).toBe(response);
    expect((invoke.mock.calls[0]![1] as any).request.action).toBe("discover_property");
  });

  it("rejects partial expansion when only a prefab source revision changed", async () => {
    const yaml = createUnityPropertyApi(workspace).backend("yaml");
    invoke.mockResolvedValue({ ...root(), dependencies: { "Assets/Base.prefab": "source1" } });
    const tree = await yaml.readTree({ ...target, propertyPath: "" });
    invoke.mockResolvedValue({ ...result(0, "r1", "nested"), dependencies: { "Assets/Base.prefab": "source2" } });
    await expect(tree.loadChildren(tree.require("nested").raw)).rejects.toThrow(/refresh the tree/);
    invoke.mockResolvedValue(applied()); await tree.require("amount").write(9, { refresh: false });
    expect((invoke.mock.calls[invoke.mock.calls.length - 1]![1] as any).request.writes[0].expectedDependencies).toEqual({ "Assets/Base.prefab": "source1" });
  });

  it("sends logical paths to Rust in one call, preserving exact values and IDs", async () => {
    invoke.mockResolvedValue(applied());
    const request = { target: { ...target, propertyPath: "node.next.amount" }, value: 9223372036854775806n, expectedRevision: "r1" };
    await createUnityPropertyApi(workspace).backend("yaml").write(request);
    expect(invoke).toHaveBeenCalledExactlyOnceWith("unity_assets_execute", { workspaceRef: workspace, request: {
      action: "apply_properties", backend: "yaml", writes: [{ ...request, value: { kind: "int64", value: "9223372036854775806" } }],
    } });
  });

  it("propagates semantic rejections without dispatching a live fallback", async () => {
    invoke.mockRejectedValue(new Error("property.unsupported_command: setType"));
    await expect(createUnityPropertyApi(workspace).backend("yaml").write({ target, value: { action: "setType" }, expectedRevision: "r1" })).rejects.toThrow(/unsupported_command/);
    expect(invoke).toHaveBeenCalledTimes(1);
    expect(invoke.mock.calls[0]![0]).toBe("unity_assets_execute");
  });

  it("rejects rounded identity and numeric values before IPC", async () => {
    const yaml = createUnityPropertyApi(workspace).backend("yaml");
    await expect(yaml.read({ target: { ...target, targetFileId: Number("9007199254740993") } })).rejects.toThrow(/exact/);
    await expect(yaml.write({ target, value: Number("9007199254740993"), expectedRevision: "r1" })).rejects.toThrow(/Unsafe/);
    expect(invoke).not.toHaveBeenCalled();
  });

  it("binds a tree to its backend and keeps numeric drag previews local", async () => {
    invoke.mockResolvedValue(root());
    const tree = await createUnityPropertyApi(workspace).backend("yaml").readTree({ ...target, propertyPath: "" });
    invoke.mockClear();
    await tree.writeProperty(tree.require("amount").raw, 8, {}, "preview");
    expect(invoke).not.toHaveBeenCalled();
    invoke.mockResolvedValueOnce(applied()).mockResolvedValueOnce(root("r2", 9));
    await tree.require("amount").write(9);
    expect(tree.require("amount").value).toBe(9);
    expect((invoke.mock.calls[0]![1] as any).request.writes[0].expectedRevision).toBe("r1");
    expect(invoke.mock.calls.every(([command]) => command === "unity_assets_execute")).toBe(true);
  });

  it("does not adopt another tree's revision or advance it during partial expansion", async () => {
    const yaml = createUnityPropertyApi(workspace).backend("yaml");
    invoke.mockResolvedValue(root()); const first = await yaml.readTree({ ...target, propertyPath: "" });
    invoke.mockResolvedValue(root("r2")); await yaml.readTree(target);
    invoke.mockResolvedValue(result(0, "r2", "nested"));
    await expect(first.loadChildren(first.require("nested").raw)).rejects.toThrow(/refresh the tree/);
    invoke.mockResolvedValue(applied());
    await first.require("amount").write(9, { refresh: false });
    expect((invoke.mock.calls[invoke.mock.calls.length - 1]![1] as any).request.writes[0].expectedRevision).toBe("r1");
  });

  it("prevents new operations after cancellation", async () => {
    const controller = new AbortController();
    const yaml = createUnityPropertyApi(workspace, { signal: controller.signal }).backend("yaml");
    controller.abort();
    await expect(yaml.write({ target, value: 9, expectedRevision: "r1" })).rejects.toThrow(/cancelled/);
    expect(invoke).not.toHaveBeenCalled();
  });
});

describe("Deferred property transactions", () => {
  it("captures prefab source revisions and authoring commands in one immutable batch", async () => {
    const batch = createUnityPropertyApi(workspace).backend("yaml").batch();
    const dependencies = { "Assets/Base.prefab": "base-v1" };
    batch.enqueue({ target, value: { action: "revert" }, expectedRevision: "r1", expectedDependencies: dependencies });
    batch.enqueue({ target, value: { action: "applyToSource", level: 1 }, expectedRevision: "r1", expectedDependencies: dependencies });
    dependencies["Assets/Base.prefab"] = "base-v2";
    invoke.mockResolvedValue(applied([9, 9])); await batch.flush();
    const writes = (invoke.mock.calls[0]![1] as any).request.writes;
    expect(writes.map((w: any) => w.value.action)).toEqual(["revert", "applyToSource"]);
    expect(writes[0].expectedDependencies).toEqual({ "Assets/Base.prefab": "base-v1" });
    expect(invoke).toHaveBeenCalledTimes(1);
  });
  it("accumulates immutable requests and shares concurrent flush calls", async () => {
    const batch = createUnityPropertyApi(workspace).backend("yaml").batch();
    const write = { target: { ...target }, value: 8, expectedRevision: "r1" };
    batch.enqueue(write); write.value = 100;
    batch.enqueue({ target, value: 9, expectedRevision: "r1" });
    expect(batch.size).toBe(2); expect(invoke).not.toHaveBeenCalled();
    invoke.mockResolvedValue(applied([9, 9]));
    const first = batch.flush(); expect(batch.flush()).toBe(first);
    expect(() => batch.clear()).toThrow(/in-flight/);
    expect(() => batch.enqueue(write)).toThrow(/flushing/);
    await first;
    expect((invoke.mock.calls[0]![1] as any).request.writes.map((write: any) => write.value)).toEqual([8, 9]);
    expect(invoke).toHaveBeenCalledTimes(1); expect(batch.size).toBe(0);
    expect(await batch.flush()).toBeNull();
  });

  it("never replays after an uncertain commit, partial success or missing results", async () => {
    const batch = createUnityPropertyApi(workspace).batch();
    for (const response of [new Error("outcome_unknown"), { ok: false, message: "partial", results: [] }, { ok: true, message: "", results: [] }]) {
      batch.clear(); batch.enqueue({ target, value: 9 });
      if (response instanceof Error) invoke.mockRejectedValueOnce(response); else invoke.mockResolvedValueOnce(response);
      await expect(batch.flush()).rejects.toThrow();
      const count = invoke.mock.calls.length;
      await expect(batch.flush()).rejects.toThrow(/failed/);
      expect(invoke).toHaveBeenCalledTimes(count); expect(batch.size).toBe(1);
    }
  });

  it("retains returned removal results without reconstructing a deleted field", async () => {
    const removed = { ...result(), value: null, message: "Property was removed by a later write in this batch." };
    invoke.mockResolvedValue({ ok: true, message: "", results: [removed] });
    const returned = await createUnityPropertyApi(workspace).backend("yaml").write({ target, value: 1, expectedRevision: "r1" });
    expect(returned).toBe(removed);
  });

  it("snapshots Vue reactive targets and values at enqueue time", async () => {
    const batch = createUnityPropertyApi(workspace).backend("yaml").batch();
    const write = reactive({ target: { ...target, propertyPath: "nested" }, value: { enabled: false, label: "draft" }, expectedRevision: "r1" });
    batch.enqueue(write); write.value.label = "later"; write.target.propertyPath = "amount";
    invoke.mockResolvedValue(applied()); await batch.flush();
    const captured = (invoke.mock.calls[0]![1] as any).request.writes[0];
    expect(captured.target.propertyPath).toBe("nested"); expect(captured.value).toEqual({ enabled: false, label: "draft" });
  });

  it("preserves raw asset operation ordering without property conversion", async () => {
    const batch = createUnityAssets(workspace).backend("yaml").batch();
    const field = { object_id: target.targetFileId, property_path: "/MonoBehaviour/values" };
    batch.enqueue(target.path, [{ ...field, op: "array_insert", index: 1, value: 5 }], { expected_revision: "r1" });
    batch.enqueue(target.path, [{ ...field, op: "array_move", index: 1, to_index: 0 }], { expected_revision: "r1" });
    expect(invoke).not.toHaveBeenCalled();
    invoke.mockResolvedValue({ applied: true, persisted: true, results: [{}] }); await batch.flush();
    expect((invoke.mock.calls[0]![1] as any).request.entries[0].operations.map((op: any) => op.op)).toEqual(["array_insert", "array_move"]);
    expect(invoke).toHaveBeenCalledTimes(1); expect(batch.operationCount).toBe(0);
  });
});
