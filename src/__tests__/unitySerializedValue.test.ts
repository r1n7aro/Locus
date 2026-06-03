import { describe, expect, it } from "vitest";
import {
  applyUnityRgbHexToColorText,
  constrainUnityNumberValue,
  formatUnityNumberValue,
  formatUnityQuaternionEulerValue,
  formatUnityVectorValue,
  isUnityNumberPropertyType,
  isUnityQuaternionPropertyType,
  isUnityVectorPropertyType,
  normalizeUnityOptions,
  parseUnityQuaternionEulerValue,
  parseUnityColorValue,
  parseUnitySerializedEditValue,
  parseUnityVectorValue,
  unityColorTextToRgbHex,
  unityEnumIndexValue,
  unityEnumNumericValue,
  unitySerializedValueToEditText,
  unityVectorKeysForType,
} from "../components/unity/unitySerializedValue";

describe("unitySerializedValue", () => {
  it("parses scalar Unity serialized values", () => {
    expect(parseUnitySerializedEditValue("Integer", "12")).toBe(12);
    expect(parseUnitySerializedEditValue("Float", "1.5")).toBe(1.5);
    expect(parseUnitySerializedEditValue("LayerMask", "7")).toBe(7);
    expect(parseUnitySerializedEditValue("Boolean", "true")).toBe(true);
    expect(parseUnitySerializedEditValue("String", 42)).toBe("42");
  });

  it("parses and formats vector values by property type", () => {
    expect(isUnityVectorPropertyType("Vector3")).toBe(true);
    expect(unityVectorKeysForType("Vector4")).toEqual(["x", "y", "z", "w"]);
    expect(parseUnityVectorValue("Vector3", "1, 2 3")).toEqual({ x: 1, y: 2, z: 3 });
    expect(formatUnityVectorValue("Vector2", { x: 4, y: 5 })).toBe("4, 5");
    expect(unitySerializedValueToEditText("Vector3", { x: 1, y: 2, z: 3 })).toBe("1, 2, 3");
  });

  it("edits Quaternion values as Euler angles", () => {
    expect(isUnityQuaternionPropertyType("Quaternion")).toBe(true);
    expect(unityVectorKeysForType("Quaternion")).toEqual(["x", "y", "z"]);
    expect(formatUnityQuaternionEulerValue({ x: 0, y: 0.70710678, z: 0, w: 0.70710678 })).toBe("0, 90, 0");
    expect(formatUnityQuaternionEulerValue({ x: 0, y: 0, z: 0, w: 1 }, "10, 20, 30")).toBe("10, 20, 30");
    expect(parseUnityQuaternionEulerValue("10, 20, 30")).toEqual({
      action: "setEuler",
      x: 10,
      y: 20,
      z: 30,
    });
    expect(parseUnitySerializedEditValue("Quaternion", "1 2 3")).toEqual({
      action: "setEuler",
      x: 1,
      y: 2,
      z: 3,
    });
  });

  it("supports Rect values as structured Unity serialized values", () => {
    expect(isUnityVectorPropertyType("Rect")).toBe(true);
    expect(unityVectorKeysForType("Rect")).toEqual(["x", "y", "width", "height"]);
    expect(parseUnityVectorValue("Rect", "1, 2, 30, 40")).toEqual({
      x: 1,
      y: 2,
      width: 30,
      height: 40,
    });
    expect(unitySerializedValueToEditText("Rect", { x: 1, y: 2, width: 30, height: 40 })).toBe("1, 2, 30, 40");
  });

  it("normalizes color and enum editor values", () => {
    expect(parseUnityColorValue({ r: 1, g: 0.5, b: 0, a: 1 })).toBe("#ff8000ff");
    expect(unityColorTextToRgbHex("#11223344")).toBe("#112233");
    expect(applyUnityRgbHexToColorText("#aabbcc", "#11223344")).toBe("#aabbcc44");
    expect(unitySerializedValueToEditText("Enum", { index: 1, name: "Loop" })).toBe("Loop");
    expect(normalizeUnityOptions([{ label: "Bee", value: "B" }])).toEqual([
      { label: "Bee", value: "B", name: undefined, index: undefined, numericValue: undefined },
    ]);
    expect(normalizeUnityOptions([{ label: "Read", value: "1", name: "Read", index: 1, numericValue: 4 }])).toEqual([
      { label: "Read", value: "1", name: "Read", index: 1, numericValue: 4 },
    ]);
    expect(unityEnumIndexValue({ index: 2 })).toBe(2);
    expect(unityEnumNumericValue({ numericValue: 5 })).toBe(5);
  });

  it("classifies numeric Unity property types", () => {
    expect(isUnityNumberPropertyType("Integer")).toBe(true);
    expect(isUnityNumberPropertyType("ArraySize")).toBe(true);
    expect(isUnityNumberPropertyType("LayerMask")).toBe(true);
    expect(isUnityNumberPropertyType("String")).toBe(false);
  });

  it("constrains range-marked numeric values like Unity inspectors", () => {
    expect(constrainUnityNumberValue("Float", 1.5, { hasRange: true, rangeMin: 0, rangeMax: 1 })).toBe(1);
    expect(constrainUnityNumberValue("Float", -0.25, { hasRange: true, rangeMin: 0, rangeMax: 1 })).toBe(0);
    expect(constrainUnityNumberValue("Integer", 2.6, { hasRange: true, rangeMin: 0, rangeMax: 10 })).toBe(3);
    expect(formatUnityNumberValue("Float", 0.1 + 0.2, { hasRange: true, rangeMin: 0, rangeMax: 1 })).toBe("0.3");
  });

  it("rejects partial numeric input before committing", () => {
    expect(() => parseUnitySerializedEditValue("Integer", "12px")).toThrow("Expected integer value");
    expect(() => parseUnitySerializedEditValue("Float", "1.5px")).toThrow("Expected number value");
    expect(() => parseUnitySerializedEditValue("Boolean", "maybe")).toThrow("Expected boolean value");
    expect(() => parseUnityVectorValue("Vector2", "1, 2, 3")).toThrow("Expected vector components");
  });
});
