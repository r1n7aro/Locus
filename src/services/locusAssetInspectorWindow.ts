import { hasTauriWindowRuntime } from "./tauriRuntime";
import { viewOpenInspectorTab } from "./view";

export const LOCUS_ASSET_INSPECTOR_WINDOW_TITLE = "Locus Inspector";

/**
 * Inspector targets live inside the View host tab system. A tab id encodes the
 * inspected target as URL params behind this prefix, e.g.
 * `locus-inspector:assetPath=Assets%2FFoo.prefab`. The id doubles as the
 * dedupe key: opening the same target focuses the already-hosted tab.
 */
export const LOCUS_ASSET_INSPECTOR_TAB_ID_PREFIX = "locus-inspector:";

export interface LocusAssetInspectorWindowPayload {
  kind?: "asset" | "sceneObject";
  assetPath?: string;
  scenePath?: string;
  objectPath?: string;
}

function trimOrEmpty(value: string | null | undefined): string {
  return value?.trim().replace(/\\/g, "/").replace(/\/+$/, "") || "";
}

function parseSceneObjectAssetPath(assetPath: string): { scenePath: string; objectPath: string } | null {
  const match = assetPath.match(/^((?:Assets|Packages)\/.+?\.unity)\/(.+)$/i);
  const scenePath = trimOrEmpty(match?.[1]);
  const objectPath = trimOrEmpty(match?.[2]);
  return scenePath && objectPath ? { scenePath, objectPath } : null;
}

export function normalizeLocusAssetInspectorPayload(
  payload: LocusAssetInspectorWindowPayload,
): LocusAssetInspectorWindowPayload {
  const assetPath = trimOrEmpty(payload.assetPath);
  const scenePath = trimOrEmpty(payload.scenePath);
  const objectPath = trimOrEmpty(payload.objectPath);
  const parsedSceneObject = parseSceneObjectAssetPath(assetPath);
  const resolvedScenePath = scenePath || parsedSceneObject?.scenePath || "";
  const resolvedObjectPath = objectPath || parsedSceneObject?.objectPath || "";

  if (
    payload.kind === "sceneObject" ||
    (!!scenePath && !!objectPath) ||
    (!!parsedSceneObject && payload.kind !== "asset")
  ) {
    return {
      kind: "sceneObject",
      scenePath: resolvedScenePath,
      objectPath: resolvedObjectPath,
    };
  }

  return { assetPath };
}

export function isValidLocusAssetInspectorPayload(
  payload: LocusAssetInspectorWindowPayload,
): boolean {
  if (payload.kind === "sceneObject") {
    return !!trimOrEmpty(payload.scenePath) && !!trimOrEmpty(payload.objectPath);
  }
  return !!trimOrEmpty(payload.assetPath);
}

export function isLocusAssetInspectorTabId(tabId: string | null | undefined): boolean {
  return (tabId ?? "").startsWith(LOCUS_ASSET_INSPECTOR_TAB_ID_PREFIX);
}

export function buildLocusAssetInspectorTabId(
  payload: LocusAssetInspectorWindowPayload,
): string {
  const nextPayload = normalizeLocusAssetInspectorPayload(payload);
  const params = new URLSearchParams();
  if (nextPayload.kind === "sceneObject") {
    params.set("kind", "sceneObject");
    params.set("scenePath", trimOrEmpty(nextPayload.scenePath));
    params.set("objectPath", trimOrEmpty(nextPayload.objectPath));
  } else {
    params.set("assetPath", trimOrEmpty(nextPayload.assetPath));
  }
  return `${LOCUS_ASSET_INSPECTOR_TAB_ID_PREFIX}${params.toString()}`;
}

export function parseLocusAssetInspectorTabId(
  tabId: string,
): LocusAssetInspectorWindowPayload | null {
  if (!isLocusAssetInspectorTabId(tabId)) return null;
  const params = new URLSearchParams(tabId.slice(LOCUS_ASSET_INSPECTOR_TAB_ID_PREFIX.length));
  const kind = params.get("kind");
  const payload = normalizeLocusAssetInspectorPayload({
    kind: kind === "sceneObject" || kind === "asset" ? kind : undefined,
    assetPath: params.get("assetPath") ?? undefined,
    scenePath: params.get("scenePath") ?? undefined,
    objectPath: params.get("objectPath") ?? undefined,
  });
  return isValidLocusAssetInspectorPayload(payload) ? payload : null;
}

export function locusAssetInspectorTargetPath(
  payload: LocusAssetInspectorWindowPayload,
): string {
  if (payload.kind === "sceneObject") {
    const scenePath = trimOrEmpty(payload.scenePath);
    const objectPath = trimOrEmpty(payload.objectPath);
    return scenePath && objectPath ? `${scenePath}/${objectPath}` : "";
  }
  return trimOrEmpty(payload.assetPath);
}

export function locusAssetInspectorTabTitle(
  payload: LocusAssetInspectorWindowPayload,
): string {
  const path = payload.kind === "sceneObject"
    ? trimOrEmpty(payload.objectPath)
    : trimOrEmpty(payload.assetPath);
  const segments = path.split("/").filter(Boolean);
  return segments[segments.length - 1] || LOCUS_ASSET_INSPECTOR_WINDOW_TITLE;
}

/**
 * Opens the target as an inspector tab in the View host window system. The
 * backend focuses an existing tab for the same target, prefers adding a tab
 * to an already-open host window, and only then creates a new window.
 */
export async function openLocusAssetInspectorWindow(
  payload: LocusAssetInspectorWindowPayload,
): Promise<boolean> {
  if (!hasTauriWindowRuntime()) return false;

  const nextPayload = normalizeLocusAssetInspectorPayload(payload);
  if (!isValidLocusAssetInspectorPayload(nextPayload)) return false;

  await viewOpenInspectorTab({ tabId: buildLocusAssetInspectorTabId(nextPayload) });
  return true;
}
