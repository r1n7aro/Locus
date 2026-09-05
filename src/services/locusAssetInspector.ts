import { emitTo } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { hasTauriWindowRuntime } from "./tauriRuntime";
import type { WorkspaceRef } from "./project";
import { isWorkbenchWindowLabel } from "./workbenchWindow";

export const LOCUS_ASSET_INSPECTOR_TITLE = "Locus Inspector";
export const WORKBENCH_INSPECTOR_OPEN_EVENT = "workbench-inspector-open";

export interface LocusAssetInspectorTarget {
  kind?: "asset" | "sceneObject";
  assetPath?: string;
  scenePath?: string;
  objectPath?: string;
}

export interface WorkbenchInspectorOpenPayload {
  targetLabel: string;
  workspaceRef: WorkspaceRef;
  inspector: LocusAssetInspectorTarget;
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

export function normalizeLocusAssetInspectorTarget(
  target: LocusAssetInspectorTarget,
): LocusAssetInspectorTarget {
  const assetPath = trimOrEmpty(target.assetPath);
  const scenePath = trimOrEmpty(target.scenePath);
  const objectPath = trimOrEmpty(target.objectPath);
  const parsedSceneObject = parseSceneObjectAssetPath(assetPath);
  const resolvedScenePath = scenePath || parsedSceneObject?.scenePath || "";
  const resolvedObjectPath = objectPath || parsedSceneObject?.objectPath || "";

  if (
    target.kind === "sceneObject"
    || (!!scenePath && !!objectPath)
    || (!!parsedSceneObject && target.kind !== "asset")
  ) {
    return {
      kind: "sceneObject",
      scenePath: resolvedScenePath,
      objectPath: resolvedObjectPath,
    };
  }

  return { assetPath };
}

export function isValidLocusAssetInspectorTarget(
  target: LocusAssetInspectorTarget,
): boolean {
  if (target.kind === "sceneObject") {
    return !!trimOrEmpty(target.scenePath) && !!trimOrEmpty(target.objectPath);
  }
  return !!trimOrEmpty(target.assetPath);
}

export function locusAssetInspectorTabTitle(
  target: LocusAssetInspectorTarget,
): string {
  const path = target.kind === "sceneObject"
    ? trimOrEmpty(target.objectPath)
    : trimOrEmpty(target.assetPath);
  const segments = path.split("/").filter(Boolean);
  return segments[segments.length - 1] || LOCUS_ASSET_INSPECTOR_TITLE;
}

/** Opens the target as a standard asset or scene-object editor in Workbench. */
export async function openLocusAssetInspectorWorkbenchTab(
  workspaceRef: WorkspaceRef,
  target: LocusAssetInspectorTarget,
): Promise<boolean> {
  if (!hasTauriWindowRuntime()) return false;

  const inspector = normalizeLocusAssetInspectorTarget(target);
  if (!isValidLocusAssetInspectorTarget(inspector)) return false;

  const currentLabel = getCurrentWindow().label;
  const targetLabel = isWorkbenchWindowLabel(currentLabel) ? currentLabel : "main";
  await emitTo<WorkbenchInspectorOpenPayload>(targetLabel, WORKBENCH_INSPECTOR_OPEN_EVENT, {
    targetLabel,
    workspaceRef,
    inspector,
  });
  return true;
}
