import { describe, expect, it } from "vitest";
import {
  placeTopBarNode,
  removeTopBarNode,
  topBarInsertionBefore,
  topBarLayout,
  topBarNodeIds,
} from "../components/workbench/workspaceTopBarNodes";

describe("workspace top bar order", () => {
  it("keeps the page tabs when nothing is stored and skips unknown or repeated entries", () => {
    expect(topBarLayout(undefined)).toEqual(["development", "plugins", "settings"]);
    expect(topBarLayout(["development", "future", " views ", "views", 3, "plugins", "settings"]))
      .toEqual(["development", "views", "plugins", "settings"]);
    // An edited value without some pages gets them back beside their neighbours.
    expect(topBarLayout(["collaboration", "settings"])).toEqual(["development", "plugins", "collaboration", "settings"]);
    expect(topBarNodeIds(["views", "development", "agents"])).toEqual(["views", "agents"]);
  });

  it("adds a node after Development and the nodes already beside it", () => {
    const first = placeTopBarNode([], "collaboration");
    expect(first).toEqual(["development", "collaboration", "plugins", "settings"]);
    expect(placeTopBarNode(first, "views")).toEqual(["development", "collaboration", "views", "plugins", "settings"]);
    // A node already in the bar keeps its slot.
    expect(placeTopBarNode(["development", "plugins", "settings", "agents"], "agents"))
      .toEqual(["development", "plugins", "settings", "agents"]);
  });

  it("moves a node before the tab it was dropped on, or to the end", () => {
    const layout = ["development", "collaboration", "views", "plugins", "settings"];
    expect(placeTopBarNode(layout, "views", "development")).toEqual(["views", "development", "collaboration", "plugins", "settings"]);
    expect(placeTopBarNode(layout, "collaboration", null)).toEqual(["development", "views", "plugins", "settings", "collaboration"]);
    expect(placeTopBarNode(layout, "views", "views")).toEqual(layout);
    expect(placeTopBarNode(layout, "assets", "plugins")).toEqual(["development", "collaboration", "views", "assets", "plugins", "settings"]);
  });

  it("stores nothing once the last node returns to the tree", () => {
    expect(removeTopBarNode(["development", "collaboration", "plugins", "settings"], "collaboration")).toEqual([]);
    expect(removeTopBarNode(["development", "views", "plugins", "settings", "agents"], "views"))
      .toEqual(["development", "plugins", "settings", "agents"]);
  });

  it("resolves the insertion point from the tab midpoints", () => {
    const tabs = [
      { id: "development" as const, left: 40, right: 80 },
      { id: "collaboration" as const, left: 80, right: 120 },
      { id: "settings" as const, left: 120, right: 160 },
    ];
    expect(topBarInsertionBefore(30, tabs)).toBe("development");
    expect(topBarInsertionBefore(70, tabs)).toBe("collaboration");
    expect(topBarInsertionBefore(141, tabs)).toBeNull();
  });
});
