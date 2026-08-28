import { ipcInvoke } from "./ipc";
import type { WorkspaceRef } from "./project";

export interface ExtraWorkdirEntry {
  path: string;
  comment: string;
  readOnly: boolean;
}

export interface ExtraWorkdirStatus extends ExtraWorkdirEntry {
  exists: boolean;
}

export function extraWorkdirsGet(workspaceRef: WorkspaceRef): Promise<ExtraWorkdirStatus[]> {
  return ipcInvoke<ExtraWorkdirStatus[]>("extra_workdirs_get", { workspaceRef });
}

export function extraWorkdirsSet(
  workspaceRef: WorkspaceRef,
  entries: ExtraWorkdirEntry[],
): Promise<ExtraWorkdirStatus[]> {
  return ipcInvoke<ExtraWorkdirStatus[]>("extra_workdirs_set", { workspaceRef, entries });
}

export function extraWorkdirsMap(
  workspaceRefs: WorkspaceRef[],
): Promise<Record<string, ExtraWorkdirStatus[]>> {
  return ipcInvoke<Record<string, ExtraWorkdirStatus[]>>("extra_workdirs_map", { workspaceRefs });
}
