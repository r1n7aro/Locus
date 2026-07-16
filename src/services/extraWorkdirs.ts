import { ipcInvoke } from "./ipc";

export interface ExtraWorkdirEntry {
  path: string;
  comment: string;
}

export interface ExtraWorkdirStatus extends ExtraWorkdirEntry {
  exists: boolean;
}

export function extraWorkdirsGet(workspacePath: string): Promise<ExtraWorkdirStatus[]> {
  return ipcInvoke<ExtraWorkdirStatus[]>("extra_workdirs_get", { workspacePath });
}

export function extraWorkdirsSet(
  workspacePath: string,
  entries: ExtraWorkdirEntry[],
): Promise<ExtraWorkdirStatus[]> {
  return ipcInvoke<ExtraWorkdirStatus[]>("extra_workdirs_set", { workspacePath, entries });
}

export function extraWorkdirsMap(
  paths: string[],
): Promise<Record<string, ExtraWorkdirStatus[]>> {
  return ipcInvoke<Record<string, ExtraWorkdirStatus[]>>("extra_workdirs_map", { paths });
}
