import { describe, expect, it } from "vitest";
import { clampFloatingPosition } from "../components/ui/floatingPosition";

describe("floatingPosition", () => {
  it("keeps menus inside the right and bottom viewport edges", () => {
    expect(clampFloatingPosition(
      { x: 780, y: 580 },
      { width: 180, height: 120 },
      { width: 800, height: 600 },
      8,
    )).toEqual({ x: 612, y: 472 });
  });

  it("keeps menus away from the top and left viewport edges", () => {
    expect(clampFloatingPosition(
      { x: -12, y: 2 },
      { width: 180, height: 120 },
      { width: 800, height: 600 },
      8,
    )).toEqual({ x: 8, y: 8 });
  });

  it("pins oversized menus to the viewport margin", () => {
    expect(clampFloatingPosition(
      { x: 80, y: 90 },
      { width: 200, height: 160 },
      { width: 100, height: 90 },
      8,
    )).toEqual({ x: 8, y: 8 });
  });
});
