import { ipcInvoke } from "./ipc";
import type { ChangedFile, UndoConflictInfo, VcsUndoEntry } from "../types";

export function undoList(sessionId: string): Promise<VcsUndoEntry[]> {
  return ipcInvoke<VcsUndoEntry[]>("undo_list", { sessionId });
}

export function undoPerform(
  sessionId: string,
  assistantMessageId: string,
  force = false,
  acceptDirty = false,
): Promise<void> {
  return ipcInvoke("undo_perform", { sessionId, assistantMessageId, force, acceptDirty });
}

export function undoPerformToMessage(
  sessionId: string,
  assistantMessageId: string,
  truncateMessageId: string,
  force = false,
  acceptDirty = false,
): Promise<void> {
  return ipcInvoke("undo_perform_to_message", {
    sessionId,
    assistantMessageId,
    truncateMessageId,
    force,
    acceptDirty,
  });
}

export function undoPreview(sessionId: string, assistantMessageId: string): Promise<ChangedFile[]> {
  return ipcInvoke<ChangedFile[]>("undo_preview", { sessionId, assistantMessageId });
}

/** Error code returned by undo_revert_file when the file was modified again after the round. */
export const UNDO_FILE_DIRTY_ERROR_CODE = "undo.file_dirty";

/**
 * Revert a single file to the pre-round snapshot anchoring the panel diff.
 * Leaves the undo stack and chat history untouched (acts like a manual edit).
 * Fails with code `undo.file_dirty` when the file was modified again after
 * the recorded rounds; confirm and retry with `force` to roll that back too.
 */
export function undoRevertFile(
  sessionId: string,
  assistantMessageId: string,
  file: Pick<ChangedFile, "path" | "oldPath" | "status">,
  force = false,
): Promise<ChangedFile[]> {
  return ipcInvoke<ChangedFile[]>("undo_revert_file", {
    sessionId,
    assistantMessageId,
    path: file.path,
    oldPath: file.oldPath ?? null,
    status: file.status,
    force,
  });
}

export function undoCheckConflicts(
  sessionId: string,
  assistantMessageId: string,
): Promise<UndoConflictInfo[]> {
  return ipcInvoke<UndoConflictInfo[]>("undo_check_conflicts", { sessionId, assistantMessageId });
}

/** Files the undo would restore that were modified again after the round ended. */
export function undoCheckDirty(
  sessionId: string,
  assistantMessageId: string,
): Promise<ChangedFile[]> {
  return ipcInvoke<ChangedFile[]>("undo_check_dirty", { sessionId, assistantMessageId });
}
