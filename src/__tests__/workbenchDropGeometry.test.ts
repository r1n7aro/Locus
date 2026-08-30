import { describe, expect, it } from "vitest";
import {
  workbenchSplitDirectionAtPoint,
  workbenchTabInsertionIndexAtPoint,
} from "../components/workbench/workbenchDropGeometry";
import { moveTabAtInsertionIndex } from "../components/ui/tabDropGeometry";

const bounds = { left: 100, right: 500, top: 50, bottom: 350 };

describe("workbench VS Code-style drop geometry", () => {
  it("maps the editor body to a single nearest half preview", () => {
    expect(workbenchSplitDirectionAtPoint({ x: 110, y: 200 }, bounds)).toBe("left");
    expect(workbenchSplitDirectionAtPoint({ x: 490, y: 200 }, bounds)).toBe("right");
    expect(workbenchSplitDirectionAtPoint({ x: 300, y: 60 }, bounds)).toBe("top");
    expect(workbenchSplitDirectionAtPoint({ x: 300, y: 340 }, bounds)).toBe("bottom");
  });

  it("uses each tab half and the strip tail as insertion positions", () => {
    const tabs = [
      { left: 100, right: 200 },
      { left: 200, right: 320 },
      { left: 320, right: 440 },
    ];
    expect(workbenchTabInsertionIndexAtPoint(110, tabs)).toBe(0);
    expect(workbenchTabInsertionIndexAtPoint(180, tabs)).toBe(1);
    expect(workbenchTabInsertionIndexAtPoint(250, tabs)).toBe(1);
    expect(workbenchTabInsertionIndexAtPoint(300, tabs)).toBe(2);
    expect(workbenchTabInsertionIndexAtPoint(480, tabs)).toBe(3);
  });

  it("uses the shared pre-removal insertion model for same-strip reordering", () => {
    expect(moveTabAtInsertionIndex(["a", "b", "c"], 0, 3)).toEqual(["b", "c", "a"]);
    expect(moveTabAtInsertionIndex(["a", "b", "c"], 2, 0)).toEqual(["c", "a", "b"]);
    expect(moveTabAtInsertionIndex(["a", "b", "c"], 1, 2)).toEqual(["a", "b", "c"]);
  });
});
