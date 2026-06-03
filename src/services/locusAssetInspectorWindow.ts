import { getCurrentWebviewWindow, WebviewWindow } from "@tauri-apps/api/webviewWindow";
import { hasTauriWindowRuntime } from "./tauriRuntime";

export const LOCUS_ASSET_INSPECTOR_WINDOW_LABEL = "locus-asset-inspector";
export const LOCUS_ASSET_INSPECTOR_WINDOW_PATH = "/locus-asset-inspector";
export const LOCUS_ASSET_INSPECTOR_WINDOW_EVENT = "locus-asset-inspector:payload";
export const LOCUS_ASSET_INSPECTOR_WINDOW_FLAG = "locusAssetInspector";
export const LOCUS_ASSET_INSPECTOR_WINDOW_TITLE = "Locus Inspector";

export interface LocusAssetInspectorWindowPayload {
  assetPath: string;
}

function trimOrEmpty(value: string | null | undefined): string {
  return value?.trim().replace(/\\/g, "/").replace(/\/+$/, "") || "";
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
  return {
    assetPath: trimOrEmpty(params.get("assetPath")),
  };
}

export function buildLocusAssetInspectorWindowUrl(
  payload: LocusAssetInspectorWindowPayload,
): string {
  const params = new URLSearchParams({
    [LOCUS_ASSET_INSPECTOR_WINDOW_FLAG]: "1",
    assetPath: trimOrEmpty(payload.assetPath),
  });
  return `${LOCUS_ASSET_INSPECTOR_WINDOW_PATH}?${params.toString()}`;
}

export async function openLocusAssetInspectorWindow(
  payload: LocusAssetInspectorWindowPayload,
): Promise<boolean> {
  if (!hasTauriWindowRuntime()) return false;

  const nextPayload = {
    assetPath: trimOrEmpty(payload.assetPath),
  };
  if (!nextPayload.assetPath) return false;

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
