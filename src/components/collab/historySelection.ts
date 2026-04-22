import type { GitCommitInfo, GitHistorySelection, GitStashEntry } from "../../types";

export type CollabHistorySelectionKind = "none" | "workspace" | "commit" | "stash";

export function resolveHistorySelectionKind(
  selection: GitHistorySelection | null,
  commits: GitCommitInfo[],
  stashes: GitStashEntry[],
  hasWorkspaceChanges: boolean,
): CollabHistorySelectionKind {
  if (selection?.kind === "workspace") {
    return hasWorkspaceChanges ? "workspace" : "none";
  }

  const selectedCommitHash = selection?.kind === "commit" || selection?.kind === "stash"
    ? selection.hash
    : null;

  if (!selectedCommitHash) {
    return hasWorkspaceChanges ? "workspace" : "none";
  }

  if (stashes.some(stash => stash.hash === selectedCommitHash)) {
    return "stash";
  }

  if (commits.some(commit => commit.hash === selectedCommitHash && !commit.isStash)) {
    return "commit";
  }

  return "none";
}
