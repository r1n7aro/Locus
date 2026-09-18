import type { WorkspaceRef } from "../../services/project";
import type { ViewPackageSummary } from "../../services/view";

export const VIEW_TREE_INTERNAL_DRAG_TYPE = "locus/view-tree";

export interface WorkspaceViewReference {
  projectId: string;
  workspaceRef: WorkspaceRef;
  view: ViewPackageSummary;
}

export interface ViewWorkspaceDragPayload {
  workspaceView?: WorkspaceViewReference | null;
}

export function viewWorkspaceDragReference(
  projectId: string | null | undefined,
  workspaceRef: WorkspaceRef | null,
  view: ViewPackageSummary | undefined,
): WorkspaceViewReference | null {
  if (!projectId?.trim() || !workspaceRef || !view?.id.trim()) return null;
  return { projectId: projectId.trim(), workspaceRef: { ...workspaceRef }, view };
}
