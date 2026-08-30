// @vitest-environment jsdom

import { describe, expect, it } from "vitest";
import {
  WORKBENCH_REFERENCE_INTERNAL_DRAG_TYPE,
  claimWorkbenchReferencePointerEvent,
  isWorkbenchReferencePointerEventClaimed,
  workbenchReferenceFromElement,
  workbenchReferenceInternalDragSource,
  type WorkbenchReferenceDragData,
} from "../components/workbench/workbenchReferenceDrag";

function targetFromHtml(html: string, selector: string): Element {
  const host = document.createElement("div");
  host.innerHTML = html;
  const target = host.querySelector(selector);
  if (!target) throw new Error(`Missing test target: ${selector}`);
  return target;
}

describe("workbench conversation reference drag", () => {
  it("normalizes plan knowledge references from markdown", () => {
    const target = targetFromHtml(
      '<span class="md-file-ref md-knowledge-ref" data-knowledge-type="plan" data-knowledge-path="plan/combat/rollout.md"><span class="md-ref-label">rollout.md</span></span>',
      ".md-ref-label",
    );
    expect(workbenchReferenceFromElement(target)).toEqual({
      kind: "knowledge",
      type: "plan",
      path: "plan/combat/rollout.md",
      name: "rollout.md",
    });
  });

  it("normalizes asset, scene-object, and local-file surfaces", () => {
    expect(workbenchReferenceFromElement(targetFromHtml(
      '<span class="asset-chip" data-ref-kind="asset" data-asset-path="Assets/Prefabs/Player.prefab"><span class="asset-chip-name">Player</span></span>',
      ".asset-chip-name",
    ))).toMatchObject({ kind: "asset", path: "Assets/Prefabs/Player.prefab", name: "Player" });

    expect(workbenchReferenceFromElement(targetFromHtml(
      '<span class="md-unity-scene-object-ref" data-scene-path="Assets/Scenes/Main.unity" data-scene-object-path="Player/Camera"><span class="md-ref-label">Camera</span></span>',
      ".md-ref-label",
    ))).toMatchObject({
      kind: "sceneObject",
      scenePath: "Assets/Scenes/Main.unity",
      objectPath: "Player/Camera",
      name: "Camera",
    });

    expect(workbenchReferenceFromElement(targetFromHtml(
      '<span class="md-file-ref" data-file-path="src/gameplay/state.ts" data-entry-kind="file"><span class="md-ref-label">state.ts</span></span>',
      ".md-ref-label",
    ))).toMatchObject({ kind: "file", path: "src/gameplay/state.ts", isDir: false });
  });

  it("accepts a Live Preview reference inside CodeMirror contenteditable", () => {
    const target = targetFromHtml(
      '<div contenteditable="true"><span class="cm-live-reference md-file-ref md-knowledge-ref" data-reference-kind="knowledge" data-knowledge-type="design" data-knowledge-path="design/editor.md"><span class="cm-live-reference-label">editor.md</span></span></div>',
      ".cm-live-reference-label",
    );
    expect(workbenchReferenceFromElement(target)).toMatchObject({
      kind: "knowledge",
      type: "design",
      path: "design/editor.md",
    });
  });

  it("publishes a copy-only semantic source and claims the captured gesture", () => {
    const data: WorkbenchReferenceDragData = {
      version: 1,
      origin: {
        projectId: "project-a",
        workspaceRef: { checkoutId: "checkout-a", expectedGeneration: 7 },
        workspaceRoot: "F:/Game",
      },
      entries: [{ kind: "asset", path: "Assets/Player.prefab", name: "Player" }],
    };
    const source = workbenchReferenceInternalDragSource(data);
    expect(source.payload.type).toBe(WORKBENCH_REFERENCE_INTERNAL_DRAG_TYPE);
    expect(source.payload.data).toBe(data);
    expect(source.allowedOperations).toEqual(["copy"]);
    expect(source.externalize).toBeTypeOf("function");

    const event = new MouseEvent("pointerdown") as PointerEvent;
    expect(isWorkbenchReferencePointerEventClaimed(event)).toBe(false);
    claimWorkbenchReferencePointerEvent(event);
    expect(isWorkbenchReferencePointerEventClaimed(event)).toBe(true);
  });
});
