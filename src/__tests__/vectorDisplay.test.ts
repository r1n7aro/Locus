import { describe, expect, it } from "vitest";
import type { InspectorField } from "../types";
import {
  detectVectorComponents,
  formatQuaternionEulerTuple,
  formatVectorTuple,
  isFullyModifiedVector,
  isQuaternionField,
  quaternionToEulerDegrees,
} from "../components/diff/vectorDisplay";

function makeField(
  propertyPath: string,
  overrides: Partial<InspectorField> = {},
): InspectorField {
  return {
    id: propertyPath,
    label: propertyPath.split(".").pop() ?? propertyPath,
    propertyPath,
    valueType: overrides.children?.length ? "group" : "number",
    changeKind: "modified",
    children: [],
    ...overrides,
  };
}

function child(
  parentPath: string,
  label: string,
  before: string,
  after: string,
  changeKind: InspectorField["changeKind"] = "modified",
): InspectorField {
  return makeField(`${parentPath}.${label}`, {
    label,
    before,
    after,
    changeKind,
  });
}

describe("vectorDisplay", () => {
  it("formats a fully modified vector as one ordered tuple change", () => {
    const field = makeField("m_LocalPosition", {
      children: [
        child("m_LocalPosition", "z", "3", "30"),
        child("m_LocalPosition", "x", "1", "10"),
        child("m_LocalPosition", "y", "2", "20"),
      ],
    });

    const components = detectVectorComponents(field);

    expect(components).not.toBeNull();
    expect(isFullyModifiedVector(components!)).toBe(true);
    expect(formatVectorTuple(components!, "before", (value) => value ?? "")).toBe("(1, 2, 3)");
    expect(formatVectorTuple(components!, "after", (value) => value ?? "")).toBe("(10, 20, 30)");
  });

  it("keeps partial vector changes in per-component mode", () => {
    const field = makeField("m_LocalPosition", {
      children: [
        child("m_LocalPosition", "x", "1", "10"),
        child("m_LocalPosition", "y", "2", "2", "unchanged"),
        child("m_LocalPosition", "z", "3", "3", "unchanged"),
      ],
    });

    const components = detectVectorComponents(field);

    expect(components).not.toBeNull();
    expect(isFullyModifiedVector(components!)).toBe(false);
  });

  it("recognizes Unity rotation quaternions without a C# field type", () => {
    const field = makeField("m_LocalRotation", {
      label: "m_LocalRotation",
      children: [
        child("m_LocalRotation", "w", "1", "0.7071068"),
        child("m_LocalRotation", "x", "0", "0"),
        child("m_LocalRotation", "y", "0", "0.7071068"),
        child("m_LocalRotation", "z", "0", "0"),
      ],
    });
    const components = detectVectorComponents(field);

    expect(components).not.toBeNull();
    expect(isQuaternionField(field, components!)).toBe(true);
    expect(formatQuaternionEulerTuple(components!, "after", (value) => value.toFixed(0))).toBe("(0, 90, 0)");
  });

  it("converts quaternions to Euler degrees", () => {
    const halfTurn = Math.sqrt(0.5);

    const euler = quaternionToEulerDegrees({
      x: halfTurn,
      y: 0,
      z: 0,
      w: halfTurn,
    });

    expect(euler).not.toBeNull();
    expect(euler![0]).toBeCloseTo(90, 3);
    expect(euler![1]).toBeCloseTo(0, 3);
    expect(euler![2]).toBeCloseTo(0, 3);
  });
});
