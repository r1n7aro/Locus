import type { AsyncTaskStatus } from "../types";
import { ipcInvoke } from "./ipc";
import { getLocusRuntime } from "./locusRuntime";

export interface BackgroundTaskInfo {
  taskId: string;
  sessionId: string;
  localId: string;
  toolName: string;
  description: string | null;
  status: AsyncTaskStatus;
  progress: string | null;
  createdAt: number;
  startedAt: number | null;
}

export function listBackgroundTasks(sessionId: string): Promise<BackgroundTaskInfo[]> {
  return ipcInvoke("list_background_tasks", { sessionId });
}

export function cancelBackgroundTask(sessionId: string, taskId: string): Promise<BackgroundTaskInfo> {
  return ipcInvoke("cancel_background_task", { sessionId, taskId });
}

/** Live panel rows of one session, pushed (coalesced) after any of them changes. */
export interface BackgroundTasksChangedEvent {
  sessionId: string;
  tasks: BackgroundTaskInfo[];
}

export function subscribeBackgroundTasksChanged(handler: (event: BackgroundTasksChangedEvent) => void) {
  return getLocusRuntime().subscribe<BackgroundTasksChangedEvent>("background-tasks-changed", handler);
}
