import { ipcInvoke } from "./ipc";
import type { WorkspaceRef } from "./project";

export function resolveRefGraphGuid(
  path: string,
  workspaceRef: WorkspaceRef,
): Promise<string | null> {
  return ipcInvoke<string | null>("ref_graph_resolve_guid", { path, workspaceRef });
}

export function resolveRefGraphPath(
  guidHex: string,
  workspaceRef: WorkspaceRef,
): Promise<string | null> {
  return ipcInvoke<string | null>("ref_graph_resolve_path", { guidHex, workspaceRef });
}
