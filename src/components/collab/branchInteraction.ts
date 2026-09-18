import type { GitBranchInfo, GitBranchTarget, GitCommitInfo, GitGraphRef } from "../../types";

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

export function resolveCommitBranchTargets(
  commit: GitCommitInfo,
  graphRefs: GitGraphRef[],
): GitBranchTarget[] {
  const targets: GitBranchTarget[] = [];
  const seen = new Set<string>();
  for (const ref of graphRefs) {
    if (ref.targetHash !== commit.hash || ref.kind === "tag" || seen.has(ref.fullName)) continue;
    seen.add(ref.fullName);
    const branch = { shortHash: commit.shortHash, message: commit.message };
    if (ref.kind === "localBranch") {
      const name = ref.branchName ?? ref.shortName;
      targets.push({ kind: "localBranch", branch: { ...branch, name, isCurrent: ref.isCurrent } });
      continue;
    }
    const remoteRefName = ref.fullName.replace(/^refs\/remotes\//, "");
    const remoteName = ref.remoteName ?? remoteRefName.split("/")[0];
    if (!remoteName) continue;
    const name = ref.branchName ?? remoteRefName.slice(remoteName.length + 1);
    if (!name || name === "HEAD" || name.endsWith("/HEAD")) continue;
    targets.push({ kind: "remoteBranch", remoteName, branch: { ...branch, name } });
  }
  return targets.sort((left, right) => {
    if (left.kind !== right.kind) return left.kind === "localBranch" ? -1 : 1;
    return branchTargetName(left).localeCompare(branchTargetName(right));
  });
}

export function branchTargetName(target: GitBranchTarget): string {
  return target.kind === "localBranch"
    ? target.branch.name
    : `${target.remoteName}/${target.branch.name}`;
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
