import { describe, expect, it } from "vitest";
import type { MergeField } from "../types";
import { detectMergeVector, formatVectorNumber } from "../components/collab/mergeFieldValues";
import { parseDisplayValue } from "../components/diff/fieldUtils";

function field(values: Partial<MergeField>): MergeField {
  return { id: "field", propertyPath: "/Transform/value", label: "value", valueType: "mapping", mergeState: "conflict", children: [], ...values };
}

describe("Collab merge field value presentation", () => {
  it("keeps 64-bit fileIDs and GUIDs as exact reference text", () => {
    const reference = "{fileID: 9007199254740993, guid: 112233445566778899aabbccddeeff00, type: 3}";
    expect(detectMergeVector(field({ base: reference, ours: reference, theirs: reference }))).toBeNull();
    expect(parseDisplayValue(reference, "mapping").primary).toBe(reference);
    expect(formatVectorNumber("9007199254740993")).toBe("9007199254740993");
    expect(formatVectorNumber("112233445566778899aabbccddeeff00")).toBe("112233445566778899aabbccddeeff00");
  });

  it("matches components by key with a stable vector order across reordered mappings", () => {
    expect(detectMergeVector(field({
      base: "{z: 30, x: 10, y: 20}",
      ours: "{y: 21, z: 31, x: 11}",
      theirs: "{x: 12, y: 22, z: 32}",
    }))).toEqual([
      { label: "X", base: "10", ours: "11", theirs: "12" },
      { label: "Y", base: "20", ours: "21", theirs: "22" },
      { label: "Z", base: "30", ours: "31", theirs: "32" },
    ]);
  });

  it("uses RGBA ordering and preserves missing snapshot sides", () => {
    expect(detectMergeVector(field({ theirs: "{a: 1, b: 0.3, r: 0.1, g: 0.2}" }))).toEqual([
      { label: "R", base: "", ours: "", theirs: "0.1" },
      { label: "G", base: "", ours: "", theirs: "0.2" },
      { label: "B", base: "", ours: "", theirs: "0.3" },
      { label: "A", base: "", ours: "", theirs: "1" },
    ]);
  });

  it("keeps changed key sets, duplicate keys, mixed groups, and nested maps raw", () => {
    for (const theirs of ["{x: 1, z: 2}", "{x: 1, x: 2}", "{x: 1, r: 2}", "{x: {rid: 1}, y: 2}", "{x: 'label', y: 2}", "null"]) {
      expect(detectMergeVector(field({ base: "{x: 0, y: 0}", theirs }))).toBeNull();
    }
  });

  it("sorts explicit child fields by their own names without changing their values", () => {
    expect(detectMergeVector(field({ children: [field({ label: "y", base: "2", ours: "3", theirs: "4" }), field({ label: "x", base: "5", ours: "6", theirs: "7" })] }))).toEqual([
      { label: "X", base: "5", ours: "6", theirs: "7" },
      { label: "Y", base: "2", ours: "3", theirs: "4" },
    ]);
    expect(detectMergeVector(field({ children: [field({ label: "x" }), field({ label: "x" })] }))).toBeNull();
  });
});
