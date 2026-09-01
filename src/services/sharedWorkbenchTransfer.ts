import type {
  WorkbenchEditorTransferRecord,
  WorkbenchWindowDropIntent,
} from "../types/workbench";
import type { WorkbenchWindowTransferAckPayload } from "./workbenchWindow";

export interface SharedWorkbenchTransferTarget {
  accept: (
    record: WorkbenchEditorTransferRecord,
    target?: WorkbenchWindowDropIntent | null,
  ) => Promise<WorkbenchWindowTransferAckPayload>;
  cancel: (token: string) => void | Promise<void>;
}

const targets = new Map<string, SharedWorkbenchTransferTarget>();
const targetWaiters = new Map<string, Set<(target: SharedWorkbenchTransferTarget | null) => void>>();

export function registerSharedWorkbenchTransferTarget(
  windowId: string,
  target: SharedWorkbenchTransferTarget,
): () => void {
  targets.set(windowId, target);
  const waiters = targetWaiters.get(windowId);
  if (waiters) {
    targetWaiters.delete(windowId);
    for (const resolve of waiters) resolve(target);
  }
  return () => {
    if (targets.get(windowId) === target) targets.delete(windowId);
  };
}

export function hasSharedWorkbenchTransferTarget(windowId: string): boolean {
  return targets.has(windowId);
}

export async function waitForSharedWorkbenchTransferTarget(
  windowId: string,
  timeoutMs: number,
): Promise<SharedWorkbenchTransferTarget | null> {
  const available = targets.get(windowId);
  if (available) return available;
  return new Promise((resolve) => {
    const waiters = targetWaiters.get(windowId) ?? new Set();
    targetWaiters.set(windowId, waiters);
    let settled = false;
    const finish = (target: SharedWorkbenchTransferTarget | null) => {
      if (settled) return;
      settled = true;
      window.clearTimeout(timer);
      waiters.delete(finish);
      if (waiters.size === 0) targetWaiters.delete(windowId);
      resolve(target);
    };
    const timer = window.setTimeout(() => finish(null), timeoutMs);
    waiters.add(finish);
  });
}

export async function dispatchSharedWorkbenchTransfer(
  windowId: string,
  record: WorkbenchEditorTransferRecord,
  target?: WorkbenchWindowDropIntent | null,
  timeoutMs = 8_000,
): Promise<WorkbenchWindowTransferAckPayload | null> {
  const receiver = await waitForSharedWorkbenchTransferTarget(windowId, timeoutMs);
  return receiver ? receiver.accept(record, target) : null;
}

export async function cancelSharedWorkbenchTransfer(
  windowId: string,
  token: string,
): Promise<void> {
  await targets.get(windowId)?.cancel(token);
}
