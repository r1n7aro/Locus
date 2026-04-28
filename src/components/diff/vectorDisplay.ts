import type { InspectorField } from "../../types";

export interface VectorComponent {
  key: string;
  label: string;
  before: string | undefined;
  after: string | undefined;
  changeKind: InspectorField["changeKind"];
}

const VECTOR_KEYS = ["x", "y", "z", "w"];
const COLOR_KEYS = ["r", "g", "b", "a"];
const VECTOR_LABELS = new Set([...VECTOR_KEYS, ...COLOR_KEYS]);

function componentOrder(components: VectorComponent[]): string[] {
  const keys = new Set(components.map((component) => component.key));
  if (components.every((component) => VECTOR_KEYS.includes(component.key))) {
    return VECTOR_KEYS.filter((key) => keys.has(key));
  }
  if (components.every((component) => COLOR_KEYS.includes(component.key))) {
    return COLOR_KEYS.filter((key) => keys.has(key));
  }
  return components.map((component) => component.key);
}

export function orderedVectorComponents(components: VectorComponent[]): VectorComponent[] {
  const byKey = new Map(components.map((component) => [component.key, component]));
  return componentOrder(components)
    .map((key) => byKey.get(key))
    .filter((component): component is VectorComponent => !!component);
}

export function detectVectorComponents(field: InspectorField): VectorComponent[] | null {
  const children = field.children;
  if (!children || children.length < 2 || children.length > 4) return null;

  const keys = new Set<string>();
  const components: VectorComponent[] = [];
  for (const child of children) {
    const key = child.label.toLowerCase();
    if (child.children?.length || !VECTOR_LABELS.has(key) || keys.has(key)) return null;
    keys.add(key);
    components.push({
      key,
      label: key.toUpperCase(),
      before: child.before ?? undefined,
      after: child.after ?? undefined,
      changeKind: child.changeKind,
    });
  }

  const vectorLike = components.every((component) => VECTOR_KEYS.includes(component.key));
  const colorLike = components.every((component) => COLOR_KEYS.includes(component.key));
  return vectorLike || colorLike ? components : null;
}

export function isFullyModifiedVector(components: VectorComponent[]): boolean {
  return components.length >= 2
    && components.every(
      (component) =>
        component.changeKind === "modified"
        && component.before !== undefined
        && component.after !== undefined,
    );
}

export function formatVectorTuple(
  components: VectorComponent[],
  side: "before" | "after",
  formatValue: (value: string | undefined) => string,
): string {
  const values = orderedVectorComponents(components).map((component) =>
    formatValue(side === "before" ? component.before : component.after),
  );
  return `(${values.join(", ")})`;
}

export function isQuaternionField(
  field: InspectorField,
  components: VectorComponent[],
): boolean {
  if (components.length !== 4) return false;
  if (!VECTOR_KEYS.every((key) => components.some((component) => component.key === key))) {
    return false;
  }

  if (field.fieldType === "Quaternion") return true;
  const pathLeaf = field.propertyPath.split(".").pop() ?? field.propertyPath;
  return pathLeaf === "m_LocalRotation"
    || pathLeaf === "m_Rotation"
    || /\brotation\b/i.test(field.label);
}

interface QuaternionValue {
  x: number;
  y: number;
  z: number;
  w: number;
}

function parseFiniteNumber(value: string | undefined): number | null {
  if (value == null) return null;
  const parsed = Number.parseFloat(value);
  return Number.isFinite(parsed) ? parsed : null;
}

function quaternionFromComponents(
  components: VectorComponent[],
  side: "before" | "after",
): QuaternionValue | null {
  const values = new Map<string, number>();
  for (const component of components) {
    const value = parseFiniteNumber(side === "before" ? component.before : component.after);
    if (value == null) return null;
    values.set(component.key, value);
  }

  const x = values.get("x");
  const y = values.get("y");
  const z = values.get("z");
  const w = values.get("w");
  if (x == null || y == null || z == null || w == null) return null;
  return { x, y, z, w };
}

function clamp(value: number, min: number, max: number): number {
  return Math.min(max, Math.max(min, value));
}

function radiansToDegrees(value: number): number {
  return value * 180 / Math.PI;
}

export function quaternionToEulerDegrees(quaternion: QuaternionValue): [number, number, number] | null {
  const length = Math.hypot(quaternion.x, quaternion.y, quaternion.z, quaternion.w);
  if (!Number.isFinite(length) || length === 0) return null;

  const x = quaternion.x / length;
  const y = quaternion.y / length;
  const z = quaternion.z / length;
  const w = quaternion.w / length;

  const eulerX = Math.asin(clamp(2 * (w * x - y * z), -1, 1));
  const eulerY = Math.atan2(2 * (w * y + x * z), 1 - 2 * (x * x + y * y));
  const eulerZ = Math.atan2(2 * (w * z + x * y), 1 - 2 * (x * x + z * z));
  return [radiansToDegrees(eulerX), radiansToDegrees(eulerY), radiansToDegrees(eulerZ)];
}

export function formatQuaternionEulerTuple(
  components: VectorComponent[],
  side: "before" | "after",
  formatNumber: (value: number) => string,
): string | null {
  const quaternion = quaternionFromComponents(components, side);
  if (!quaternion) return null;
  const euler = quaternionToEulerDegrees(quaternion);
  if (!euler) return null;
  return `(${euler.map(formatNumber).join(", ")})`;
}
