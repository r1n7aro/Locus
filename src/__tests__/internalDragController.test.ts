// @vitest-environment jsdom

import { afterEach, describe, expect, it, vi } from "vitest";
import {
  createInternalDragController,
  internalDragFloatingTransform,
} from "../composables/useInternalDrag";

function pointerEvent(type: string, x: number, y: number, pointerId = 1): PointerEvent {
  const event = new MouseEvent(type, {
    bubbles: true,
    cancelable: true,
    button: 0,
    clientX: x,
    clientY: y,
  });
  Object.defineProperties(event, {
    pointerId: { value: pointerId },
    isPrimary: { value: true },
  });
  return event as unknown as PointerEvent;
}

afterEach(() => {
  document.body.replaceChildren();
  document.body.className = "";
  vi.restoreAllMocks();
});

describe("internal drag controller", () => {
  it("keeps the floating hotspot invariant at viewport edges and fractional coordinates", () => {
    expect(internalDragFloatingTransform(
      { x: 10.25, y: 899.75 },
      { x: 98.5, y: 15.25 },
    )).toBe("translate3d(-88.25px, 884.5px, 0)");
    expect(internalDragFloatingTransform(
      { x: 1399.5, y: 0.5 },
      { x: 98.5, y: 15.25 },
    )).toBe("translate3d(1301px, -14.75px, 0)");
  });

  it("tracks every pointer position and commits one accepted target", () => {
    const controller = createInternalDragController();
    const root = document.createElement("div");
    const sourceElement = document.createElement("button");
    const targetElement = document.createElement("div");
    root.append(sourceElement, targetElement);
    document.body.append(root);
    Object.defineProperty(document, "elementFromPoint", {
      configurable: true,
      value: vi.fn(() => targetElement),
    });
    const drop = vi.fn();
    const targetChanges: string[] = [];
    controller.registerTarget({
      id: "tree",
      root: () => root,
      accepts: (source) => source.payload.type === "test/item",
      resolve: ({ point }) => ({
        key: `target:${point.x}:${point.y}`,
        operation: "move",
        intent: { id: "target" },
      }),
      onTargetChange: (decision) => targetChanges.push(decision?.key ?? "none"),
      drop,
    });
    sourceElement.addEventListener("pointerdown", (event) => {
      controller.start(event as PointerEvent, {
        id: "source",
        payload: { type: "test/item", data: { id: "source" } },
        preview: { label: "Source", kind: "item" },
        allowedOperations: ["move"],
      });
    });

    sourceElement.dispatchEvent(pointerEvent("pointerdown", 10, 10));
    window.dispatchEvent(pointerEvent("pointermove", 30, 35));
    expect(controller.dragging.value).toBe(true);
    expect(controller.point.value).toEqual({ x: 30, y: 35 });
    expect(controller.activeTarget.value?.decision.operation).toBe("move");

    window.dispatchEvent(pointerEvent("pointermove", 80, 90));
    expect(controller.point.value).toEqual({ x: 80, y: 90 });
    expect(targetChanges).toContain("target:80:90");

    window.dispatchEvent(pointerEvent("pointerup", 80, 90));
    expect(drop).toHaveBeenCalledOnce();
    expect(controller.phase.value).toBe("idle");
    sourceElement.dispatchEvent(new MouseEvent("click", { bubbles: true, cancelable: true }));
    controller.dispose();
  });

  it("keeps invalid targets rejected and cancels cleanly with Escape", () => {
    const controller = createInternalDragController();
    const root = document.createElement("div");
    const sourceElement = document.createElement("button");
    const invalidTarget = document.createElement("div");
    root.append(sourceElement, invalidTarget);
    document.body.append(root);
    Object.defineProperty(document, "elementFromPoint", {
      configurable: true,
      value: vi.fn(() => invalidTarget),
    });
    const drop = vi.fn();
    const finished = vi.fn();
    controller.registerTarget({
      id: "tree",
      root: () => root,
      accepts: () => true,
      resolve: () => null,
      drop,
    });
    sourceElement.addEventListener("pointerdown", (event) => {
      controller.start(event as PointerEvent, {
        id: "source",
        payload: { type: "test/item", data: null },
        preview: { label: "Source" },
        onFinished: finished,
      });
    });

    sourceElement.dispatchEvent(pointerEvent("pointerdown", 5, 5));
    window.dispatchEvent(pointerEvent("pointermove", 25, 25));
    expect(controller.dragging.value).toBe(true);
    expect(controller.activeTarget.value).toBeNull();

    window.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
    expect(controller.phase.value).toBe("idle");
    expect(drop).not.toHaveBeenCalled();
    expect(finished).toHaveBeenCalledWith({ dropped: false, reason: "escape" });
    sourceElement.dispatchEvent(new MouseEvent("click", { bubbles: true, cancelable: true }));
    controller.dispose();
  });

  it("preserves ordinary clicks below the movement threshold", () => {
    const controller = createInternalDragController();
    const sourceElement = document.createElement("button");
    document.body.append(sourceElement);
    const activated = vi.fn();
    const click = vi.fn();
    sourceElement.addEventListener("click", click);
    sourceElement.addEventListener("pointerdown", (event) => {
      controller.start(event as PointerEvent, {
        id: "source",
        payload: { type: "test/item", data: null },
        preview: { label: "Source" },
        onActivated: activated,
      });
    });

    sourceElement.dispatchEvent(pointerEvent("pointerdown", 10, 10));
    window.dispatchEvent(pointerEvent("pointermove", 12, 12));
    window.dispatchEvent(pointerEvent("pointerup", 12, 12));
    sourceElement.dispatchEvent(new MouseEvent("click", { bubbles: true, cancelable: true }));
    expect(activated).not.toHaveBeenCalled();
    expect(click).toHaveBeenCalledOnce();
    controller.dispose();
  });

  it("switches to inline presentation anywhere inside a registered list", () => {
    const controller = createInternalDragController();
    const root = document.createElement("div");
    const sourceElement = document.createElement("button");
    const invalidListArea = document.createElement("div");
    const outside = document.createElement("div");
    root.append(sourceElement, invalidListArea);
    document.body.append(root, outside);
    let hit: Element = invalidListArea;
    Object.defineProperty(document, "elementFromPoint", {
      configurable: true,
      value: vi.fn(() => hit),
    });
    controller.registerTarget({
      id: "list",
      root: () => root,
      accepts: () => true,
      resolve: () => null,
      previewMode: "inline",
      drop: vi.fn(),
    });
    sourceElement.addEventListener("pointerdown", (event) => {
      controller.start(event as PointerEvent, {
        id: "source",
        payload: { type: "test/item", data: null },
        preview: { label: "Source" },
      });
    });

    sourceElement.dispatchEvent(pointerEvent("pointerdown", 5, 5));
    window.dispatchEvent(pointerEvent("pointermove", 25, 25));
    expect(controller.previewMode.value).toBe("inline");
    expect(controller.activeTarget.value).toBeNull();

    hit = outside;
    window.dispatchEvent(pointerEvent("pointermove", 45, 45));
    expect(controller.previewMode.value).toBe("floating");
    window.dispatchEvent(pointerEvent("pointercancel", 45, 45));
    sourceElement.dispatchEvent(new MouseEvent("click", { bubbles: true, cancelable: true }));
    controller.dispose();
  });

  it("streams visual coordinates synchronously and avoids repeated target updates", () => {
    const controller = createInternalDragController();
    const root = document.createElement("div");
    const sourceElement = document.createElement("button");
    const targetElement = document.createElement("div");
    root.append(sourceElement, targetElement);
    document.body.append(root);
    const elementFromPoint = vi.fn(() => targetElement);
    Object.defineProperty(document, "elementFromPoint", {
      configurable: true,
      value: elementFromPoint,
    });
    const targetChange = vi.fn();
    controller.registerTarget({
      id: "stable-target",
      root: () => root,
      accepts: () => true,
      resolve: () => ({ key: "stable", operation: "move", intent: { id: "stable" } }),
      onTargetChange: targetChange,
      drop: vi.fn(),
    });
    const visualPoints: Array<{ x: number; y: number }> = [];
    controller.subscribeVisualPoint((nextPoint) => visualPoints.push(nextPoint));
    sourceElement.addEventListener("pointerdown", (event) => {
      controller.start(event as PointerEvent, {
        id: "source",
        payload: { type: "test/item", data: null },
        preview: { label: "Source" },
      });
    });

    sourceElement.dispatchEvent(pointerEvent("pointerdown", 10, 10));
    window.dispatchEvent(pointerEvent("pointermove", 30, 35));
    expect(visualPoints[visualPoints.length - 1]).toEqual({ x: 30, y: 35 });
    window.dispatchEvent(pointerEvent("pointermove", 40, 45));
    expect(visualPoints[visualPoints.length - 1]).toEqual({ x: 40, y: 45 });
    expect(targetChange).toHaveBeenCalledTimes(1);
    expect(elementFromPoint).toHaveBeenCalledTimes(2);

    window.dispatchEvent(pointerEvent("pointerrawupdate", 52, 58));
    expect(visualPoints[visualPoints.length - 1]).toEqual({ x: 52, y: 58 });
    expect(controller.point.value).toEqual({ x: 40, y: 45 });
    expect(elementFromPoint).toHaveBeenCalledTimes(2);

    window.dispatchEvent(pointerEvent("pointercancel", 52, 58));
    controller.dispose();
  });

  it("keeps dragging through window listeners when an inline gap removes the capture node", () => {
    const controller = createInternalDragController();
    const sourceElement = document.createElement("button");
    document.body.append(sourceElement);
    Object.defineProperty(document, "elementFromPoint", {
      configurable: true,
      value: vi.fn(() => document.body),
    });
    sourceElement.addEventListener("pointerdown", (event) => {
      controller.start(event as PointerEvent, {
        id: "source",
        payload: { type: "test/item", data: null },
        preview: { label: "Source" },
      });
    });

    sourceElement.dispatchEvent(pointerEvent("pointerdown", 20, 20));
    window.dispatchEvent(pointerEvent("pointermove", 30, 30));
    expect(controller.dragging.value).toBe(true);

    sourceElement.remove();
    sourceElement.dispatchEvent(pointerEvent("lostpointercapture", 30, 30));
    expect(controller.dragging.value).toBe(true);

    window.dispatchEvent(pointerEvent("pointermove", 80, 90));
    expect(controller.point.value).toEqual({ x: 80, y: 90 });
    window.dispatchEvent(pointerEvent("pointercancel", 80, 90));
    expect(controller.phase.value).toBe("idle");
    controller.dispose();
  });
});
