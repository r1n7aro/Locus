import type { UnitySerializedPropertyTarget } from "./unitySerializedProperty";

export type UnityPropertyPathTargetKind =
  | "selection"
  | "guid"
  | "asset"
  | "scene"
  | "prefab"
  | "gameObject"
  | "component";

const PROPERTY_SEGMENT = "property";
const OBJECT_SEGMENT = "object";
const COMPONENT_SEGMENT = "component";

function normalizeSegmentPath(value: string | null | undefined): string {
  return (value ?? "")
    .trim()
    .replace(/\\/g, "/")
    .replace(/^\/+|\/+$/g, "");
}

function splitPath(value: string): string[] {
  return normalizeSegmentPath(value).split("/").filter(Boolean);
}

function findSegment(parts: string[], segment: string, start = 0): number {
  const normalized = segment.toLowerCase();
  for (let index = start; index < parts.length; index += 1) {
    if (parts[index].toLowerCase() === normalized) return index;
  }
  return -1;
}

function parseIntegerSegment(value: string | undefined, fallback: number): number {
  if (value == null || value.trim() === "") return fallback;
  const parsed = Number(value);
  return Number.isInteger(parsed) && parsed >= 0 ? parsed : fallback;
}

function targetWithProperty(
  target: UnitySerializedPropertyTarget,
  propertyPath: string,
): UnitySerializedPropertyTarget {
  return {
    ...target,
    propertyPath: propertyPath.trim(),
  };
}

function targetWithoutProperty(target: UnitySerializedPropertyTarget): UnitySerializedPropertyTarget {
  const next = { ...target };
  delete next.propertyPath;
  return next;
}

function parsePropertySuffix(parts: string[]): {
  targetParts: string[];
  propertyPath: string;
} {
  const propertyIndex = findSegment(parts, PROPERTY_SEGMENT);
  if (propertyIndex < 0) {
    throw new Error("Unity property path must include /property/<propertyPath>.");
  }
  const propertyPath = parts.slice(propertyIndex + 1).join(".").trim();
  if (!propertyPath) {
    throw new Error("Unity property path property segment cannot be empty.");
  }
  return {
    targetParts: parts.slice(0, propertyIndex),
    propertyPath,
  };
}

function parseComponentTarget(
  base: UnitySerializedPropertyTarget,
  parts: string[],
): UnitySerializedPropertyTarget {
  const componentIndex = findSegment(parts, COMPONENT_SEGMENT);
  if (componentIndex < 0) return base;

  const componentType = parts[componentIndex + 1]?.trim();
  if (!componentType) {
    throw new Error("Unity component path requires /component/<type>.");
  }
  const componentOrdinal = parseIntegerSegment(parts[componentIndex + 2], 0);
  const objectPath = parts.slice(0, componentIndex).join("/");
  return {
    ...base,
    kind: "component",
    objectPath: objectPath || base.objectPath,
    componentType,
    componentIndex: componentOrdinal,
  };
}

function parseObjectTarget(
  base: UnitySerializedPropertyTarget,
  parts: string[],
): UnitySerializedPropertyTarget {
  if (parts.length === 0) return base;
  return parseComponentTarget({
    ...base,
    kind: base.kind === "asset" ? "gameObject" : base.kind,
    objectPath: parts.join("/"),
  }, parts);
}

function parseAssetLikeTarget(
  kind: "asset" | "prefab",
  targetParts: string[],
  base: Partial<UnitySerializedPropertyTarget> = {},
): UnitySerializedPropertyTarget {
  const objectIndex = findSegment(targetParts, OBJECT_SEGMENT);
  if (objectIndex < 0) {
    const path = targetParts.join("/");
    if (!path && !base.guid) throw new Error("Unity asset property path requires an asset path.");
    return {
      kind: "asset",
      ...base,
      ...(path ? { path } : {}),
    };
  }

  const path = targetParts.slice(0, objectIndex).join("/");
  if (!path && !base.guid) throw new Error("Unity asset object property path requires an asset path.");
  const objectParts = targetParts.slice(objectIndex + 1);
  return parseObjectTarget({
    kind: kind === "prefab" ? "gameObject" : "asset",
    ...base,
    ...(path ? { path } : {}),
  }, objectParts);
}

function parseGuidTarget(targetParts: string[]): UnitySerializedPropertyTarget {
  const guid = targetParts[0]?.trim();
  if (!guid) {
    throw new Error("Unity GUID property path requires /guid/<guid>.");
  }
  return parseAssetLikeTarget("asset", targetParts.slice(1), { guid });
}

function parseSceneTarget(targetParts: string[]): UnitySerializedPropertyTarget {
  const objectIndex = findSegment(targetParts, OBJECT_SEGMENT);
  if (objectIndex < 0) {
    throw new Error("Unity scene property path requires /object/<objectPath>.");
  }

  const scenePath = targetParts.slice(0, objectIndex).join("/");
  const objectParts = targetParts.slice(objectIndex + 1);
  return parseObjectTarget({
    kind: "gameObject",
    scenePath: scenePath || undefined,
  }, objectParts);
}

function parseSelectionTarget(targetParts: string[]): UnitySerializedPropertyTarget {
  if (targetParts.length === 0) return { kind: "selection" };
  return parseComponentTarget({ kind: "selection" }, targetParts);
}

export function parseUnityPropertyPath(value: string): UnitySerializedPropertyTarget {
  const parts = splitPath(value);
  if (parts.length === 0) {
    throw new Error("Unity property path cannot be empty.");
  }

  const kind = parts[0].toLowerCase();
  const { targetParts, propertyPath } = parsePropertySuffix(parts.slice(1));
  let target: UnitySerializedPropertyTarget;
  if (kind === "selection") {
    target = parseSelectionTarget(targetParts);
  } else if (kind === "guid") {
    target = parseGuidTarget(targetParts);
  } else if (kind === "asset" || kind === "prefab") {
    target = parseAssetLikeTarget(kind, targetParts);
  } else if (kind === "scene" || kind === "gameobject" || kind === "component") {
    target = parseSceneTarget(targetParts);
  } else {
    throw new Error(`Unsupported Unity property path kind: ${parts[0]}`);
  }

  return targetWithProperty(target, propertyPath);
}

export function resolveUnityPropertyTarget(
  input: string | UnitySerializedPropertyTarget,
): UnitySerializedPropertyTarget {
  return typeof input === "string" ? parseUnityPropertyPath(input) : { ...input };
}

export function unityPropertyObjectTarget(
  input: string | UnitySerializedPropertyTarget,
): UnitySerializedPropertyTarget {
  return targetWithoutProperty(resolveUnityPropertyTarget(input));
}

export function unityPropertyTargetWithPath(
  input: string | UnitySerializedPropertyTarget,
  propertyPath: string,
): UnitySerializedPropertyTarget {
  return targetWithProperty(unityPropertyObjectTarget(input), propertyPath);
}

export function unityPropertyTargetKey(
  input: string | UnitySerializedPropertyTarget | null | undefined,
): string {
  if (!input) return "";
  const target = resolveUnityPropertyTarget(input);
  return JSON.stringify({
    kind: target.kind,
    path: target.path ?? "",
    scenePath: target.scenePath ?? "",
    guid: target.guid ?? "",
    objectPath: target.objectPath ?? "",
    objectFileId: target.objectFileId ?? 0,
    targetFileId: target.targetFileId ?? 0,
    componentType: target.componentType ?? "",
    componentIndex: target.componentIndex ?? 0,
    propertyPath: target.propertyPath ?? "",
  });
}
