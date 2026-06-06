import type { UnitySerializedPropertyTarget } from "../services/unitySerializedProperty";

export interface UnityPropertyFenceEntry {
  id: string;
  line: number;
  source: string;
  target: UnitySerializedPropertyTarget;
  objectLabel: string;
  objectTitle: string;
  propertyLabel: string;
}

export interface UnityPropertyFenceIssue {
  line: number;
  source: string;
  message: string;
}

export interface UnityPropertyFenceParseResult {
  entries: UnityPropertyFenceEntry[];
  issues: UnityPropertyFenceIssue[];
}

export interface UnityPropertyFenceBlock<TItem = UnityPropertyFenceEntry> {
  id: string;
  entry: UnityPropertyFenceEntry;
  items: TItem[];
}

export type UnityPropertyFenceUnitySelection =
  | { kind: "asset"; path: string }
  | { kind: "sceneObject"; scenePath: string; objectPath: string };

interface UnityObjectPathParts {
  rawPath: string;
  assetPath: string;
  scenePath: string;
  objectPath: string;
  kind: "asset" | "sceneObject" | "prefabObject";
}

interface PropertySelector {
  propertyPath: string;
  targetKind: "default" | "asset" | "gameObject" | "component";
  componentType: string;
  componentIndex: number;
}

const SCENE_OBJECT_PATH_RE = /^((?:Assets|Packages)\/.+?\.unity)(?:\/(.+))?$/i;
const PREFAB_OBJECT_PATH_RE = /^((?:Assets|Packages)\/.+?\.prefab)(?:\/(.+))?$/i;

export function parseUnityPropertyFence(source: string): UnityPropertyFenceParseResult {
  const trimmed = source.trim();
  if (!trimmed) return { entries: [], issues: [] };

  const jsonResult = parseUnityPropertyFenceJson(trimmed);
  if (jsonResult) return jsonResult;

  const entries: UnityPropertyFenceEntry[] = [];
  const issues: UnityPropertyFenceIssue[] = [];
  const lines = source.split(/\r?\n/g);

  lines.forEach((rawLine, index) => {
    const line = rawLine.trim();
    const lineNumber = index + 1;
    if (!line || line.startsWith("//")) return;

    const jsonLineResult = line.startsWith("{") ? parseUnityPropertyFenceJson(line, lineNumber) : null;
    if (jsonLineResult) {
      entries.push(...jsonLineResult.entries);
      issues.push(...jsonLineResult.issues);
      return;
    }

    const parsed = parseUnityPropertyFenceLine(line, lineNumber);
    if ("entry" in parsed) {
      entries.push(parsed.entry);
    } else {
      issues.push(parsed.issue);
    }
  });

  return { entries, issues };
}

export function groupUnityPropertyFenceItems<TItem>(
  items: TItem[],
  entryForItem: (item: TItem) => UnityPropertyFenceEntry,
): UnityPropertyFenceBlock<TItem>[] {
  const blocks: UnityPropertyFenceBlock<TItem>[] = [];
  let activeKey = "";

  items.forEach((item) => {
    const entry = entryForItem(item);
    const key = unityPropertyFenceTargetBlockKey(entry.target);
    const activeBlock = blocks[blocks.length - 1];
    if (activeBlock && key === activeKey) {
      activeBlock.items.push(item);
      return;
    }

    blocks.push({
      id: `${entry.id}:block:${key}`,
      entry,
      items: [item],
    });
    activeKey = key;
  });

  return blocks;
}

export function unityPropertyFenceTargetBlockKey(target: UnitySerializedPropertyTarget): string {
  return [
    target.kind ?? "",
    target.path ?? "",
    target.scenePath ?? "",
    target.objectPath ?? "",
    target.objectFileId ?? "",
    target.targetFileId ?? "",
    target.componentType ?? "",
    target.componentIndex ?? "",
    target.targetTypeFullName ?? "",
    target.targetTypeName ?? "",
  ].join("|");
}

export function unityPropertyFenceUnitySelectionTarget(
  target: UnitySerializedPropertyTarget,
): UnityPropertyFenceUnitySelection | null {
  const scenePath = stringField(target.scenePath);
  const objectPath = stringField(target.objectPath);
  if (scenePath && objectPath) {
    return { kind: "sceneObject", scenePath, objectPath };
  }

  const assetPath = stringField(target.path) || scenePath;
  if (/^(Assets|Packages)\//i.test(assetPath)) {
    return { kind: "asset", path: assetPath };
  }

  return null;
}

function parseUnityPropertyFenceJson(
  source: string,
  fallbackLine = 1,
): UnityPropertyFenceParseResult | null {
  if (!source.startsWith("{") && !source.startsWith("[")) return null;

  let parsed: unknown;
  try {
    parsed = JSON.parse(source);
  } catch {
    return null;
  }

  const items = Array.isArray(parsed)
    ? parsed
    : Array.isArray((parsed as { properties?: unknown }).properties)
      ? (parsed as { properties: unknown[] }).properties
      : [parsed];

  const entries: UnityPropertyFenceEntry[] = [];
  const issues: UnityPropertyFenceIssue[] = [];
  items.forEach((item, index) => {
    const line = fallbackLine + index;
    const sourceText = stringifySourceItem(item);
    const entry = entryFromJsonItem(item, line, sourceText);
    if (entry) {
      entries.push(entry);
    } else {
      issues.push({
        line,
        source: sourceText,
        message: "Invalid Unity property target.",
      });
    }
  });

  return { entries, issues };
}

function stringifySourceItem(item: unknown): string {
  if (typeof item === "string") return item;
  try {
    return JSON.stringify(item);
  } catch {
    return String(item);
  }
}

function entryFromJsonItem(
  item: unknown,
  line: number,
  source: string,
): UnityPropertyFenceEntry | null {
  if (typeof item === "string") {
    const parsed = parseUnityPropertyFenceLine(item, line);
    return "entry" in parsed ? parsed.entry : null;
  }
  if (!item || typeof item !== "object") return null;

  const record = item as Record<string, unknown>;
  const targetValue = record.target && typeof record.target === "object"
    ? record.target as Record<string, unknown>
    : record;
  const target = normalizeJsonTarget(targetValue);
  if (!target) return null;

  const label = stringField(record.label) || stringField(record.name);
  return buildEntry({
    line,
    source,
    target,
    objectLabel: label || targetObjectDisplayLabel(target),
    objectTitle: targetObjectTitle(target) || label || targetObjectDisplayLabel(target),
    propertyLabel: label || propertyPathLeaf(target.propertyPath || ""),
  });
}

function normalizeJsonTarget(record: Record<string, unknown>): UnitySerializedPropertyTarget | null {
  const kind = stringField(record.kind);
  const propertyPath = stringField(record.propertyPath);
  if (!kind || !propertyPath) return null;

  const componentIndex = numberField(record.componentIndex);
  const objectFileId = nonZeroIntegerField(record.objectFileId)
    ?? nonZeroIntegerField(record.gameObjectFileId)
    ?? nonZeroIntegerField(record.fileId);
  const targetFileId = nonZeroIntegerField(record.targetFileId);
  return removeEmptyTargetFields({
    kind,
    path: stringField(record.path) || null,
    scenePath: stringField(record.scenePath) || null,
    objectPath: stringField(record.objectPath) || null,
    objectFileId,
    targetFileId,
    componentType: stringField(record.componentType) || null,
    componentIndex: componentIndex == null ? null : componentIndex,
    propertyPath,
  });
}

function parseUnityPropertyFenceLine(
  line: string,
  lineNumber: number,
): { entry: UnityPropertyFenceEntry } | { issue: UnityPropertyFenceIssue } {
  const source = line.replace(/^@(?=(?:Assets|Packages)\/)/i, "").trim();
  const pipeParts = source.split("|").map((part) => part.trim()).filter(Boolean);
  if (pipeParts.length >= 2) {
    const selector = pipeParts.length === 2
      ? pipeParts[1]
      : `${pipeParts[1]}:${pipeParts.slice(2).join("|").trim()}`;
    return entryFromCompactParts(pipeParts[0], selector, lineNumber, line);
  }

  const hashIndex = source.lastIndexOf("#");
  if (hashIndex <= 0 || hashIndex >= source.length - 1) {
    return {
      issue: {
        line: lineNumber,
        source: line,
        message: "Expected object path and property path separated by # or |.",
      },
    };
  }

  return entryFromCompactParts(
    source.slice(0, hashIndex).trim(),
    source.slice(hashIndex + 1).trim(),
    lineNumber,
    line,
  );
}

function entryFromCompactParts(
  objectPathSource: string,
  selectorSource: string,
  line: number,
  source: string,
): { entry: UnityPropertyFenceEntry } | { issue: UnityPropertyFenceIssue } {
  const objectPath = parseUnityObjectPath(objectPathSource);
  const selector = parsePropertySelector(selectorSource);
  if (!objectPath.rawPath || !selector.propertyPath) {
    return {
      issue: {
        line,
        source,
        message: "Unity property path is incomplete.",
      },
    };
  }

  const target = targetFromCompactParts(objectPath, selector);
  if (!target) {
    return {
      issue: {
        line,
        source,
        message: "Unity property target cannot be resolved from this path.",
      },
    };
  }

  return {
    entry: buildEntry({
      line,
      source,
      target,
      objectLabel: unityObjectDisplayName(objectPath),
      objectTitle: objectPath.rawPath,
      propertyLabel: selector.propertyPath,
    }),
  };
}

function parseUnityObjectPath(source: string): UnityObjectPathParts {
  const normalized = normalizeUnityPath(source);
  const sceneMatch = normalized.match(SCENE_OBJECT_PATH_RE);
  if (sceneMatch) {
    return {
      rawPath: normalized,
      assetPath: "",
      scenePath: sceneMatch[1],
      objectPath: (sceneMatch[2] || "").replace(/^\/+|\/+$/g, ""),
      kind: "sceneObject",
    };
  }

  const prefabMatch = normalized.match(PREFAB_OBJECT_PATH_RE);
  if (prefabMatch) {
    return {
      rawPath: normalized,
      assetPath: prefabMatch[1],
      scenePath: "",
      objectPath: (prefabMatch[2] || "").replace(/^\/+|\/+$/g, ""),
      kind: "prefabObject",
    };
  }

  return {
    rawPath: normalized,
    assetPath: normalized,
    scenePath: "",
    objectPath: "",
    kind: "asset",
  };
}

function parsePropertySelector(source: string): PropertySelector {
  const selector = source.trim();
  const componentDirective = selector.match(/^component\s*[:=]\s*([^:]+):(.+)$/i);
  if (componentDirective) {
    return componentSelector(componentDirective[1], componentDirective[2]);
  }

  const colonIndex = selector.indexOf(":");
  if (colonIndex <= 0) {
    return {
      propertyPath: selector,
      targetKind: "default",
      componentType: "",
      componentIndex: 0,
    };
  }

  const prefix = selector.slice(0, colonIndex).trim();
  const propertyPath = selector.slice(colonIndex + 1).trim();
  const normalizedPrefix = prefix.toLowerCase();
  if (["asset", "mainasset"].includes(normalizedPrefix)) {
    return { propertyPath, targetKind: "asset", componentType: "", componentIndex: 0 };
  }
  if (["gameobject", "game-object", "go"].includes(normalizedPrefix)) {
    return { propertyPath, targetKind: "gameObject", componentType: "", componentIndex: 0 };
  }

  return componentSelector(prefix, propertyPath);
}

function componentSelector(componentSource: string, propertyPathSource: string): PropertySelector {
  const propertyPath = propertyPathSource.trim();
  const componentPrefix = componentSource
    .replace(/^component\s*=/i, "")
    .replace(/^component\s+/i, "")
    .trim();
  const component = componentPrefix.match(/^(.*?)(?:\[(\d+)])?$/);
  return {
    propertyPath,
    targetKind: "component",
    componentType: component?.[1]?.trim() || componentPrefix,
    componentIndex: component?.[2] ? Number(component[2]) : 0,
  };
}

function targetFromCompactParts(
  objectPath: UnityObjectPathParts,
  selector: PropertySelector,
): UnitySerializedPropertyTarget | null {
  const propertyPath = selector.propertyPath;
  if (!propertyPath) return null;

  if (selector.targetKind === "asset") {
    return removeEmptyTargetFields({
      kind: "asset",
      path: objectPath.assetPath || objectPath.rawPath,
      propertyPath,
    });
  }

  if (selector.targetKind === "component") {
    if (!selector.componentType) return null;
    if (objectPath.kind === "sceneObject") {
      return removeEmptyTargetFields({
        kind: "component",
        scenePath: objectPath.scenePath,
        objectPath: objectPath.objectPath,
        componentType: selector.componentType,
        componentIndex: selector.componentIndex,
        propertyPath,
      });
    }
    if (objectPath.kind === "prefabObject") {
      return removeEmptyTargetFields({
        kind: "component",
        path: objectPath.assetPath,
        objectPath: objectPath.objectPath,
        componentType: selector.componentType,
        componentIndex: selector.componentIndex,
        propertyPath,
      });
    }
    return null;
  }

  if (selector.targetKind === "gameObject" || objectPath.kind === "sceneObject" || objectPath.kind === "prefabObject") {
    if (objectPath.kind === "sceneObject") {
      return removeEmptyTargetFields({
        kind: "gameObject",
        scenePath: objectPath.scenePath,
        objectPath: objectPath.objectPath,
        propertyPath,
      });
    }
    if (objectPath.kind === "prefabObject") {
      return removeEmptyTargetFields({
        kind: "gameObject",
        path: objectPath.assetPath,
        objectPath: objectPath.objectPath,
        propertyPath,
      });
    }
  }

  return removeEmptyTargetFields({
    kind: "asset",
    path: objectPath.assetPath,
    propertyPath,
  });
}

function buildEntry(input: Omit<UnityPropertyFenceEntry, "id">): UnityPropertyFenceEntry {
  const targetKey = [
    input.target.kind,
    input.target.path ?? "",
    input.target.scenePath ?? "",
    input.target.objectPath ?? "",
    input.target.objectFileId ?? "",
    input.target.targetFileId ?? "",
    input.target.componentType ?? "",
    input.target.componentIndex ?? "",
    input.target.propertyPath ?? "",
  ].join("|");
  return {
    ...input,
    id: `${input.line}:${targetKey}`,
  };
}

function removeEmptyTargetFields(target: UnitySerializedPropertyTarget): UnitySerializedPropertyTarget {
  return Object.fromEntries(
    Object.entries(target).filter(([, value]) =>
      value !== "" && value !== null && value !== undefined,
    ),
  ) as UnitySerializedPropertyTarget;
}

function targetObjectLabel(target: UnitySerializedPropertyTarget): string {
  if (target.scenePath || target.objectPath) {
    return [target.scenePath, target.objectPath].filter(Boolean).join("/");
  }
  return target.path || target.kind;
}

function targetObjectTitle(target: UnitySerializedPropertyTarget): string {
  return targetObjectLabel(target);
}

function targetObjectDisplayLabel(target: UnitySerializedPropertyTarget): string {
  const objectPath = (target.objectPath || "").trim();
  if (objectPath) return objectPathLeaf(objectPath);
  const path = (target.path || target.scenePath || "").trim();
  if (path) return assetNameFromPath(path);
  return target.kind || "Unity";
}

function unityObjectDisplayName(path: UnityObjectPathParts): string {
  if (path.objectPath) return objectPathLeaf(path.objectPath);
  if (path.assetPath) return assetNameFromPath(path.assetPath);
  if (path.scenePath) return assetNameFromPath(path.scenePath);
  return assetNameFromPath(path.rawPath);
}

function objectPathLeaf(path: string): string {
  const normalized = path.replace(/\/+$/g, "");
  const slash = normalized.lastIndexOf("/");
  return slash >= 0 ? normalized.slice(slash + 1) : normalized;
}

function assetNameFromPath(path: string): string {
  const leaf = objectPathLeaf(path);
  const dot = leaf.lastIndexOf(".");
  return dot > 0 ? leaf.slice(0, dot) : leaf;
}

function propertyPathLeaf(propertyPath: string): string {
  const normalized = propertyPath.trim();
  if (!normalized) return "";
  const match = normalized.match(/(?:^|\.)([^.[\]]+)(?:\[\d+])?$/);
  return match?.[1] || normalized;
}

function normalizeUnityPath(source: string): string {
  return source
    .trim()
    .replace(/^@/, "")
    .replace(/\\/g, "/")
    .replace(/#fileID:-?\d+$/i, "")
    .replace(/\/+$/g, "");
}

function stringField(value: unknown): string {
  return typeof value === "string" ? value.trim() : "";
}

function numberField(value: unknown): number | null {
  if (typeof value === "number" && Number.isInteger(value) && value >= 0) return value;
  if (typeof value === "string" && /^\d+$/.test(value.trim())) return Number(value.trim());
  return null;
}

function nonZeroIntegerField(value: unknown): number | null {
  if (typeof value === "number" && Number.isInteger(value) && value !== 0) return value;
  if (typeof value === "string" && /^-?\d+$/.test(value.trim())) {
    const parsed = Number(value.trim());
    return parsed === 0 ? null : parsed;
  }
  return null;
}
