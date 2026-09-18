import type { MergeField } from "../../types";

export interface VectorMergeComponent {
  label: string;
  base: string;
  ours: string;
  theirs: string;
}

const VECTOR_ORDER = ["x", "y", "z", "w", "r", "g", "b", "a"];
const VECTOR_LABELS = new Set(VECTOR_ORDER);
const NUMBER = /^[+-]?(?:(?:\d+(?:\.\d*)?|\.\d+)(?:e[+-]?\d+)?|Infinity|NaN|\.inf|\.nan)$/i;

function orderedKeys(keys: string[]): string[] | null {
  if (keys.length < 2 || keys.length > 4 || new Set(keys).size !== keys.length) return null;
  if (!keys.every(key => VECTOR_LABELS.has(key))) return null;
  const coordinates = keys.every(key => "xyzw".includes(key));
  const colors = keys.every(key => "rgba".includes(key));
  if (!coordinates && !colors) return null;
  return VECTOR_ORDER.filter(key => keys.includes(key));
}

/** Presentation-only recognition of numeric Unity vectors/colors. Reference
 * identities and unknown flow mappings always retain their original text. */
function parseVector(value: string | undefined): Map<string, string> | null {
  if (value == null) return null;
  const inner = value.trim().match(/^\{([^{}\[\]]+)\}$/s)?.[1];
  if (!inner) return null;
  const entries = new Map<string, string>();
  for (const part of inner.split(",")) {
    const match = part.trim().match(/^(\w+):\s*(.+)$/s);
    if (!match) return null;
    const key = match[1].toLowerCase();
    const raw = match[2].trim();
    if (entries.has(key) || !VECTOR_LABELS.has(key) || !NUMBER.test(raw)) return null;
    entries.set(key, raw);
  }
  return orderedKeys([...entries.keys()]) ? entries : null;
}

export function detectMergeVector(field: MergeField): VectorMergeComponent[] | null {
  if (field.children.length > 0) {
    if (!field.children.every(child => child.children.length === 0)) return null;
    const children = new Map(field.children.map(child => [child.label.toLowerCase(), child]));
    const keys = orderedKeys(field.children.map(child => child.label.toLowerCase()));
    if (!keys) return null;
    return keys.map(key => {
      const child = children.get(key)!;
      return { label: key.toUpperCase(), base: child.base ?? "", ours: child.ours ?? "", theirs: child.theirs ?? "" };
    });
  }

  const rawSides = [field.base, field.ours, field.theirs];
  const sides = rawSides.map(parseVector);
  // A present null/string/reference or a changed tuple shape must remain raw.
  if (rawSides.some((raw, index) => raw != null && sides[index] === null)) return null;
  const sample = sides.find(side => side !== null);
  if (!sample) return null;
  const keys = orderedKeys([...sample.keys()])!;
  if (sides.some(side => side !== null && (side.size !== keys.length || keys.some(key => !side.has(key))))) return null;
  return keys.map(key => ({
    label: key.toUpperCase(),
    base: sides[0]?.get(key) ?? "",
    ours: sides[1]?.get(key) ?? "",
    theirs: sides[2]?.get(key) ?? "",
  }));
}

export function formatVectorNumber(value: string | undefined): string {
  if (value == null || value === "") return "-";
  if (!NUMBER.test(value.trim())) return value;
  const number = Number(value);
  // Avoid rounding integers/identifiers if a custom numeric tuple exceeds the
  // JavaScript exact-integer range, including exponent-form values.
  if (!Number.isFinite(number) || Math.abs(number) > Number.MAX_SAFE_INTEGER) return value;
  return Number.isInteger(number) ? String(number) : number.toFixed(3).replace(/0+$/, "").replace(/\.$/, "");
}
