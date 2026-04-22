import type { GitBranchInfo, GitBranchTarget } from "../../types";

export interface BranchDblclickAction {
  action: "switch" | "checkoutTracking";
  branchName: string;
  targetKind: "local" | "remote";
}

export function resolveBranchDblclickAction(
  target: GitBranchTarget,
  localBranches: GitBranchInfo[],
): BranchDblclickAction | null {
  if (target.kind === "localBranch") {
    if (target.branch.isCurrent) return null;
    return {
      action: "switch",
      branchName: target.branch.name,
      targetKind: "local",
    };
  }

  const matchingLocal = localBranches.find(branch => branch.name === target.branch.name);
  if (matchingLocal) {
    if (matchingLocal.isCurrent) return null;
    return {
      action: "switch",
      branchName: matchingLocal.name,
      targetKind: "local",
    };
  }

  return {
    action: "checkoutTracking",
    branchName: `${target.remoteName}/${target.branch.name}`,
    targetKind: "remote",
  };
}
