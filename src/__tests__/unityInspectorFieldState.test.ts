import { describe, expect, it } from "vitest";
import type { InspectorField } from "../types";
import {
  AUTO_COLLAPSE_CHILD_COUNT,
  shouldAutoCollapseField,
} from "../components/diff/unityInspectorFieldState";

function makeField(
  propertyPath: string,
  childCount = 0,
): InspectorField {
  return {
    id: propertyPath,
    label: propertyPath,
    propertyPath,
    valueType: childCount > 0 ? "group" : "string",
    changeKind: "unchanged",
    children: Array.from({ length: childCount }, (_, index) => ({
      id: `${propertyPath}.${index}`,
      label: `[${index}]`,
      propertyPath: `${propertyPath}[${index}]`,
      valueType: "string",
      changeKind: "unchanged" as const,
      children: [],
    })),
  };
}

describe("unityInspectorFieldState", () => {
  it("keeps small field groups expanded by default", () => {
    expect(shouldAutoCollapseField(makeField("smallStruct", AUTO_COLLAPSE_CHILD_COUNT))).toBe(false);
  });

  it("collapses large field groups by default", () => {
    expect(shouldAutoCollapseField(makeField("largeArray", AUTO_COLLAPSE_CHILD_COUNT + 1))).toBe(true);
  });

  it("does not collapse leaf fields", () => {
    expect(shouldAutoCollapseField(makeField("leafField"))).toBe(false);
  });
});
