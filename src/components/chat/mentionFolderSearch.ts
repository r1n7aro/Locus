import type { WorkspaceSearchEntry } from "../../services/project";

export interface FolderMentionSearchResult {
  relPath: string;
  name: string;
  parentPath: string;
  isDir: true;
  matchScore: number;
  entryKind: "asset";
}

const UNITY_REFERENCE_ROOT_RE = /^(?:Assets|Packages|ProjectSettings)(?:\/|$)/i;

export function mapWorkspaceFolderMentionResults(
  entries: WorkspaceSearchEntry[],
): FolderMentionSearchResult[] {
  return entries
    .filter((entry) => entry.isDir && UNITY_REFERENCE_ROOT_RE.test(entry.relPath.replace(/\\/g, "/")))
    .map((entry) => ({
      relPath: entry.relPath.replace(/\\/g, "/").replace(/\/+$/, ""),
      name: entry.name,
      parentPath: entry.parentPath.replace(/\\/g, "/").replace(/\/+$/, ""),
      isDir: true,
      matchScore: entry.matchScore,
      entryKind: "asset",
    }));
}
