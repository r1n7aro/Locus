import type { NotificationLevel } from "../types";
import { normalizeAppError } from "./errors";
import { useNotificationStore } from "../stores/notification";
import { getLocusRuntime } from "./locusRuntime";

interface IpcOptions {
  operation?: string;
  notify?: boolean;
  severity?: NotificationLevel;
  throwOnError?: boolean;
}

export async function ipcInvoke<T>(
  cmd: string,
  args?: Record<string, unknown>,
  options?: IpcOptions,
): Promise<T> {
  try {
    return await getLocusRuntime().invoke<T>(cmd, args);
  } catch (e) {
    const normalized = normalizeAppError(e);
    if (options?.operation) normalized.operation = options.operation;
    else if (!normalized.operation) normalized.operation = cmd;
    if (options?.severity) normalized.severity = options.severity;

    if (options?.notify) {
      const store = useNotificationStore();
      store.addNotice(normalized.severity, normalized.message, {
        code: normalized.code,
        operation: normalized.operation,
      });
    }

    if (options?.throwOnError === false) return undefined as unknown as T;
    throw normalized;
  }
}
