import type { InspectorField, InspectorPanel } from "../../types";

const MODEL_EXTENSIONS = new Set([
  "3ds",
  "blend",
  "dae",
  "dxf",
  "fbx",
  "ma",
  "mb",
  "max",
  "obj",
]);

const PREFAB_INSTANCE_SUBTITLE_RE = /^Prefab Instance\s*[·•]\s*(.+)$/i;

export interface GameObjectHeaderSummary {
  name: string | null;
  tag: string | null;
  layer: string | null;
  active: boolean | null;
  isStatic: boolean | null;
}

export interface InspectorSourceSummary {
  kind: "prefab" | "fbx" | "model" | "asset";
  path: string;
  name: string;
  extension: string | null;
}

function resolveFieldValue(field?: InspectorField | null): string | null {
  const raw = field?.after ?? field?.before;
  if (raw == null) return null;
  const value = raw.trim();
  return value.length > 0 ? value : null;
}

function resolveBoolFlag(field?: InspectorField | null): boolean | null {
  const value = resolveFieldValue(field);
  if (value == null) return null;

  switch (value.toLowerCase()) {
    case "1":
    case "true":
      return true;
    case "0":
    case "false":
      return false;
    default:
      return null;
  }
}

function findField(panel: InspectorPanel | null | undefined, propertyPath: string): InspectorField | null {
  if (!panel) return null;
  return panel.fields.find((field) => field.propertyPath === propertyPath) ?? null;
}

export function buildGameObjectHeaderSummary(
  panel: InspectorPanel | null | undefined,
): GameObjectHeaderSummary {
  return {
    name: resolveFieldValue(findField(panel, "m_Name")),
    tag: resolveFieldValue(findField(panel, "m_TagString")),
    layer: resolveFieldValue(findField(panel, "m_Layer")),
    active: resolveBoolFlag(findField(panel, "m_IsActive")),
    isStatic: resolveBoolFlag(findField(panel, "m_StaticEditorFlags")),
  };
}

export function parsePrefabSourceSummary(
  subtitle: string | null | undefined,
): InspectorSourceSummary | null {
  if (!subtitle) return null;

  const match = subtitle.trim().match(PREFAB_INSTANCE_SUBTITLE_RE);
  const path = match?.[1]?.trim();
  if (!path) return null;

  const segments = path.split(/[\\/]/).filter(Boolean);
  const tail = segments[segments.length - 1] ?? path;
  const extension = tail.includes(".") ? tail.split(".").pop()?.toLowerCase() ?? null : null;

  let kind: InspectorSourceSummary["kind"] = "asset";
  if (extension === "prefab") {
    kind = "prefab";
  } else if (extension === "fbx") {
    kind = "fbx";
  } else if (extension && MODEL_EXTENSIONS.has(extension)) {
    kind = "model";
  }

  return {
    kind,
    path,
    name: tail.replace(/\.[^.\\/]+$/, ""),
    extension,
  };
}
