import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("../services/ipc", () => ({ ipcInvoke: vi.fn() }));
import { ipcInvoke } from "../services/ipc";
import { assetInteger, assetUnsignedInteger, createUnityAssets, type AssetOperation, type AssetValueInput } from "../services/unityAssets";
import { applyUnitySerializedProperties, readUnitySerializedProperty } from "../services/unitySerializedProperty";
import { parseUnityPropertyFence } from "../composables/unityPropertyFence";

const invoke = vi.mocked(ipcInvoke);
const scope = { checkoutId: "asset-checkout", expectedGeneration: 4, expectedMaterializationEpoch: 8 };
const field = { object_id: "9007199254740993", property_path: "/MonoBehaviour/data" };
const set = (value: AssetValueInput): AssetOperation => ({ op: "set", ...field, value });
beforeEach(() => { invoke.mockReset(); invoke.mockResolvedValue({}); });

describe("unified asset SDK contract", () => {
  it("round-trips uint64 max from a snapshot while keeping object and reference IDs signed", async () => {
    const assets = createUnityAssets(scope);
    const value = assetUnsignedInteger("18446744073709551615");
    invoke.mockResolvedValueOnce({ revision: "unsigned-revision", diagnostics: [], objects: [{
      object_id: field.object_id, class_id: "114", root_type: "MonoBehaviour",
      fields: [{ property_path: field.property_path, kind: "array", value: [0, value] }],
    }] });
    const snapshot = await assets.read("Assets/A.asset");
    await assets.apply("Assets/A.asset", [set(snapshot.objects[0]!.fields[0]!.value)], { expected_revision: snapshot.revision });
    const wire = JSON.parse(JSON.stringify(invoke.mock.calls[1]![1]));
    expect(wire.request.operations).toEqual([set([0, { kind: "uint64", value: "18446744073709551615" }])]);
    expect(assets.unsignedInteger(0n)).toEqual({ kind: "uint64", value: "0" });
    invoke.mockClear();
    for (const invalid of [
      { kind: "uint64", value: "18446744073709551616" }, { kind: "uint64", value: "-1" },
      { kind: "uint64", value: 1 }, { kind: "uint64", value: "1", extra: true },
      { kind: "int64", value: "18446744073709551615" }, { fileID: "18446744073709551615" }, { rid: "18446744073709551615" },
    ]) await expect(assets.preview("Assets/A.asset", [set(invalid as AssetValueInput)])).rejects.toThrow();
    await expect(assets.preview("Assets/A.asset", [{ ...set(1), object_id: "18446744073709551615" }])).rejects.toThrow();
    expect(invoke).not.toHaveBeenCalled();
  });

  it("validates the unsigned helper without broadening the signed helper", () => {
    for (const invalid of ["-1", "-0", "01", "1.0", " 1", "18446744073709551616"]) expect(() => assetUnsignedInteger(invalid)).toThrow();
    expect(() => assetUnsignedInteger(9007199254740992 as never)).toThrow();
    expect(assetUnsignedInteger((1n << 64n) - 1n)).toEqual({ kind: "uint64", value: "18446744073709551615" });
    expect(() => assetInteger((1n << 64n) - 1n)).toThrow();
  });

  it("keeps legacy Inspector target IDs exact and rejects a rounded batch target before IPC", async () => {
    const target = { kind: "asset", path: "Assets/A.asset", targetFileId: "9007199254740993", objectFileId: 11400000 };
    await readUnitySerializedProperty(scope, { target });
    expect(invoke).toHaveBeenCalledExactlyOnceWith("unity_serialized_property_read", { workspaceRef: scope,
      request: { target: { ...target, objectFileId: "11400000" } } });
    invoke.mockClear();
    await expect(applyUnitySerializedProperties(scope, { writes: [
      { target: { ...target, propertyPath: "amount" }, value: 1 },
      { target: { ...target, propertyPath: "amount", targetFileId: Number("9007199254740993") }, value: 2 },
    ] })).rejects.toThrow(/unsafe numeric IDs/);
    expect(invoke).not.toHaveBeenCalled();
    const parsed = parseUnityPropertyFence(JSON.stringify({ ...target, propertyPath: "amount" }));
    expect(parsed.entries[0]?.target.targetFileId).toBe("9007199254740993");
  });
  it("binds backend selection and checkout without mutating another context", async () => {
    const input = { ...scope };
    const yaml = createUnityAssets(input);
    const live = yaml.backend("live");
    input.checkoutId = "other";
    await yaml.read("Assets/A.asset");
    await live.read("Assets/A.asset");
    expect(invoke.mock.calls.map((call) => call[1])).toEqual([
      { workspaceRef: scope, request: { action: "read", backend: "yaml", path: "Assets/A.asset", object_id: undefined, property_path: undefined } },
      { workspaceRef: scope, request: { action: "read", backend: "live", path: "Assets/A.asset", object_id: undefined, property_path: undefined } },
    ]);
  });

  it("preserves 64-bit scalar values and reference identities through JSON", async () => {
    const assets = createUnityAssets(scope);
    const operations = [set({ exact: 9007199254740993n, owner: { fileID: 9007199254740993n }, node: { rid: assets.integer("9223372036854775807") }, literal: "9007199254740993" })];
    await assets.apply("Assets/A.asset", operations, { expected_revision: "read-revision" });
    const payload = JSON.parse(JSON.stringify(invoke.mock.calls[0]![1]));
    expect(payload.request.operations[0]).toEqual(set({
      exact: { kind: "int64", value: "9007199254740993" },
      owner: { fileID: "9007199254740993" }, node: { rid: "9223372036854775807" }, literal: "9007199254740993",
    }));
    expect(operations[0]).toEqual(set({ exact: 9007199254740993n, owner: { fileID: 9007199254740993n }, node: { rid: assets.integer("9223372036854775807") }, literal: "9007199254740993" }));
  });

  it("rejects rounded numbers, unsupported values and cycles before dispatch", async () => {
    const assets = createUnityAssets(scope);
    const circular: { [key: string]: AssetValueInput } = {}; circular.self = circular;
    for (const value of [Number("9007199254740993"), { fileID: Number("9007199254740993") }, NaN, Infinity, circular, { missing: undefined }]) {
      await expect(assets.preview("Assets/A.asset", [set(value as AssetValueInput)])).rejects.toThrow();
    }
    expect(invoke).not.toHaveBeenCalled();
    expect(() => assetInteger("9223372036854775808")).toThrow(/64-bit/);
    expect(() => assetInteger("01")).toThrow(/canonical/);
    expect(() => assetInteger("-0")).toThrow(/canonical/);
  });

  it("requires every batch revision and checks every operation before dispatch", async () => {
    const assets = createUnityAssets(scope);
    await expect(assets.apply("Assets/A.asset", [set(1)], { expected_revision: " " })).rejects.toThrow(/expected_revision/);
    await expect(assets.apply_batch([
      { path: "Assets/A.asset", operations: [set(1)], expected_revision: "A" },
      { path: "Assets/B.asset", operations: [set(2)], expected_revision: "" },
    ])).rejects.toThrow(/expected_revision/);
    await expect(assets.apply_batch([
      { path: "Assets/A.asset", operations: [set(1)], expected_revision: "A" },
      { path: "Assets/B.asset", operations: [{ ...field, op: "array_remove", index: -1 }], expected_revision: "B" },
    ])).rejects.toThrow(/index/);
    expect(invoke).not.toHaveBeenCalled();
  });

  it("submits all files in one transaction, without changing explicit operation semantics", async () => {
    const entries = [
      { path: "Assets/A.asset", expected_revision: "A", operations: [{ ...field, op: "array_insert" as const, index: 0, value: 9 }] },
      { path: "Assets/B.asset", expected_revision: "B", operations: [{ ...field, op: "array_resize" as const, size: 5, value: 0 }] },
    ];
    await createUnityAssets(scope).backend("live").apply_batch(entries);
    expect(invoke).toHaveBeenCalledExactlyOnceWith("unity_assets_execute", { workspaceRef: scope, request: { action: "apply_batch", backend: "live", entries, persist: "disk" } });
  });

  it("blocks all new calls after the View or execution is cancelled", async () => {
    const controller = new AbortController();
    const assets = createUnityAssets(scope, { signal: controller.signal });
    const live = assets.backend("live");
    controller.abort();
    for (const pending of [assets.read("Assets/A.asset"), live.capabilities(), assets.apply("Assets/A.asset", [set(1)], { expected_revision: "A" })]) {
      await expect(pending).rejects.toThrow(/cancelled/);
    }
    expect(invoke).not.toHaveBeenCalled();
  });

  it("cannot turn read options into a different action, backend or checkout", async () => {
    await createUnityAssets(scope).read("Assets/A.asset", { ...field, action: "apply", backend: "live", workspaceRef: { checkoutId: "other" } } as never);
    expect(invoke).toHaveBeenCalledExactlyOnceWith("unity_assets_execute", { workspaceRef: scope, request: { action: "read", backend: "yaml", path: "Assets/A.asset", ...field } });
  });

  it("does not accept numeric IDs, malformed pointers or misspelled operation fields", async () => {
    const assets = createUnityAssets(scope);
    for (const operation of [
      { ...set(1), object_id: 11400000 }, { ...set(1), property_path: "speed" },
      { ...set(1), property_path: "/MonoBehaviour/a~2b" }, { ...set(1), index: 3 },
      { ...set(1), object_id: undefined },
    ]) await expect(assets.preview("Assets/A.asset", [operation as AssetOperation])).rejects.toThrow();
    expect(invoke).not.toHaveBeenCalled();
  });
});
