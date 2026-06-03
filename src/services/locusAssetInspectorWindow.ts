import { getCurrentWebviewWindow, WebviewWindow } from "@tauri-apps/api/webviewWindow";
import { hasTauriWindowRuntime } from "./tauriRuntime";

export const LOCUS_ASSET_INSPECTOR_WINDOW_LABEL = "locus-asset-inspector";
export const LOCUS_ASSET_INSPECTOR_WINDOW_PATH = "/locus-asset-inspector";
export const LOCUS_ASSET_INSPECTOR_WINDOW_EVENT = "locus-asset-inspector:payload";
export const LOCUS_ASSET_INSPECTOR_WINDOW_FLAG = "locusAssetInspector";
export const LOCUS_ASSET_INSPECTOR_WINDOW_TITLE = "Locus Inspector";

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

function normalizePayload(payload: LocusAssetInspectorWindowPayload): LocusAssetInspectorWindowPayload {
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

function hasValidPayload(payload: LocusAssetInspectorWindowPayload): boolean {
  if (payload.kind === "sceneObject") {
    return !!trimOrEmpty(payload.scenePath) && !!trimOrEmpty(payload.objectPath);
  }
  return !!trimOrEmpty(payload.assetPath);
}

export function isLocusAssetInspectorWindowLocation(
  locationLike: Pick<Location, "pathname" | "search"> = window.location,
): boolean {
  return locationLike.pathname === LOCUS_ASSET_INSPECTOR_WINDOW_PATH
    || locationLike.search.includes(`${LOCUS_ASSET_INSPECTOR_WINDOW_FLAG}=1`);
}

export function getLocusAssetInspectorWindowPayload(
  search = window.location.search,
): LocusAssetInspectorWindowPayload {
  const params = new URLSearchParams(search);
  const kind = params.get("kind");
  return normalizePayload({
    kind: kind === "sceneObject" || kind === "asset" ? kind : undefined,
    assetPath: params.get("assetPath") ?? undefined,
    scenePath: params.get("scenePath") ?? undefined,
    objectPath: params.get("objectPath") ?? undefined,
  });
}

export function buildLocusAssetInspectorWindowUrl(
  payload: LocusAssetInspectorWindowPayload,
): string {
  const nextPayload = normalizePayload(payload);
  const params = new URLSearchParams({
    [LOCUS_ASSET_INSPECTOR_WINDOW_FLAG]: "1",
  });
  if (nextPayload.kind === "sceneObject") {
    params.set("kind", "sceneObject");
    params.set("scenePath", trimOrEmpty(nextPayload.scenePath));
    params.set("objectPath", trimOrEmpty(nextPayload.objectPath));
  } else {
    params.set("assetPath", trimOrEmpty(nextPayload.assetPath));
  }
  return `${LOCUS_ASSET_INSPECTOR_WINDOW_PATH}?${params.toString()}`;
}

export async function openLocusAssetInspectorWindow(
  payload: LocusAssetInspectorWindowPayload,
): Promise<boolean> {
  if (!hasTauriWindowRuntime()) return false;

  const nextPayload = normalizePayload(payload);
  if (!hasValidPayload(nextPayload)) return false;

  const existingWindow = await WebviewWindow.getByLabel(LOCUS_ASSET_INSPECTOR_WINDOW_LABEL);
  if (existingWindow) {
    await existingWindow.emit(LOCUS_ASSET_INSPECTOR_WINDOW_EVENT, nextPayload);
    await existingWindow.setFocus();
    return true;
  }

  await new Promise<void>((resolve, reject) => {
    const inspectorWindow = new WebviewWindow(LOCUS_ASSET_INSPECTOR_WINDOW_LABEL, {
      url: buildLocusAssetInspectorWindowUrl(nextPayload),
      title: LOCUS_ASSET_INSPECTOR_WINDOW_TITLE,
      width: 1040,
      height: 720,
      minWidth: 720,
      minHeight: 480,
      decorations: false,
      resizable: true,
      closable: true,
      minimizable: false,
      maximizable: true,
      parent: getCurrentWebviewWindow(),
      center: true,
      shadow: true,
    });

    inspectorWindow.once("tauri://created", () => {
      resolve();
    });
    inspectorWindow.once("tauri://error", (event) => {
      reject(event);
    });
  });

  return true;
}
