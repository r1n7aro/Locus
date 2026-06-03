import { ipcInvoke } from "./ipc";

export function resolveRefGraphGuid(path: string): Promise<string | null> {
  return ipcInvoke<string | null>("ref_graph_resolve_guid", { path });
}

export function resolveRefGraphPath(guidHex: string): Promise<string | null> {
  return ipcInvoke<string | null>("ref_graph_resolve_path", { guidHex });
}
