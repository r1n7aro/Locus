export type UnitySerializedPropertyType =
  | "Integer"
  | "Boolean"
  | "Float"
  | "String"
  | "Enum"
  | "ObjectReference"
  | "LayerMask"
  | "ArraySize"
  | "Vector2"
  | "Vector3"
  | "Vector4"
  | "Quaternion"
  | "Color"
  | "Rect"
  | string;

export interface UnitySelectOption {
  label: string;
  value: string;
  name?: string;
  index?: number;
  numericValue?: number;
}

export interface UnityManagedReferenceTypeOption {
  label: string;
  value: string;
  fullName?: string;
  assembly?: string;
}

export interface UnitySerializedPropertyAttributeInfo {
  type?: string;
  displayName?: string;
  value?: string;
}

export interface UnitySerializedPropertyTargetSnapshot {
  kind: string;
  guid?: string | null;
  path?: string | null;
  scenePath?: string | null;
  objectPath?: string | null;
  objectFileId?: number | null;
  targetFileId?: number | null;
  componentType?: string | null;
  componentIndex?: number | null;
  targetTypeFullName?: string | null;
  targetTypeAssembly?: string | null;
  targetTypeName?: string | null;
  propertyPath?: string | null;
}

export interface UnitySerializedPropertySnapshot {
  propertyPath: string;
  bindingTarget?: UnitySerializedPropertyTargetSnapshot | null;
  target?: UnitySerializedPropertyTargetSnapshot | null;
  displayName?: string;
  name?: string;
  type?: string;
  valueType?: string;
  fieldTypeFullName?: string;
  fieldTypeAssembly?: string;
  value: unknown;
  displayValue?: string;
  editable?: boolean;
  hasChildren?: boolean;
  isArray?: boolean;
  arraySize?: number;
  isFlagsEnum?: boolean;
  enumValueIndex?: number;
  enumValueFlag?: number;
  enumOptions?: UnitySelectOption[];
  children?: UnitySerializedPropertySnapshot[];
  isManagedReference?: boolean;
  managedReferenceFullTypename?: string;
  managedReferenceFieldTypename?: string;
  managedReferenceDisplayName?: string;
  managedReferenceTypes?: UnityManagedReferenceTypeOption[];
  tooltip?: string;
  header?: string;
  hasRange?: boolean;
  rangeMin?: number;
  rangeMax?: number;
  numberStep?: number;
  multiline?: boolean;
  minLines?: number;
  maxLines?: number;
  referenceTypeFullName?: string;
  referenceTypeAssembly?: string;
  attributes?: UnitySerializedPropertyAttributeInfo[];
}

export interface UnitySerializedPropertyCommitEvent {
  propertyPath: string;
  value: unknown;
  property: UnitySerializedPropertySnapshot;
  target?: UnitySerializedPropertyTargetSnapshot | null;
  writeMode?: "commit" | "preview";
}

export interface UnityParseResult {
  ok: boolean;
  value: unknown;
  message?: string;
}

export interface UnityNumberConstraintOptions {
  hasRange?: boolean;
  rangeMin?: number;
  rangeMax?: number;
}

type VectorKey = "x" | "y" | "z" | "w" | "width" | "height";

const VECTOR_KEYS: Record<string, VectorKey[]> = {
  Vector2: ["x", "y"],
  Vector3: ["x", "y", "z"],
  Vector4: ["x", "y", "z", "w"],
  Quaternion: ["x", "y", "z"],
  Rect: ["x", "y", "width", "height"],
};

export function normalizeUnityPropertyType(type: string | null | undefined): string {
  return (type || "String").trim() || "String";
}

export function isUnityIntegerPropertyType(type: string | null | undefined): boolean {
  return ["Integer", "ArraySize", "LayerMask"].includes(normalizeUnityPropertyType(type));
}

export function isUnityNumberPropertyType(type: string | null | undefined): boolean {
  return isUnityIntegerPropertyType(type) || normalizeUnityPropertyType(type) === "Float";
}

export function isUnityVectorPropertyType(type: string | null | undefined): boolean {
  return Object.prototype.hasOwnProperty.call(VECTOR_KEYS, normalizeUnityPropertyType(type));
}

export function isUnityQuaternionPropertyType(type: string | null | undefined): boolean {
  return normalizeUnityPropertyType(type) === "Quaternion";
}

export function unityVectorKeysForType(type: string | null | undefined): VectorKey[] {
  return VECTOR_KEYS[normalizeUnityPropertyType(type)] ?? [];
}

export function normalizeUnityOptions(options: UnitySelectOption[] | null | undefined): UnitySelectOption[] {
  return (options ?? []).map((option) => {
    const index = Number.isFinite(option.index) ? Number(option.index) : undefined;
    const numericValue = Number.isFinite(option.numericValue) ? Number(option.numericValue) : undefined;
    const fallbackValue = index != null ? String(index) : option.name || option.label;
    return {
      label: option.label || option.value,
      value: option.value || fallbackValue,
      name: option.name,
      index,
      numericValue,
    };
  });
}

export function unitySerializedValueToEditText(
  type: string | null | undefined,
  value: unknown,
  displayValue = "",
): string {
  const normalized = normalizeUnityPropertyType(type);
  if (value == null) return displayValue;
  if (normalized === "Boolean") return value === true ? "true" : "false";
  if (normalized === "Enum") return formatUnityEnumValue(value, displayValue);
  if (isUnityVectorPropertyType(normalized)) return formatUnityVectorValue(normalized, value, displayValue);
  if (normalized === "Color") return formatUnityColorValue(value, displayValue);
  if (typeof value === "object") return displayValue || JSON.stringify(value);
  return String(value);
}

export function tryParseUnitySerializedEditValue(
  type: string | null | undefined,
  rawValue: string | boolean | number | unknown,
): UnityParseResult {
  try {
    return {
      ok: true,
      value: parseUnitySerializedEditValue(type, rawValue),
    };
  } catch (error) {
    return {
      ok: false,
      value: rawValue,
      message: error instanceof Error ? error.message : String(error),
    };
  }
}

export function parseUnitySerializedEditValue(
  type: string | null | undefined,
  rawValue: string | boolean | number | unknown,
): unknown {
  const normalized = normalizeUnityPropertyType(type);
  if (normalized === "Boolean") {
    if (typeof rawValue === "boolean") return rawValue;
    const text = String(rawValue ?? "").trim().toLowerCase();
    if (text === "true" || text === "1" || text === "yes" || text === "on") return true;
    if (text === "false" || text === "0" || text === "no" || text === "off") return false;
    throw new Error("Expected boolean value");
  }

  if (isUnityIntegerPropertyType(normalized)) {
    return parseUnityInteger(rawValue);
  }

  if (normalized === "Float") {
    return parseUnityNumber(rawValue);
  }

  if (isUnityVectorPropertyType(normalized)) {
    return parseUnityVectorValue(normalized, rawValue);
  }

  if (normalized === "Color") {
    return parseUnityColorValue(rawValue);
  }

  if (normalized === "ManagedReference") {
    return rawValue;
  }

  if (rawValue == null) return "";
  return String(rawValue);
}

export function constrainUnityNumberValue(
  type: string | null | undefined,
  value: number,
  options: UnityNumberConstraintOptions = {},
): number {
  if (!Number.isFinite(value)) throw new Error("Expected number value");
  let next = value;
  if (
    options.hasRange === true &&
    Number.isFinite(options.rangeMin) &&
    Number.isFinite(options.rangeMax)
  ) {
    const min = Math.min(Number(options.rangeMin), Number(options.rangeMax));
    const max = Math.max(Number(options.rangeMin), Number(options.rangeMax));
    next = Math.max(min, Math.min(max, next));
  }
  if (isUnityIntegerPropertyType(type)) next = Math.round(next);
  return normalizeUnityNumberPrecision(next);
}

export function formatUnityNumberValue(
  type: string | null | undefined,
  value: number,
  options: UnityNumberConstraintOptions = {},
): string {
  return String(constrainUnityNumberValue(type, value, options));
}

export function formatUnityEnumValue(value: unknown, displayValue = ""): string {
  if (value && typeof value === "object") {
    const record = value as Record<string, unknown>;
    return String(record.name ?? record.label ?? record.value ?? record.index ?? displayValue);
  }
  return displayValue || String(value ?? "");
}

export function unityEnumIndexValue(value: unknown, fallback = -1): number {
  if (typeof value === "number" && Number.isInteger(value)) return value;
  if (value && typeof value === "object") {
    const record = value as Record<string, unknown>;
    const index = Number(record.index);
    if (Number.isInteger(index)) return index;
  }
  return fallback;
}

export function unityEnumNumericValue(value: unknown, fallback = 0): number {
  if (typeof value === "number" && Number.isInteger(value)) return value;
  if (value && typeof value === "object") {
    const record = value as Record<string, unknown>;
    const numericValue = Number(record.numericValue ?? record.value ?? record.index);
    if (Number.isInteger(numericValue)) return numericValue;
  }
  return fallback;
}

export function parseUnityVectorValue(
  type: string | null | undefined,
  rawValue: string | number | boolean | unknown,
): Record<string, number | string> {
  if (isUnityQuaternionPropertyType(type)) {
    return parseUnityQuaternionEulerValue(rawValue);
  }

  const keys = unityVectorKeysForType(type);
  if (!keys.length) throw new Error("Expected vector type");
  if (rawValue && typeof rawValue === "object") {
    const record = rawValue as Record<string, unknown>;
    const next = {} as Record<string, number>;
    for (const key of keys) {
      next[key] = parseUnityNumber(record[key] ?? 0);
    }
    return next;
  }

  const parts = String(rawValue ?? "")
    .trim()
    .split(/[,\s]+/)
    .filter(Boolean);
  if (parts.length !== keys.length) {
    throw new Error("Expected vector components");
  }
  const values = parts.map((part) => parseUnityNumber(part));
  const next = {} as Record<string, number>;
  keys.forEach((key, index) => {
    next[key] = values[index];
  });
  return next;
}

export function formatUnityVectorValue(
  type: string | null | undefined,
  value: unknown,
  displayValue = "",
): string {
  if (isUnityQuaternionPropertyType(type)) return formatUnityQuaternionEulerValue(value, displayValue);

  const keys = unityVectorKeysForType(type);
  if (!keys.length) return displayValue;
  if (value && typeof value === "object") {
    const record = value as Record<string, unknown>;
    return keys.map((key) => String(record[key] ?? 0)).join(", ");
  }
  return displayValue || String(value ?? "");
}

export function parseUnityQuaternionEulerValue(rawValue: unknown): Record<string, number | string> {
  const vector = parseUnityQuaternionEulerVector(rawValue);
  return {
    action: "setEuler",
    x: vector.x,
    y: vector.y,
    z: vector.z,
  };
}

export function formatUnityQuaternionEulerValue(value: unknown, displayValue = ""): string {
  const displayText = displayValue.trim();
  if (displayText) {
    try {
      return formatUnityVectorValue("Vector3", parseUnityVectorValue("Vector3", displayText));
    } catch {
      return displayText;
    }
  }

  try {
    return formatUnityVectorValue("Vector3", parseUnityQuaternionEulerVector(value));
  } catch {
    return typeof value === "string" ? value : "";
  }
}

export function parseUnityColorValue(rawValue: unknown): string {
  if (rawValue && typeof rawValue === "object") {
    const record = rawValue as Record<string, unknown>;
    const r = colorChannelToHex(Number(record.r ?? 0));
    const g = colorChannelToHex(Number(record.g ?? 0));
    const b = colorChannelToHex(Number(record.b ?? 0));
    const a = colorChannelToHex(Number(record.a ?? 1));
    return `#${r}${g}${b}${a}`;
  }
  const text = String(rawValue ?? "").trim();
  if (!text) return "";
  if (/^#[0-9a-fA-F]{6}([0-9a-fA-F]{2})?$/.test(text)) return text;
  throw new Error("Expected color value");
}

export function formatUnityColorValue(value: unknown, displayValue = ""): string {
  if (typeof value === "string") return value;
  if (value && typeof value === "object") return parseUnityColorValue(value);
  return displayValue || "";
}

export function unityColorTextToRgbHex(value: unknown): string {
  const color = formatUnityColorValue(value);
  if (/^#[0-9a-fA-F]{6}/.test(color)) return color.slice(0, 7);
  return "#000000";
}

export function applyUnityRgbHexToColorText(rgbHex: string, previous: unknown): string {
  const previousText = formatUnityColorValue(previous);
  const alpha = /^#[0-9a-fA-F]{8}$/.test(previousText) ? previousText.slice(7, 9) : "ff";
  return `${rgbHex}${alpha}`;
}

function colorChannelToHex(value: number): string {
  const normalized = Number.isFinite(value) ? value : 0;
  const channel = Math.round(Math.max(0, Math.min(1, normalized)) * 255);
  return channel.toString(16).padStart(2, "0");
}

function parseUnityInteger(rawValue: unknown): number {
  if (typeof rawValue === "number") {
    if (Number.isInteger(rawValue)) return rawValue;
    throw new Error("Expected integer value");
  }
  const text = String(rawValue ?? "").trim();
  if (!/^[+-]?\d+$/.test(text)) throw new Error("Expected integer value");
  const value = Number(text);
  if (!Number.isSafeInteger(value)) throw new Error("Expected integer value");
  return value;
}

function parseUnityNumber(rawValue: unknown): number {
  if (typeof rawValue === "number") {
    if (Number.isFinite(rawValue)) return rawValue;
    throw new Error("Expected number value");
  }
  const text = String(rawValue ?? "").trim();
  if (!text) throw new Error("Expected number value");
  const value = Number(text);
  if (!Number.isFinite(value)) throw new Error("Expected number value");
  return value;
}

function parseUnityQuaternionEulerVector(rawValue: unknown): { x: number; y: number; z: number } {
  if (rawValue && typeof rawValue === "object") {
    const record = rawValue as Record<string, unknown>;
    const action = String(record.action ?? "").trim().toLowerCase();
    if (action === "seteuler" || action === "euler") {
      return {
        x: parseUnityNumber(record.x ?? 0),
        y: parseUnityNumber(record.y ?? 0),
        z: parseUnityNumber(record.z ?? 0),
      };
    }
    if ("w" in record) {
      const euler = quaternionToEulerDegrees({
        x: parseUnityNumber(record.x ?? 0),
        y: parseUnityNumber(record.y ?? 0),
        z: parseUnityNumber(record.z ?? 0),
        w: parseUnityNumber(record.w ?? 1),
      });
      if (euler) return euler;
    }
  }

  const parsed = parseUnityVectorValue("Vector3", rawValue);
  return {
    x: Number(parsed.x),
    y: Number(parsed.y),
    z: Number(parsed.z),
  };
}

function quaternionToEulerDegrees(quaternion: { x: number; y: number; z: number; w: number }): {
  x: number;
  y: number;
  z: number;
} | null {
  const length = Math.hypot(quaternion.x, quaternion.y, quaternion.z, quaternion.w);
  if (!Number.isFinite(length) || length === 0) return null;

  const x = quaternion.x / length;
  const y = quaternion.y / length;
  const z = quaternion.z / length;
  const w = quaternion.w / length;

  const eulerX = Math.asin(clamp(2 * (w * x - y * z), -1, 1));
  const eulerY = Math.atan2(2 * (w * y + x * z), 1 - 2 * (x * x + y * y));
  const eulerZ = Math.atan2(2 * (w * z + x * y), 1 - 2 * (x * x + z * z));
  return {
    x: normalizeEulerDegrees(radiansToDegrees(eulerX)),
    y: normalizeEulerDegrees(radiansToDegrees(eulerY)),
    z: normalizeEulerDegrees(radiansToDegrees(eulerZ)),
  };
}

function clamp(value: number, min: number, max: number): number {
  return Math.min(max, Math.max(min, value));
}

function radiansToDegrees(value: number): number {
  return value * 180 / Math.PI;
}

function normalizeEulerDegrees(value: number): number {
  const normalized = normalizeUnityNumberPrecision(value % 360);
  return normalized < 0 ? normalizeUnityNumberPrecision(normalized + 360) : normalized;
}

function normalizeUnityNumberPrecision(value: number): number {
  if (Object.is(value, -0)) return 0;
  if (Number.isInteger(value)) return value;
  return Number(value.toPrecision(12));
}
