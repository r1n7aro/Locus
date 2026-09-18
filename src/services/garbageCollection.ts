import { ipcInvoke } from "./ipc";
import type { WorkspaceRef } from "./project";

export interface DatabaseSpace {
  databaseBytes: number;
  walBytes: number;
  logicalBytes: number;
  freePageBytes: number;
}

export interface DatabaseCollection {
  kind: "session" | "project";
  path: string;
  skipped: boolean;
  error: string | null;
  result: {
    before: DatabaseSpace;
    after: DatabaseSpace;
    reclaimedBytes: number;
    warning: string | null;
  } | null;
}

export function garbageCollection(workspaceRef: WorkspaceRef | null): Promise<DatabaseCollection[]> {
  return ipcInvoke("garbage_collection", { workspaceRef });
}
