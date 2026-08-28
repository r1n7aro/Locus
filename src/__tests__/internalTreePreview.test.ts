import { describe, expect, it } from "vitest";
import { withInternalTreePreview } from "../components/explorer/internalTreePreview";

interface Entry {
  key: string;
  depth: number;
}

const entries: Entry[] = [
  { key: "a", depth: 0 },
  { key: "a-child", depth: 1 },
  { key: "b", depth: 0 },
  { key: "b-child", depth: 1 },
  { key: "c", depth: 0 },
];

const describeEntry = (entry: Entry) => ({ nodeKey: entry.key, depth: entry.depth });
const preview = (depth: number): Entry => ({ key: "preview", depth });

describe("internal tree inline preview", () => {
  it("removes the source subtree and inserts after the complete target subtree", () => {
    const result = withInternalTreePreview(
      entries,
      { sourceKey: "a", targetKey: "b", position: "after" },
      describeEntry,
      preview,
    );
    expect(result).toEqual([
      { key: "b", depth: 0 },
      { key: "b-child", depth: 1 },
      { key: "preview", depth: 0 },
      { key: "c", depth: 0 },
    ]);
  });

  it("places an inside preview after the target's visible children", () => {
    const result = withInternalTreePreview(
      entries,
      { sourceKey: "c", targetKey: "b", position: "inside" },
      describeEntry,
      preview,
    );
    expect(result.map((entry) => `${entry.key}:${entry.depth}`)).toEqual([
      "a:0",
      "a-child:1",
      "b:0",
      "b-child:1",
      "preview:1",
    ]);
  });

  it("places a root preview at the list tail", () => {
    const result = withInternalTreePreview(
      entries,
      { sourceKey: "b", targetKey: null, position: "root", rootDepth: 0 },
      describeEntry,
      preview,
    );
    expect(result.map((entry) => entry.key)).toEqual(["a", "a-child", "c", "preview"]);
  });
});
