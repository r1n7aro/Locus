import type { UnityAnimationCurveValue } from "../unitySerializedValue";

/**
 * Editor-side model of a Unity AnimationCurve keyframe.
 *
 * Tangents follow Unity semantics: slopes in value-per-second, with
 * `Infinity` marking a constant (stepped) segment. The wire format encodes
 * infinite tangents as the strings "Infinity"/"-Infinity" because neither
 * strict JSON nor the Tauri bridge can carry non-finite numbers.
 */
export interface UnityCurveEditorKey {
  time: number;
  value: number;
  inTangent: number;
  outTangent: number;
  inWeight: number;
  outWeight: number;
  weightedMode: string;
  /** Editor bookkeeping; loaded keys start as "custom" to preserve shape. */
  mode: UnityCurveTangentMode;
}

export type UnityCurveTangentMode = "custom" | "auto" | "linear" | "flat" | "constant";

export interface UnityCurveEditorState {
  keys: UnityCurveEditorKey[];
  preWrapMode: string;
  postWrapMode: string;
}

const DEFAULT_WEIGHT = 1 / 3;
const MIN_KEY_TIME_GAP = 1e-4;

export function parseCurveTangent(raw: unknown): number {
  if (typeof raw === "number") return Number.isNaN(raw) ? 0 : raw;
  const text = String(raw ?? "").trim().toLowerCase();
  if (text === "infinity" || text === "+infinity") return Number.POSITIVE_INFINITY;
  if (text === "-infinity") return Number.NEGATIVE_INFINITY;
  const numeric = Number(text);
  return Number.isFinite(numeric) ? numeric : 0;
}

export function serializeCurveTangent(value: number): number | string {
  if (value === Number.POSITIVE_INFINITY) return "Infinity";
  if (value === Number.NEGATIVE_INFINITY) return "-Infinity";
  return Number.isFinite(value) ? value : 0;
}

export function curveEditorStateFromValue(
  value: UnityAnimationCurveValue | null | undefined,
): UnityCurveEditorState {
  const sourceKeys = value?.keys ?? [];
  const keys = sourceKeys.map((key) => {
    const record = key as Record<string, unknown>;
    return {
      time: Number(record.time ?? 0) || 0,
      value: Number(record.value ?? 0) || 0,
      inTangent: parseCurveTangent(record.inTangent ?? 0),
      outTangent: parseCurveTangent(record.outTangent ?? 0),
      inWeight: clampWeight(Number(record.inWeight)),
      outWeight: clampWeight(Number(record.outWeight)),
      weightedMode: String(record.weightedMode ?? "None"),
      mode: "custom" as UnityCurveTangentMode,
    };
  });
  keys.sort((left, right) => left.time - right.time);
  if (!keys.length) {
    keys.push(
      makeCurveKey(0, 0, "auto"),
      makeCurveKey(1, 1, "auto"),
    );
    applyCurveTangentMode(keys, 0, "auto");
    applyCurveTangentMode(keys, 1, "auto");
  }
  const record = (value ?? {}) as Record<string, unknown>;
  return {
    keys,
    preWrapMode: String(record.preWrapMode ?? "ClampForever"),
    postWrapMode: String(record.postWrapMode ?? "ClampForever"),
  };
}

export function curveValuePayload(state: UnityCurveEditorState): Record<string, unknown> {
  const keys = [...state.keys].sort((left, right) => left.time - right.time);
  return {
    keys: keys.map((key) => ({
      time: key.time,
      value: key.value,
      inTangent: serializeCurveTangent(key.inTangent),
      outTangent: serializeCurveTangent(key.outTangent),
      inWeight: clampWeight(key.inWeight),
      outWeight: clampWeight(key.outWeight),
      weightedMode: key.weightedMode || "None",
    })),
    preWrapMode: state.preWrapMode,
    postWrapMode: state.postWrapMode,
  };
}

export function makeCurveKey(
  time: number,
  value: number,
  mode: UnityCurveTangentMode = "auto",
): UnityCurveEditorKey {
  return {
    time,
    value,
    inTangent: 0,
    outTangent: 0,
    inWeight: DEFAULT_WEIGHT,
    outWeight: DEFAULT_WEIGHT,
    weightedMode: "None",
    mode,
  };
}

/**
 * Cubic-Hermite evaluation matching Unity's unweighted keyframe math; an
 * infinite tangent on either side of a segment makes it stepped. Weighted
 * tangents are approximated by their unweighted shape.
 */
export function evaluateUnityCurve(keys: readonly UnityCurveEditorKey[], time: number): number {
  if (!keys.length) return 0;
  if (time <= keys[0].time) return keys[0].value;
  const last = keys[keys.length - 1];
  if (time >= last.time) return last.value;

  let rightIndex = 1;
  while (rightIndex < keys.length - 1 && keys[rightIndex].time < time) rightIndex += 1;
  const left = keys[rightIndex - 1];
  const right = keys[rightIndex];
  const dt = right.time - left.time;
  if (dt <= 0) return right.value;
  if (!Number.isFinite(left.outTangent) || !Number.isFinite(right.inTangent)) {
    return left.value;
  }

  const t = (time - left.time) / dt;
  const t2 = t * t;
  const t3 = t2 * t;
  const m0 = left.outTangent * dt;
  const m1 = right.inTangent * dt;
  return (2 * t3 - 3 * t2 + 1) * left.value
    + (t3 - 2 * t2 + t) * m0
    + (-2 * t3 + 3 * t2) * right.value
    + (t3 - t2) * m1;
}

export function sampleUnityCurve(
  keys: readonly UnityCurveEditorKey[],
  startTime: number,
  endTime: number,
  count: number,
): Array<{ time: number; value: number }> {
  const points: Array<{ time: number; value: number }> = [];
  const span = endTime - startTime;
  const steps = Math.max(2, count);
  for (let index = 0; index < steps; index += 1) {
    const time = startTime + (span * index) / (steps - 1);
    points.push({ time, value: evaluateUnityCurve(keys, time) });
  }
  // Stepped segments need explicit corner points to render as right angles.
  for (let index = 0; index < keys.length - 1; index += 1) {
    const left = keys[index];
    const right = keys[index + 1];
    if (!Number.isFinite(left.outTangent) || !Number.isFinite(right.inTangent)) {
      points.push({ time: right.time - 1e-6, value: left.value });
    }
  }
  points.sort((a, b) => a.time - b.time);
  return points;
}

function slope(from: UnityCurveEditorKey, to: UnityCurveEditorKey): number {
  const dt = to.time - from.time;
  if (Math.abs(dt) < 1e-9) return 0;
  return (to.value - from.value) / dt;
}

/** Recomputes one key's tangents for its (newly assigned) tangent mode. */
export function applyCurveTangentMode(
  keys: UnityCurveEditorKey[],
  index: number,
  mode: UnityCurveTangentMode,
): void {
  const key = keys[index];
  if (!key) return;
  key.mode = mode;
  const previous = keys[index - 1];
  const next = keys[index + 1];

  if (mode === "flat") {
    key.inTangent = 0;
    key.outTangent = 0;
  } else if (mode === "linear") {
    key.inTangent = previous ? slope(previous, key) : 0;
    key.outTangent = next ? slope(key, next) : 0;
  } else if (mode === "constant") {
    key.inTangent = Number.POSITIVE_INFINITY;
    key.outTangent = Number.POSITIVE_INFINITY;
  } else if (mode === "auto") {
    let tangent = 0;
    if (previous && next) tangent = slope(previous, next);
    else if (previous) tangent = slope(previous, key);
    else if (next) tangent = slope(key, next);
    key.inTangent = tangent;
    key.outTangent = tangent;
  }
  if (mode !== "custom") key.weightedMode = "None";
}

/**
 * After a key moves, keys with managed (non-custom) modes around it need
 * their tangents refreshed; custom keys keep their authored tangents.
 */
export function refreshManagedTangentsAround(keys: UnityCurveEditorKey[], index: number): void {
  for (const neighbor of [index - 1, index, index + 1]) {
    const key = keys[neighbor];
    if (key && key.mode !== "custom") {
      applyCurveTangentMode(keys, neighbor, key.mode);
    }
  }
}

/** Keeps a dragged key strictly ordered between its neighbors. */
export function clampCurveKeyTime(
  keys: readonly UnityCurveEditorKey[],
  index: number,
  time: number,
): number {
  const min = index > 0 ? keys[index - 1].time + MIN_KEY_TIME_GAP : Number.NEGATIVE_INFINITY;
  const max = index < keys.length - 1
    ? keys[index + 1].time - MIN_KEY_TIME_GAP
    : Number.POSITIVE_INFINITY;
  return Math.min(Math.max(time, min), max);
}

export interface UnityCurvePreset {
  id: string;
  label: string;
  build: () => UnityCurveEditorKey[];
}

export const UNITY_CURVE_PRESETS: UnityCurvePreset[] = [
  {
    id: "linear",
    label: "0 → 1",
    build: () => {
      const keys = [makeCurveKey(0, 0, "linear"), makeCurveKey(1, 1, "linear")];
      applyCurveTangentMode(keys, 0, "linear");
      applyCurveTangentMode(keys, 1, "linear");
      return keys;
    },
  },
  {
    id: "ease-in-out",
    label: "Ease",
    build: () => {
      const keys = [makeCurveKey(0, 0, "flat"), makeCurveKey(1, 1, "flat")];
      applyCurveTangentMode(keys, 0, "flat");
      applyCurveTangentMode(keys, 1, "flat");
      return keys;
    },
  },
  {
    id: "one",
    label: "1",
    build: () => {
      const keys = [makeCurveKey(0, 1, "flat"), makeCurveKey(1, 1, "flat")];
      applyCurveTangentMode(keys, 0, "flat");
      applyCurveTangentMode(keys, 1, "flat");
      return keys;
    },
  },
  {
    id: "zero",
    label: "0",
    build: () => {
      const keys = [makeCurveKey(0, 0, "flat"), makeCurveKey(1, 0, "flat")];
      applyCurveTangentMode(keys, 0, "flat");
      applyCurveTangentMode(keys, 1, "flat");
      return keys;
    },
  },
];

function clampWeight(value: number): number {
  if (!Number.isFinite(value)) return DEFAULT_WEIGHT;
  return Math.max(0, Math.min(1, value));
}
