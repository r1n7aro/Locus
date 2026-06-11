import { describe, expect, it } from "vitest";
import {
  applyCurveTangentMode,
  clampCurveKeyTime,
  curveEditorStateFromValue,
  curveValuePayload,
  evaluateUnityCurve,
  makeCurveKey,
  parseCurveTangent,
  serializeCurveTangent,
} from "../components/unity/editors/curveMath";
import type { UnityAnimationCurveValue } from "../components/unity/unitySerializedValue";

function linearKeys() {
  const keys = [makeCurveKey(0, 0), makeCurveKey(1, 1)];
  applyCurveTangentMode(keys, 0, "linear");
  applyCurveTangentMode(keys, 1, "linear");
  return keys;
}

describe("curveMath", () => {
  it("round-trips infinite tangents through the wire encoding", () => {
    expect(parseCurveTangent("Infinity")).toBe(Number.POSITIVE_INFINITY);
    expect(parseCurveTangent("-Infinity")).toBe(Number.NEGATIVE_INFINITY);
    expect(parseCurveTangent(1.5)).toBe(1.5);
    expect(serializeCurveTangent(Number.POSITIVE_INFINITY)).toBe("Infinity");
    expect(serializeCurveTangent(Number.NEGATIVE_INFINITY)).toBe("-Infinity");
    expect(serializeCurveTangent(2)).toBe(2);
  });

  it("evaluates linear segments exactly", () => {
    const keys = linearKeys();
    expect(evaluateUnityCurve(keys, 0)).toBeCloseTo(0, 6);
    expect(evaluateUnityCurve(keys, 0.25)).toBeCloseTo(0.25, 6);
    expect(evaluateUnityCurve(keys, 0.5)).toBeCloseTo(0.5, 6);
    expect(evaluateUnityCurve(keys, 1)).toBeCloseTo(1, 6);
  });

  it("clamps evaluation outside the key range", () => {
    const keys = linearKeys();
    expect(evaluateUnityCurve(keys, -5)).toBe(0);
    expect(evaluateUnityCurve(keys, 5)).toBe(1);
  });

  it("steps across constant segments", () => {
    const keys = [makeCurveKey(0, 0.2), makeCurveKey(1, 0.9)];
    applyCurveTangentMode(keys, 0, "constant");
    expect(evaluateUnityCurve(keys, 0.5)).toBe(0.2);
    expect(evaluateUnityCurve(keys, 0.999)).toBe(0.2);
    expect(evaluateUnityCurve(keys, 1)).toBe(0.9);
  });

  it("keeps dragged keys strictly ordered", () => {
    const keys = [makeCurveKey(0, 0), makeCurveKey(0.5, 0.5), makeCurveKey(1, 1)];
    expect(clampCurveKeyTime(keys, 1, -2)).toBeGreaterThan(0);
    expect(clampCurveKeyTime(keys, 1, 2)).toBeLessThan(1);
    expect(clampCurveKeyTime(keys, 0, -2)).toBe(-2);
  });

  it("builds a payload that survives a state round-trip", () => {
    const value: UnityAnimationCurveValue = {
      keyCount: 2,
      startTime: 0,
      endTime: 1,
      minValue: 0,
      maxValue: 1,
      samples: [0, 1],
      keys: [
        { time: 0, value: 0 },
        { time: 1, value: 1 },
      ],
    };
    const state = curveEditorStateFromValue({
      ...value,
      keys: value.keys.map((key) => ({
        ...key,
        inTangent: "Infinity" as unknown as number,
        outTangent: 1,
      })),
    } as UnityAnimationCurveValue);
    expect(state.keys[0].inTangent).toBe(Number.POSITIVE_INFINITY);

    const payload = curveValuePayload(state);
    const keys = payload.keys as Array<Record<string, unknown>>;
    expect(keys).toHaveLength(2);
    expect(keys[0].inTangent).toBe("Infinity");
    expect(keys[0].outTangent).toBe(1);
    expect(payload.preWrapMode).toBe("ClampForever");
  });

  it("creates a default 0-1 curve when the value has no keys", () => {
    const state = curveEditorStateFromValue(null);
    expect(state.keys).toHaveLength(2);
    expect(evaluateUnityCurve(state.keys, 0.5)).toBeGreaterThan(0);
    expect(evaluateUnityCurve(state.keys, 0.5)).toBeLessThan(1);
  });
});
