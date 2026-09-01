import { describe, expect, it } from "vitest";
import { workbenchTabCloseIds } from "../components/workbench/workbenchTabClose";

describe("workbench tab close targets", () => {
  const tabs = ["a", "b", "c", "d"];

  it("selects the current tab and every tab on either side", () => {
    expect(workbenchTabCloseIds(tabs, "c", "current")).toEqual(["c"]);
    expect(workbenchTabCloseIds(tabs, "c", "left")).toEqual(["a", "b"]);
    expect(workbenchTabCloseIds(tabs, "c", "right")).toEqual(["d"]);
    expect(workbenchTabCloseIds(tabs, "c", "all")).toEqual(tabs);
    expect(workbenchTabCloseIds(tabs, "a", "left")).toEqual([]);
    expect(workbenchTabCloseIds(tabs, "d", "right")).toEqual([]);
  });

  it("returns no targets when the context tab is stale", () => {
    expect(workbenchTabCloseIds(tabs, "missing", "all")).toEqual([]);
  });
});
