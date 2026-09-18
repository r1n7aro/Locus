import { ArchiveRestore, ArrowDownToLine, ArrowRightLeft, ArrowUpFromLine, Copy, FileDiff, FolderOpen, GitBranch, GitBranchPlus, GitCommitHorizontal, GitCompareArrows, GitMerge, GitPullRequestArrow, Minus, PencilLine, Plus, RefreshCw, RotateCcw, Tag, Trash2, Undo2 } from "lucide";
import type { GitBranchInfo, GitBranchTarget, GitFileChange, GitGraphRef, GitHistoryTarget } from "../../types";
import { t } from "../../i18n";
import { canOpenInEditor } from "../../composables/useHideMeta";
import { branchTargetName, resolveBranchDblclickAction, resolveBranchTargetHash, resolveCommitBranchTargets } from "./branchInteraction";

export type GitFileTarget = {
  kind: "file";
  file: GitFileChange;
  source: "gitUnstaged" | "gitStaged" | "gitCommit";
  commitHash?: string;
  selectedFiles: GitFileChange[];
};
export type GitContextTarget = GitHistoryTarget | GitBranchTarget | GitFileTarget;
export type GitMenuAction =
  | "checkoutBranch" | "checkoutDetached" | "cherryPick" | "revert"
  | "resetSoft" | "resetMixed" | "resetHard" | "createBranch" | "createBranchAndCheckout" | "createTag"
  | "mergeIntoCurrent" | "rebaseCurrentOnto" | "renameBranch" | "deleteBranch" | "deleteRemoteBranch"
  | "pull" | "push" | "fetch" | "copyBranch" | "copyHash" | "copyMessage"
  | "stashApply" | "stashApplyIndex" | "stashPop" | "stashBranch" | "stashDrop"
  | "viewDiff" | "openEditor" | "selectUnity" | "showInFolder" | "copyRelativePath" | "copyAbsolutePath"
  | "stage" | "unstage" | "discard";
export interface GitMenuItem {
  action: GitMenuAction;
  label: string;
  icon: typeof Copy;
  disabled: boolean;
  title?: string;
  danger?: boolean;
  branch?: GitBranchTarget;
}
export interface GitMenuContext {
  busy: boolean;
  conflict: boolean;
  conflictHint: string;
  currentBranch: string;
  localBranches: GitBranchInfo[];
  graphRefs: GitGraphRef[];
  unityConnected: boolean;
}

/** Keep availability and translated labels shared by every Git menu entry point. */
export function buildGitMenu(target: GitContextTarget, context: GitMenuContext): GitMenuItem[][] {
  const blocked = context.busy || context.conflict;
  const blockedHint = context.busy ? t("collab.menu.busy") : context.conflict ? context.conflictHint : undefined;
  function item(action: GitMenuAction, icon: typeof Copy, options: Partial<GitMenuItem> = {}): GitMenuItem {
    return { action, icon, label: t(`collab.menu.${action}`), disabled: blocked, title: blockedHint, ...options };
  }
  function read(action: GitMenuAction, icon: typeof Copy, options: Partial<GitMenuItem> = {}) {
    return item(action, icon, { disabled: false, title: undefined, ...options });
  }
  function checkout(branch: GitBranchTarget) {
    const resolved = resolveBranchDblclickAction(branch, context.localBranches);
    const name = resolved?.branchName ?? branch.branch.name;
    return item("checkoutBranch", GitBranch, {
      branch,
      label: t(resolved?.action === "checkoutTracking" ? "collab.menu.checkoutTracking" : "collab.menu.checkoutBranch", name),
      disabled: blocked || !resolved,
      title: blockedHint ?? (!resolved ? t("collab.menu.currentBranch")
        : branch.kind === "remoteBranch" && resolved.action === "switch" ? t("collab.menu.existingLocal", name) : branchTargetName(branch)),
    });
  }
  const create = () => [item("createBranch", GitBranchPlus), item("createBranchAndCheckout", ArrowRightLeft)];
  const copies = () => [read("copyHash", Copy), read("copyMessage", Copy)];

  if (target.kind === "commit") {
    return [
      [...resolveCommitBranchTargets(target.commit, context.graphRefs).map(checkout),
        item("checkoutDetached", GitCommitHorizontal, { title: blockedHint ?? t("collab.menu.detachedHint") })],
      [...create(), item("createTag", Tag)],
      [item("cherryPick", GitPullRequestArrow, { label: t("collab.menu.cherryPick") + (target.commit.parents.length > 1 ? "…" : ""), title: blockedHint ?? t("collab.menu.cherryPickHint") }),
        item("revert", Undo2, { label: t("collab.menu.revert") + (target.commit.parents.length > 1 ? "…" : ""), title: blockedHint ?? t("collab.menu.revertHint") })],
      [item("resetSoft", RotateCcw, { title: blockedHint ?? t("collab.menu.resetSoftHint") }),
        item("resetMixed", RotateCcw, { title: blockedHint ?? t("collab.menu.resetMixedHint") }),
        item("resetHard", RotateCcw, { danger: true, title: blockedHint ?? t("collab.menu.resetHardHint") })],
      copies(),
    ];
  }
  if (target.kind === "stash") {
    const count = target.selectedStashes?.length || 1;
    return [
      ...(count === 1 ? [
        [item("stashApply", ArchiveRestore, { title: blockedHint ?? t("collab.menu.stashApplyHint") }),
          item("stashApplyIndex", ArchiveRestore, { title: blockedHint ?? t("collab.menu.stashApplyIndexHint") }),
          item("stashPop", ArchiveRestore, { title: blockedHint ?? t("collab.menu.stashPopHint") })],
        [item("stashBranch", GitBranchPlus, { title: blockedHint ?? t("collab.menu.stashBranchHint") })],
        copies(),
      ] : []),
      [item("stashDrop", Trash2, { danger: true, label: t(count === 1 ? "collab.menu.stashDrop" : "collab.menu.stashDropMulti", count) })],
    ];
  }
  if (target.kind === "localBranch" || target.kind === "remoteBranch") {
    const current = target.kind === "localBranch" && (target.branch.isCurrent || target.branch.name === context.currentBranch);
    const integrationHint = blockedHint ?? (current ? t("collab.menu.currentBranch") : !context.currentBranch ? t("collab.menu.branchRequired") : undefined);
    return [
      [checkout(target)],
      create(),
      [item("mergeIntoCurrent", GitMerge, { disabled: blocked || current || !context.currentBranch, title: integrationHint }),
        item("rebaseCurrentOnto", GitCompareArrows, { disabled: blocked || current || !context.currentBranch, title: integrationHint })],
      ...(current ? [[item("pull", ArrowDownToLine, { title: blockedHint ?? t("collab.menu.pullHint") }), item("push", ArrowUpFromLine)]] : []),
      ...(target.kind === "remoteBranch" ? [[item("fetch", RefreshCw)]] : []),
      [read("copyBranch", Copy), read("copyHash", Copy, { disabled: !resolveBranchTargetHash(target, context.graphRefs) })],
      target.kind === "localBranch"
        ? [item("renameBranch", PencilLine), item("deleteBranch", Trash2, { danger: true, disabled: blocked || current, title: blockedHint ?? (current ? t("collab.menu.currentBranch") : undefined) })]
        : [item("deleteRemoteBranch", Trash2, { danger: true })],
    ];
  }
  const count = target.selectedFiles.length;
  const single = count === 1;
  const workspaceFile = target.source !== "gitCommit";
  const deleted = target.file.status === "D";
  // Historical file menus never mutate the index or discard working-tree files.
  return [
    [read("viewDiff", FileDiff, { disabled: !single }),
      ...(canOpenInEditor(target.file.path) ? [read("openEditor", PencilLine, { disabled: !single || deleted, title: !workspaceFile ? t("collab.menu.openWorkspaceFile") : undefined })] : []),
      ...(context.unityConnected ? [read("selectUnity", ArrowRightLeft, { disabled: !single || deleted })] : []),
      read("showInFolder", FolderOpen, { disabled: !single })],
    [read("copyRelativePath", Copy), read("copyAbsolutePath", Copy)],
    ...(workspaceFile ? [
      [item(target.source === "gitStaged" ? "unstage" : "stage", target.source === "gitStaged" ? Minus : Plus, {
        disabled: context.busy,
        title: context.busy ? t("collab.menu.busy") : undefined,
        label: t(`collab.menu.${target.source === "gitStaged" ? "unstage" : "stage"}${single ? "" : "Multi"}`, count),
      })],
      [item("discard", Undo2, { danger: true, disabled: context.busy || context.conflict, label: t(single ? "collab.menu.discard" : "collab.menu.discardMulti", count) })],
    ] : []),
  ];
}

export function validateGitName(name: string): boolean {
  return !!name && name !== "@" && !name.startsWith("-") && !name.startsWith("/") && !name.endsWith("/")
    && !name.endsWith(".") && !/[\s\x00-\x1f\x7f~^:?*\[\\]/.test(name)
    && !name.includes("..") && !name.includes("@{")
    && name.split("/").every(part => !!part && !part.startsWith(".") && !part.endsWith(".lock"));
}
