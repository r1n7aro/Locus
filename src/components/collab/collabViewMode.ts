import type { MergeOperation } from "../../types";
import { t } from "../../i18n";
import type { CollabHistorySelectionKind } from "./historySelection";

export type CollabRightPanelMode = "merge" | "commit" | "workspace";

export function resolveCollabRightPanelMode(
  selectionKind: CollabHistorySelectionKind,
  hasConflictState: boolean,
): CollabRightPanelMode {
  if (hasConflictState) return "merge";
  if (selectionKind === "commit" || selectionKind === "stash") return "commit";
  return "workspace";
}

export function resolveMergeOperationBadge(
  operation: MergeOperation | null,
  hasUnresolvedFiles: boolean,
): string {
  if (operation) {
    switch (operation.kind) {
      case "merge": return "MERGE";
      case "cherryPick": return "CHERRY-PICK";
      case "rebase": return "REBASE";
      case "revert": return "REVERT";
      default: return "CONFLICT";
    }
  }
  return hasUnresolvedFiles ? "CONFLICT" : "";
}

export function resolveConflictActionHint(operation: MergeOperation | null): string {
  return operation
    ? t("collab.conflict.actionDisabledDuring", operation.label)
    : t("collab.conflict.actionDisabled");
}
