import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createPinia, setActivePinia } from "pinia";
import { useNotificationStore } from "../stores/notification";

describe("notification store", () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    vi.useFakeTimers();
  });

  afterEach(() => {
    vi.restoreAllMocks();
    vi.useRealTimers();
  });

  it("updates an existing notice in place when replaceOperation is enabled", () => {
    const store = useNotificationStore();

    const noticeId = store.addNotice("info", "Switching branch", {
      operation: "collab:git-switch",
      sticky: true,
      spinner: true,
      replaceOperation: true,
    });

    expect(store.notices).toHaveLength(1);
    expect(store.notices[0].id).toBe(noticeId);
    expect(store.notices[0].spinner).toBe(true);
    expect(store.notices[0].sticky).toBe(true);

    const updatedId = store.addNotice("success", "Switched branch", {
      operation: "collab:git-switch",
      ttl: 3000,
      replaceOperation: true,
    });

    expect(updatedId).toBe(noticeId);
    expect(store.notices).toHaveLength(1);
    expect(store.notices[0].level).toBe("success");
    expect(store.notices[0].message).toBe("Switched branch");
    expect(store.notices[0].spinner).toBe(false);
    expect(store.notices[0].sticky).toBe(false);

    vi.advanceTimersByTime(2999);
    expect(store.notices).toHaveLength(1);

    vi.advanceTimersByTime(1);
    expect(store.notices).toHaveLength(0);
  });

  it("keeps the newest notices visible when the queue is saturated", () => {
    const store = useNotificationStore();

    store.addNotice("info", "First", {
      operation: "first",
      sticky: true,
      replaceOperation: true,
    });
    store.addNotice("info", "Second", { sticky: true });
    store.addNotice("info", "Third", { sticky: true });
    store.addNotice("info", "Fourth", { sticky: true });

    expect(store.visibleNotices.map((notice) => notice.message)).toEqual([
      "Fourth",
      "Third",
      "Second",
    ]);

    store.addNotice("success", "First updated", {
      operation: "first",
      sticky: true,
      replaceOperation: true,
    });

    expect(store.visibleNotices.map((notice) => notice.message)).toEqual([
      "First updated",
      "Fourth",
      "Third",
    ]);
  });

  it("logs unique error notices to the console with operation metadata", () => {
    const store = useNotificationStore();
    const consoleError = vi.spyOn(console, "error").mockImplementation(() => {});

    store.addNotice("error", "Download failed", {
      code: "download.failed",
      operation: "knowledge_download_local_embedding_model",
    });
    store.addNotice("error", "Download failed", {
      code: "download.failed",
      operation: "knowledge_download_local_embedding_model",
    });

    expect(consoleError).toHaveBeenCalledTimes(1);
    expect(consoleError).toHaveBeenCalledWith(
      "[notification] (knowledge_download_local_embedding_model) Download failed",
      {
        code: "download.failed",
        operation: "knowledge_download_local_embedding_model",
      },
    );
  });

  it("allows callers to suppress automatic console logging", () => {
    const store = useNotificationStore();
    const consoleError = vi.spyOn(console, "error").mockImplementation(() => {});

    store.addNotice("error", "Already logged elsewhere", {
      operation: "chat",
      skipConsoleLog: true,
    });

    expect(consoleError).not.toHaveBeenCalled();
  });

  it("applies shorter default ttl values by level", () => {
    const store = useNotificationStore();

    store.addNotice("info", "Info");
    store.addNotice("warning", "Warning");
    store.addNotice("error", "Error", {
      skipConsoleLog: true,
    });

    expect(store.notices.map((notice) => ({
      level: notice.level,
      ttl: notice.ttl,
    }))).toEqual([
      { level: "info", ttl: 2400 },
      { level: "warning", ttl: 4200 },
      { level: "error", ttl: 6000 },
    ]);

    vi.advanceTimersByTime(2399);
    expect(store.notices.map((notice) => notice.message)).toEqual([
      "Info",
      "Warning",
      "Error",
    ]);

    vi.advanceTimersByTime(1);
    expect(store.notices.map((notice) => notice.message)).toEqual([
      "Warning",
      "Error",
    ]);

    vi.advanceTimersByTime(1799);
    expect(store.notices.map((notice) => notice.message)).toEqual([
      "Warning",
      "Error",
    ]);

    vi.advanceTimersByTime(1);
    expect(store.notices.map((notice) => notice.message)).toEqual([
      "Error",
    ]);

    vi.advanceTimersByTime(1799);
    expect(store.notices.map((notice) => notice.message)).toEqual([
      "Error",
    ]);

    vi.advanceTimersByTime(60_000);
    expect(store.notices.map((notice) => notice.message)).toEqual(["Error"]);
  });

  it("keeps errors until the user removes them explicitly", () => {
    const store = useNotificationStore();
    const errorId = store.addNotice("error", "Compact failed", {
      operation: "chat",
      ttl: 1,
      sticky: false,
      replaceOperation: true,
      skipConsoleLog: true,
    });

    vi.advanceTimersByTime(60_000);
    store.clearByOperation("chat");
    store.clearAll();
    store.addNotice("success", "Chat recovered", {
      operation: "chat",
      replaceOperation: true,
    });
    const secondErrorId = store.addNotice("error", "Compact retry failed", {
      operation: "chat",
      replaceOperation: true,
      skipConsoleLog: true,
    });

    expect(store.notices.find((notice) => notice.id === errorId)).toMatchObject({
      level: "error",
      message: "Compact failed",
      sticky: true,
    });
    expect(store.notices.find((notice) => notice.id === secondErrorId)).toMatchObject({
      level: "error",
      message: "Compact retry failed",
      sticky: true,
    });

    store.removeNotice(errorId);
    expect(store.notices.some((notice) => notice.id === errorId)).toBe(false);
  });

  it("pauses and resumes timed removal", () => {
    const store = useNotificationStore();
    const noticeId = store.addNotice("info", "Hover me");

    vi.advanceTimersByTime(1000);
    store.pauseNotice(noticeId);

    vi.advanceTimersByTime(10000);
    expect(store.notices.map((notice) => notice.message)).toEqual(["Hover me"]);

    store.resumeNotice(noticeId);

    vi.advanceTimersByTime(1399);
    expect(store.notices.map((notice) => notice.message)).toEqual(["Hover me"]);

    vi.advanceTimersByTime(1);
    expect(store.notices).toHaveLength(0);
  });
});
