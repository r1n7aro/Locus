import { ipcInvoke } from "./ipc";
import type { ChangedFile, UndoConflictInfo, VcsUndoEntry } from "../types";

export function undoList(sessionId: string): Promise<VcsUndoEntry[]> {
  return ipcInvoke<VcsUndoEntry[]>("undo_list", { sessionId });
}

export function undoPerform(
  sessionId: string,
  assistantMessageId: string,
  force = false,
): Promise<void> {
  return ipcInvoke("undo_perform", { sessionId, assistantMessageId, force });
}

export function undoPreview(sessionId: string, assistantMessageId: string): Promise<ChangedFile[]> {
  return ipcInvoke<ChangedFile[]>("undo_preview", { sessionId, assistantMessageId });
}

export function undoCheckConflicts(
  sessionId: string,
  assistantMessageId: string,
): Promise<UndoConflictInfo[]> {
  return ipcInvoke<UndoConflictInfo[]>("undo_check_conflicts", { sessionId, assistantMessageId });
}
