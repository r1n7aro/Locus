import { ipcInvoke } from "./ipc";

export interface DirEntry {
  relPath: string;
  name: string;
  isDir: boolean;
}

export interface DirEntriesPage {
  entries: DirEntry[];
  totalCount: number;
  nextOffset: number;
  hasMore: boolean;
}

export interface WorkspaceSearchEntry {
  relPath: string;
  name: string;
  parentPath: string;
  isDir: boolean;
  matchScore: number;
}

export type WorkspaceEntryKind = "file" | "folder" | "other" | "missing";

export interface WorkspaceEntryStat {
  path: string;
  exists: boolean;
  entryKind: WorkspaceEntryKind;
}

export function getWorkingDir(): Promise<string> {
  return ipcInvoke<string>("get_working_dir");
}

export function setWorkingDir(path: string): Promise<string> {
  return ipcInvoke<string>("set_working_dir", { path });
}

export function listRecentDirs(): Promise<string[]> {
  return ipcInvoke<string[]>("list_recent_dirs");
}

export function removeRecentDir(path: string): Promise<string[]> {
  return ipcInvoke<string[]>("remove_recent_dir", { path });
}

export function openDirInFileExplorer(path: string): Promise<void> {
  return ipcInvoke<void>("open_dir_in_file_explorer", { path });
}

export function listDirEntries(subPath: string): Promise<DirEntry[]> {
  return ipcInvoke<DirEntry[]>("list_dir_entries", { subPath });
}

export function listDirEntriesPage(
  subPath: string,
  offset = 0,
  limit = 200,
  excludeMeta = false,
): Promise<DirEntriesPage> {
  return ipcInvoke<DirEntriesPage>("list_dir_entries_page", {
    subPath,
    offset,
    limit,
    excludeMeta,
  });
}

export function searchWorkspaceEntries(
  query: string,
  limit = 200,
): Promise<WorkspaceSearchEntry[]> {
  return ipcInvoke<WorkspaceSearchEntry[]>("search_workspace_entries", { query, limit });
}

export function statWorkspaceEntries(paths: string[]): Promise<WorkspaceEntryStat[]> {
  return ipcInvoke<WorkspaceEntryStat[]>("stat_workspace_entries", { paths });
}

export function resetAllConfig(): Promise<void> {
  return ipcInvoke("reset_all_config");
}
