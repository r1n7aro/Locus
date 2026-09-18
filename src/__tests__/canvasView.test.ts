import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const canvasSource = readFileSync(resolve(process.cwd(), "src/components/canvas/LocusCanvasView.ts"), "utf8");
const canvasTypes = readFileSync(resolve(process.cwd(), "src/components/canvas/canvasTypes.ts"), "utf8");
const canvasStyles = readFileSync(resolve(process.cwd(), "src/components/canvas/canvasStyles.ts"), "utf8");

describe("LocusCanvasView editor behavior", () => {
  it("exposes multi-selection, clipboard, context menu, and edit behavior hooks", () => {
    expect(canvasTypes).toContain("export interface CanvasEditBehavior");
    expect(canvasTypes).toContain("allowBoxSelect?: boolean");
    expect(canvasTypes).toContain("allowDelete?: boolean");
    expect(canvasTypes).toContain("export interface CanvasContextMenuEvent");
    expect(canvasTypes).toContain("export interface CanvasClipboardEvent");
    expect(canvasSource).toContain("selectedItemIds");
    expect(canvasSource).toContain("editBehavior");
    expect(canvasSource).toContain("\"copySelection\"");
    expect(canvasSource).toContain("\"pasteSelection\"");
    expect(canvasSource).toContain("\"contextMenu\"");
    expect(canvasSource).toContain("function startBoxSelection");
    expect(canvasSource).toContain("function emitContextMenu");
    expect(canvasSource).toContain("readonlyLocksEdits");
  });

  it("renders a selection rectangle for box select", () => {
    expect(canvasStyles).toContain(".locus-canvas-selection-box");
    expect(canvasSource).toContain("renderSelectionBox()");
    expect(canvasSource).toContain("behavior.allowBoxSelect ? \"box-select-enabled\" : \"\"");
  });

});
