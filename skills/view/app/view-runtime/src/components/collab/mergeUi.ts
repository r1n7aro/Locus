import { t } from "../../i18n";
import type { DocMergeStatus } from "../../types";

type MergeSidePosition = "left" | "right";
export type MergeDisplayStatus = DocMergeStatus | "stagedResolved" | "stagedPartial";

function translateDetailedLabel(label: string): string | null {
  const detailedMatchers: Array<{
    pattern: RegExp;
    key: string;
  }> = [
    { pattern: /^Current \((.+)\)$/, key: "merge.side.currentWithDetail" },
    { pattern: /^Incoming \((.+)\)$/, key: "merge.side.incomingWithDetail" },
    { pattern: /^Cherry-pick \((.+)\)$/, key: "merge.side.cherryPickWithDetail" },
    { pattern: /^Revert of \((.+)\)$/, key: "merge.side.revertWithDetail" },
    { pattern: /^Rebase target \((.+)\)$/, key: "merge.side.rebaseTargetWithDetail" },
    { pattern: /^Your commit \((.+)\)$/, key: "merge.side.yourCommitWithDetail" },
  ];

  for (const { pattern, key } of detailedMatchers) {
    const match = label.match(pattern);
    if (match) {
      return t(key, match[1]);
    }
  }

  return null;
}

function normalizeKnownLabel(label: string): string | null {
  const trimmed = label.trim();
  switch (trimmed) {
    case "Updated upstream":
      return t("merge.side.current");
    case "Stashed changes":
      return t("merge.side.stashed");
    case "Ours":
    case "HEAD":
      return t("merge.side.current");
    case "Theirs":
      return t("merge.side.incoming");
    case "Current":
      return t("merge.side.current");
    case "Incoming":
      return t("merge.side.incoming");
    default:
      return translateDetailedLabel(trimmed);
  }
}

function stripTrailingDetail(label: string): string {
  return label.replace(/\s*[\(\（][^\)\）]*[\)\）]\s*$/, "").trim();
}

export function humanizeMergeSideLabel(
  label: string | null | undefined,
  side: MergeSidePosition,
): string {
  const trimmed = (label ?? "").trim();
  if (!trimmed) {
    return side === "left" ? t("merge.side.current") : t("merge.side.incoming");
  }
  return normalizeKnownLabel(trimmed) ?? trimmed;
}

export function compactMergeSideLabel(
  label: string | null | undefined,
  side: MergeSidePosition,
): string {
  const trimmed = (label ?? "").trim();
  switch (trimmed) {
    case "Updated upstream":
      return t("merge.side.currentShort");
    case "Theirs":
      return t("merge.side.incomingShort");
    case "Stashed changes":
      return t("merge.side.stashedShort");
    case "Ours":
    case "HEAD":
      return t("merge.side.currentShort");
    default: {
      const humanized = humanizeMergeSideLabel(trimmed, side);
      const compact = stripTrailingDetail(humanized);
      return compact || humanized;
    }
  }
}

export function sharedBaseLabel(): string {
  return t("merge.side.sharedBase");
}

export function compactBaseLabel(): string {
  return t("merge.side.sharedBaseShort");
}

export function mergeStatusTone(status: MergeDisplayStatus): string {
  switch (status) {
    case "stagedResolved":
      return "resolved";
    case "stagedPartial":
      return "partial";
    case "hasConflicts":
      return "conflict";
    case "autoResolved":
      return "auto";
    case "addedOurs":
      return "current";
    case "addedTheirs":
      return "incoming";
    case "removedOurs":
    case "removedTheirs":
      return "removed";
    default:
      return "";
  }
}

export function mergeStatusLabel(status: MergeDisplayStatus): string {
  switch (status) {
    case "stagedResolved":
      return t("merge.status.stagedResolved");
    case "stagedPartial":
      return t("merge.status.stagedPartial");
    case "hasConflicts":
      return t("merge.status.needsReview");
    case "autoResolved":
      return t("merge.status.autoResolved");
    case "addedOurs":
      return t("merge.status.currentOnly");
    case "addedTheirs":
      return t("merge.status.incomingOnly");
    case "removedOurs":
      return t("merge.status.removedCurrent");
    case "removedTheirs":
      return t("merge.status.removedIncoming");
    default:
      return t("merge.status.noChanges");
  }
}

export function conflictCodeLabel(conflictCode: string, semanticLabel?: string): string {
  switch (conflictCode) {
    case "UU":
      return t("merge.queue.badge.bothChanged");
    case "AA":
      return t("merge.queue.badge.bothAdded");
    case "DD":
      return t("merge.queue.badge.bothDeleted");
    case "AU":
      return t("merge.queue.badge.currentAdded");
    case "DU":
      return t("merge.queue.badge.currentDeleted");
    case "UA":
      return t("merge.queue.badge.incomingAdded");
    case "UD":
      return t("merge.queue.badge.incomingDeleted");
    default:
      return semanticLabel || t("merge.queue.badge.conflict");
  }
}

export function hierarchyBadgeLabel(kind: string, count: number): string {
  switch (kind) {
    case "added":
      return t("merge.tree.badgeAdded", count);
    case "removed":
      return t("merge.tree.badgeRemoved", count);
    case "modified":
      return t("merge.tree.badgeChanged", count);
    case "comp":
      return t("merge.tree.badgeComponents", count);
    default:
      return String(count);
  }
}
