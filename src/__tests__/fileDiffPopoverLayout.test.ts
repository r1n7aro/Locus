import { describe, expect, it } from "vitest";
import {
  DIFF_POPOVER_MAX_HEIGHT_PX,
  DIFF_POPOVER_MIN_HEIGHT_PX,
  DIFF_POPOVER_WIDTH_PX,
  estimateDiffPopoverHeight,
} from "../components/diff/fileDiffPopoverLayout";
import type { FileDiffPayload, InspectorField } from "../types";

function makePayload(overrides: Partial<FileDiffPayload> = {}): FileDiffPayload {
  return {
    key: "chatCheckpoint:Assets/Foo.prefab::::message-1:preview:",
    filePath: "Assets/Foo.prefab",
    status: "M",
    isBinary: false,
    isLarge: false,
    contentState: { type: "normal" },
    stats: { additions: 0, deletions: 0, changedHunks: 0 },
    previewSummary: [],
    ...overrides,
  };
}

function makeField(id: string, children: InspectorField[] = []): InspectorField {
  return {
    id,
    label: id,
    propertyPath: id,
    valueType: children.length ? "group" : "string",
    changeKind: "modified",
    before: "old",
    after: "new",
    children,
  };
}

describe("fileDiffPopoverLayout", () => {
  it("uses a wider desktop popover width", () => {
    expect(DIFF_POPOVER_WIDTH_PX).toBeGreaterThanOrEqual(720);
  });

  it("keeps small previews readable", () => {
    expect(estimateDiffPopoverHeight(makePayload())).toBe(DIFF_POPOVER_MIN_HEIGHT_PX);
  });

  it("grows with semantic changed field count", () => {
    const small = makePayload({
      semantic: {
        engine: "unity-yaml",
        assetKind: "prefab",
        layout: "assetInspector",
        summary: { changedTargets: 1, changedObjects: 0, changedComponents: 1, changedFields: 2 },
      },
    });
    const large = makePayload({
      semantic: {
        engine: "unity-yaml",
        assetKind: "prefab",
        layout: "assetInspector",
        summary: { changedTargets: 1, changedObjects: 0, changedComponents: 1, changedFields: 24 },
      },
    });

    expect(estimateDiffPopoverHeight(large)).toBeGreaterThan(estimateDiffPopoverHeight(small));
    expect(estimateDiffPopoverHeight(large)).toBeLessThanOrEqual(DIFF_POPOVER_MAX_HEIGHT_PX);
  });

  it("counts nested inspector fields when a preview inspector is present", () => {
    const compact = makePayload({
      semantic: {
        engine: "unity-yaml",
        assetKind: "prefab",
        layout: "assetInspector",
        summary: { changedTargets: 1, changedObjects: 0, changedComponents: 1, changedFields: 1 },
      },
    });
    const nested = makePayload({
      semantic: {
        engine: "unity-yaml",
        assetKind: "prefab",
        layout: "assetInspector",
        summary: { changedTargets: 1, changedObjects: 0, changedComponents: 1, changedFields: 1 },
        inspector: {
          targetId: "target-1",
          title: "Foo",
          path: "Assets/Foo.prefab",
          panels: [{
            panelKind: "component",
            title: "Transform",
            changeKind: "modified",
            added: false,
            removed: false,
            fields: [
              makeField("m_LocalPosition", [
                makeField("m_LocalPosition.x"),
                makeField("m_LocalPosition.y"),
                makeField("m_LocalPosition.z"),
              ]),
              makeField("m_LocalRotation", [
                makeField("m_LocalRotation.x"),
                makeField("m_LocalRotation.y"),
                makeField("m_LocalRotation.z"),
                makeField("m_LocalRotation.w"),
              ]),
              makeField("m_LocalScale", [
                makeField("m_LocalScale.x"),
                makeField("m_LocalScale.y"),
                makeField("m_LocalScale.z"),
              ]),
            ],
          }],
        },
      },
    });

    expect(estimateDiffPopoverHeight(nested)).toBeGreaterThan(estimateDiffPopoverHeight(compact));
  });
});
