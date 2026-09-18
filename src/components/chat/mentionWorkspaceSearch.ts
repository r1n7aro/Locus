import type { WorkspaceSearchEntry } from "../../services/project";

export interface WorkspaceMentionSearchResult {
  relPath: string;
  name: string;
  parentPath: string;
  isDir: boolean;
  matchScore: number;
  entryKind: "asset";
}

export function mapWorkspaceMentionResults(entries: WorkspaceSearchEntry[]): WorkspaceMentionSearchResult[] {
  return entries.map((entry) => ({
    relPath: entry.relPath.replace(/\\/g, "/").replace(/\/+$/, ""),
    name: entry.name,
    parentPath: entry.parentPath.replace(/\\/g, "/").replace(/\/+$/, ""),
    isDir: entry.isDir,
    matchScore: entry.matchScore,
    entryKind: "asset",
  }));
}
