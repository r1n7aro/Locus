import type { ViewWorkspaceRef } from "./view";

export const VIEW_WORKBENCH_OPEN_EVENT = "view-workbench-open";

export interface ViewWorkbenchOpenPayload {
  viewId: string;
  title: string;
  targetLabel: string;
  workspaceRef: ViewWorkspaceRef;
}
