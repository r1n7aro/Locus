import type { WorkspaceRef } from "./project";

export interface FrontendWorkbenchTab {
  editorId: string;
  paneId: string;
  title: string;
  kind: string;
  active: boolean;
  checkoutId?: string | null;
}
export interface FrontendWorkbench {
  ownerWindow?: Window;
  ready?(workspaceRef?: WorkspaceRef): Promise<void>;
  tabs(): FrontendWorkbenchTab[];
  activate(editorId: string): Promise<void>;
  close(editorId: string): Promise<void>;
}
const workbenches = new Map<string, FrontendWorkbench>();
export function registerFrontendWorkbench(value: FrontendWorkbench, windowId = "main"): () => void {
  workbenches.set(windowId, value);
  return () => { if (workbenches.get(windowId) === value) workbenches.delete(windowId); };
}
export function findFrontendWorkbench(windowId: string): FrontendWorkbench | null { return workbenches.get(windowId) ?? null; }
export function getFrontendWorkbench(windowId = "main"): FrontendWorkbench {
  const workbench = findFrontendWorkbench(windowId);
  if (!workbench) throw new Error("This window has no mounted Workbench.");
  return workbench;
}
