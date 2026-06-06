import { describe, expect, it } from "vitest";
import { isVNode } from "vue";
import {
  createUnityPropertyRuntime,
  type UnityBoundPropertyApplyRequest,
  type UnityBoundPropertyReadRequest,
  type UnityBoundPropertyWriteRequest,
} from "../components/unity/unityPropertyBinding";
import type { InspectorPropertySnapshot } from "../services/propertyTree";

function makeSnapshot(value = 1): InspectorPropertySnapshot {
  return {
    propertyPath: "m_Speed",
    displayName: "Speed",
    name: "m_Speed",
    type: "Float",
    valueType: "Float",
    value,
    displayValue: String(value),
    editable: true,
    children: [],
  };
}

describe("unityPropertyBinding", () => {
  it("reads a path into a bound property with write, undo, redo, and default draw APIs", async () => {
    const writes: UnityBoundPropertyWriteRequest[] = [];
    let undoCount = 0;
    let redoCount = 0;
    const runtime = createUnityPropertyRuntime({
      read: async (request: UnityBoundPropertyReadRequest) => ({
        ...makeSnapshot(),
        ok: true,
        bindingId: request.bindingId,
        message: "ok",
        target: request.target,
      }),
      write: async (request) => {
        writes.push(request);
        return {
          ...makeSnapshot(Number(request.value)),
          ok: true,
          bindingId: request.bindingId,
          message: "ok",
          target: request.target,
          saved: request.writeMode !== "preview",
        };
      },
      apply: async () => ({ ok: true, message: "ok", results: [] }),
      undo: () => {
        undoCount += 1;
      },
      redo: () => {
        redoCount += 1;
      },
    });

    const property = await runtime.property("selection/property/m_Speed");
    expect(property.propertyPath).toBe("m_Speed");
    expect(property.value).toBe(1);
    expect(isVNode(property.drawDefaultEditor())).toBe(true);

    await property.write(2, { refresh: false });
    await property.preview(3);
    await property.undo();
    await property.redo();

    expect(writes).toMatchObject([
      {
        target: { kind: "selection", propertyPath: "m_Speed" },
        value: 2,
        writeMode: "commit",
      },
      {
        target: { kind: "selection", propertyPath: "m_Speed" },
        value: 3,
        writeMode: "preview",
      },
    ]);
    expect(undoCount).toBe(1);
    expect(redoCount).toBe(1);
  });

  it("reads a tree and applies bound writes", async () => {
    const applyRequests: UnityBoundPropertyApplyRequest[] = [];
    const runtime = createUnityPropertyRuntime({
      read: async (request) => ({
        ...makeSnapshot(),
        ok: true,
        bindingId: request.bindingId,
        message: "ok",
        target: request.target,
        properties: [makeSnapshot()],
      }),
      write: async (request) => ({
        ...makeSnapshot(Number(request.value)),
        ok: true,
        bindingId: request.bindingId,
        message: "ok",
        target: request.target,
        saved: true,
      }),
      apply: async (request) => {
        applyRequests.push(request);
        return { ok: true, message: "ok", results: [] };
      },
    });

    const tree = await runtime.fromPath("asset/Assets/Data/Config.asset/property/m_Speed");
    expect(tree.require("m_Speed").raw.label).toBe("Speed");
    await tree.apply([
      {
        target: tree.require("m_Speed").target,
        value: 4,
      },
    ], { refresh: false });

    expect(applyRequests).toMatchObject([
      {
        writes: [
          {
            target: {
              kind: "asset",
              path: "Assets/Data/Config.asset",
              propertyPath: "m_Speed",
            },
            value: 4,
          },
        ],
      },
    ]);
  });
});

