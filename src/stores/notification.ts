import { ref, computed } from "vue";
import { defineStore } from "pinia";
import type { NotificationLevel } from "../types";

export interface Notice {
  id: string;
  level: NotificationLevel;
  message: string;
  code?: string;
  operation?: string;
  createdAt: number;
  ttl: number;
  remainingTtl: number;
  sticky: boolean;
  spinner: boolean;
  paused: boolean;
  timerStartedAt?: number;
  timerId?: ReturnType<typeof setTimeout>;
}

interface AddNoticeOptions {
  code?: string;
  operation?: string;
  ttl?: number;
  sticky?: boolean;
  spinner?: boolean;
  replaceOperation?: boolean;
  skipConsoleLog?: boolean;
}

const DEFAULT_TTLS: Record<NotificationLevel, number> = {
  info: 2_400,
  success: 2_800,
  warning: 4_200,
  error: 6_000,
};
const MAX_STORED = 10;
const MAX_VISIBLE = 3;

export const useNotificationStore = defineStore("notification", () => {
  const notices = ref<Notice[]>([]);

  const visibleNotices = computed(() => notices.value.slice(-MAX_VISIBLE).reverse());

  function dedupKey(message: string, opts?: { code?: string; operation?: string }): string {
    return `${opts?.code ?? ""}|${opts?.operation ?? ""}|${message}`;
  }

  function clearRemovalTimer(notice: Notice) {
    if (!notice.timerId) return;
    clearTimeout(notice.timerId);
    notice.timerId = undefined;
  }

  function scheduleRemoval(notice: Notice, ttl: number, sticky: boolean) {
    clearRemovalTimer(notice);
    if (sticky) {
      notice.timerStartedAt = undefined;
      notice.remainingTtl = ttl;
      notice.paused = false;
      return;
    }

    notice.timerStartedAt = Date.now();
    notice.remainingTtl = ttl;
    notice.paused = false;
    notice.timerId = setTimeout(() => removeNotice(notice.id), ttl);
  }

  function resolveNoticeTtl(level: NotificationLevel, options?: AddNoticeOptions) {
    return options?.ttl ?? DEFAULT_TTLS[level];
  }

  function shouldLogToConsole(
    notice: Notice | undefined,
    level: NotificationLevel,
    message: string,
    options?: AddNoticeOptions,
  ): boolean {
    if (options?.skipConsoleLog || level !== "error") return false;
    if (!notice) return true;
    return (
      notice.level !== level
      || notice.message !== message
      || notice.code !== options?.code
      || notice.operation !== options?.operation
    );
  }

  function logNoticeToConsole(message: string, options?: AddNoticeOptions) {
    const prefix = options?.operation
      ? `[notification] (${options.operation}) ${message}`
      : `[notification] ${message}`;
    const meta = {
      ...(options?.code ? { code: options.code } : {}),
      ...(options?.operation ? { operation: options.operation } : {}),
    };
    if (Object.keys(meta).length > 0) {
      console.error(prefix, meta);
      return;
    }
    console.error(prefix);
  }

  function applyNoticeUpdate(
    notice: Notice,
    level: NotificationLevel,
    message: string,
    options?: AddNoticeOptions,
  ) {
    clearRemovalTimer(notice);
    const ttl = resolveNoticeTtl(level, options);
    const sticky = options?.sticky ?? false;
    notice.level = level;
    notice.message = message;
    notice.code = options?.code;
    notice.operation = options?.operation;
    notice.createdAt = Date.now();
    notice.ttl = ttl;
    notice.remainingTtl = ttl;
    notice.sticky = sticky;
    notice.spinner = options?.spinner ?? false;
    notice.paused = false;
    scheduleRemoval(notice, ttl, sticky);
  }

  function moveNoticeToTail(id: string) {
    const idx = notices.value.findIndex((n) => n.id === id);
    if (idx === -1 || idx === notices.value.length - 1) return;
    const [notice] = notices.value.splice(idx, 1);
    notices.value.push(notice);
  }

  function addNotice(
    level: NotificationLevel,
    message: string,
    options?: AddNoticeOptions,
  ): string {
    if (options?.replaceOperation && options.operation) {
      const existingByOperation = notices.value.find((n) => n.operation === options.operation);
      if (existingByOperation) {
        if (shouldLogToConsole(existingByOperation, level, message, options)) {
          logNoticeToConsole(message, options);
        }
        applyNoticeUpdate(existingByOperation, level, message, options);
        moveNoticeToTail(existingByOperation.id);
        return existingByOperation.id;
      }
    }

    const key = dedupKey(message, options);
    const existing = notices.value.find(
      (n) => n.level === level && dedupKey(n.message, n) === key,
    );

    if (existing) {
      applyNoticeUpdate(existing, level, message, options);
      moveNoticeToTail(existing.id);
      return existing.id;
    }

    if (shouldLogToConsole(undefined, level, message, options)) {
      logNoticeToConsole(message, options);
    }

    const id = crypto.randomUUID();
    const notice: Notice = {
      id,
      level,
      message,
      createdAt: 0,
      ttl: DEFAULT_TTLS[level],
      remainingTtl: DEFAULT_TTLS[level],
      sticky: false,
      spinner: false,
      paused: false,
    };

    applyNoticeUpdate(notice, level, message, options);
    notices.value.push(notice);
    if (notices.value.length > MAX_STORED) {
      const removed = notices.value.shift()!;
      clearRemovalTimer(removed);
    }

    return id;
  }

  function pauseNotice(id: string) {
    const notice = notices.value.find((n) => n.id === id);
    if (!notice || notice.sticky || notice.paused || !notice.timerId) return;

    const elapsed = notice.timerStartedAt ? Date.now() - notice.timerStartedAt : 0;
    notice.remainingTtl = Math.max(0, notice.remainingTtl - elapsed);
    notice.paused = true;
    clearRemovalTimer(notice);
  }

  function resumeNotice(id: string) {
    const notice = notices.value.find((n) => n.id === id);
    if (!notice || notice.sticky || !notice.paused) return;

    if (notice.remainingTtl <= 0) {
      removeNotice(id);
      return;
    }

    scheduleRemoval(notice, notice.remainingTtl, false);
  }

  function removeNotice(id: string) {
    const idx = notices.value.findIndex((n) => n.id === id);
    if (idx !== -1) {
      const [removed] = notices.value.splice(idx, 1);
      clearRemovalTimer(removed);
    }
  }

  function clearByOperation(operation: string) {
    const toRemove = notices.value.filter((n) => n.operation === operation);
    for (const n of toRemove) {
      removeNotice(n.id);
    }
  }

  function clearAll() {
    for (const n of notices.value) {
      clearRemovalTimer(n);
    }
    notices.value = [];
  }

  return {
    notices,
    visibleNotices,
    addNotice,
    pauseNotice,
    resumeNotice,
    removeNotice,
    clearByOperation,
    clearAll,
  };
});
