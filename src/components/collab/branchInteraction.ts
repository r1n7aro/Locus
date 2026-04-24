import type { GitBranchInfo, GitBranchTarget, GitGraphRef } from "../../types";

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

export function resolveBranchTargetHash(
  target: GitBranchTarget,
  graphRefs: GitGraphRef[],
): string | null {
  const branchName = target.branch.name;

  if (target.kind === "localBranch") {
    const ref = graphRefs.find(ref =>
      ref.kind === "localBranch"
      && (
        ref.branchName === branchName
        || ref.shortName === branchName
        || ref.fullName === `refs/heads/${branchName}`
      )
    );
    return ref?.targetHash ?? null;
  }

  const fullRemoteName = `${target.remoteName}/${branchName}`;
  const fullRefName = `refs/remotes/${target.remoteName}/${branchName}`;
  const ref = graphRefs.find(ref =>
    ref.kind === "remoteBranch"
    && (
      (ref.remoteName === target.remoteName && ref.branchName === branchName)
      || ref.shortName === fullRemoteName
      || ref.fullName === fullRefName
    )
  );
  return ref?.targetHash ?? null;
}
