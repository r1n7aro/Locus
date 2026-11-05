// @vitest-environment jsdom
import { createApp, defineComponent, h, nextTick, reactive, ref, type App } from "vue";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import BackgroundTasksIndicator from "../components/chat/BackgroundTasksIndicator.vue";
import { provideSurfaceHidden } from "../composables/surfaceVisibility";
import { resetBackgroundTaskActivityForTests } from "../components/chat/backgroundTaskActivity";
import { resetAppTaskActivityForTests } from "../components/chat/appTaskActivity";
import type { AppTaskInfo } from "../services/appTasks";
import type { BackgroundTaskInfo } from "../services/asyncTasks";
import type { AsyncTaskUpdatedEvent } from "../types";

const mocks = vi.hoisted(() => ({
  list: vi.fn(), cancel: vi.fn(),
  subscribeUpdates: vi.fn(), unsubscribeUpdates: vi.fn(),
  listApp: vi.fn(), subscribeApp: vi.fn(), unsubscribeApp: vi.fn(),
  subscribePushes: vi.fn(), unsubscribePushes: vi.fn(),
}));
vi.mock("../services/appTasks", () => ({
  listAppTasks: mocks.listApp,
  subscribeAppTaskUpdates: mocks.subscribeApp,
}));
vi.mock("../services/asyncTasks", () => ({
  listBackgroundTasks: mocks.list,
  cancelBackgroundTask: mocks.cancel,
  subscribeBackgroundTasksChanged: mocks.subscribePushes,
}));
vi.mock("../services/sessionStreamEventHub", () => ({
  subscribeSessionAsyncTaskUpdates: mocks.subscribeUpdates,
}));
vi.mock("../i18n", () => ({ t: (key: string, ...args: unknown[]) => [key, ...args].join(" ") }));

let app: App | undefined;
let host: HTMLElement;
let publishUpdate: ((update: AsyncTaskUpdatedEvent) => void) | undefined;
let publishAppTask: ((task: AppTaskInfo) => void) | undefined;
let pushHandlers: Array<(event: { sessionId: string; tasks: BackgroundTaskInfo[] }) => void> = [];

/** The backend pushes a session's live rows after any of them changes. */
async function pushTasks(tasks: BackgroundTaskInfo[], sessionId = "session-1") {
  for (const handler of pushHandlers) handler({ sessionId, tasks });
  await flush();
}

function task(overrides: Partial<BackgroundTaskInfo> = {}): BackgroundTaskInfo {
  return {
    taskId: "internal-1", sessionId: "session-1", localId: "build", toolName: "bash",
    description: "Build project", status: "running", progress: "Compiling scripts",
    createdAt: Date.now() - 65_000, startedAt: Date.now() - 65_000, ...overrides,
  };
}

function appTask(overrides: Partial<AppTaskInfo> = {}): AppTaskInfo {
  return {
    taskId: "app-1", kind: "worktree", title: "feat/x", detail: "F:/repo", status: "running",
    progress: { phase: "creating", completed: 45, total: 100 }, error: null,
    startedAt: Date.now() - 12_000, ...overrides,
  };
}

async function emitApp(overrides: Partial<AppTaskInfo> = {}) {
  publishAppTask!(appTask(overrides));
  await flush();
}

async function emit(overrides: Partial<AsyncTaskUpdatedEvent> = {}) {
  publishUpdate!({
    sessionId: "session-1", assistantMessageId: "message-1", toolCallId: "call-1",
    taskId: "build", toolName: "bash", status: "running", output: "", ...overrides,
  });
  await flush();
}

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason: unknown) => void;
  const promise = new Promise<T>((yes, no) => { resolve = yes; reject = no; });
  return { promise, resolve, reject };
}

async function flush() {
  for (let index = 0; index < 6; index += 1) await nextTick();
}

async function mount(sessionId: string | null = "session-1") {
  const state = reactive({ sessionId });
  host = document.createElement("div");
  document.body.append(host);
  app = createApp({ render: () => h(BackgroundTasksIndicator, state) });
  app.mount(host);
  await flush();
  return state;
}

function trigger() { return host.querySelector<HTMLButtonElement>("button")!; }
function countBadge() { return host.querySelector(".background-tasks-count")?.textContent ?? null; }
function failureDot() { return host.querySelector(".background-tasks-failure-dot"); }
function panel() { return document.querySelector<HTMLElement>(".background-tasks-panel"); }
function stop() { return panel()!.querySelector<HTMLButtonElement>(".background-task-stop")!; }
function summary() { return panel()!.querySelector<HTMLButtonElement>(".background-task-summary")!; }
async function open() { trigger().click(); await flush(); }

beforeEach(() => {
  vi.useFakeTimers();
  vi.setSystemTime(new Date("2026-09-27T12:00:00Z"));
  vi.resetAllMocks();
  mocks.list.mockResolvedValue([task()]);
  mocks.subscribeUpdates.mockImplementation((listener) => {
    publishUpdate = listener;
    return mocks.unsubscribeUpdates;
  });
  mocks.listApp.mockResolvedValue([]);
  mocks.subscribeApp.mockImplementation(async (handler) => {
    publishAppTask = handler;
    return mocks.unsubscribeApp;
  });
  pushHandlers = [];
  mocks.subscribePushes.mockImplementation(async (handler) => {
    pushHandlers.push(handler);
    return () => {
      pushHandlers = pushHandlers.filter((item) => item !== handler);
      mocks.unsubscribePushes();
    };
  });
});

afterEach(() => {
  app?.unmount();
  app = undefined;
  host?.remove();
  resetBackgroundTaskActivityForTests();
  resetAppTaskActivityForTests();
  publishUpdate = undefined;
  publishAppTask = undefined;
  vi.restoreAllMocks();
  vi.useRealTimers();
});

describe("background task panel", () => {
  it("seeds the trigger once and renders pushed live information without polling", async () => {
    await mount();
    expect(mocks.list).toHaveBeenCalledOnce();
    expect(countBadge()).toBe("1");
    await open();
    expect(mocks.list).toHaveBeenCalledWith("session-1");
    expect(panel()!.textContent).toContain("Build project");
    expect(panel()!.textContent).toContain("chat.backgroundTasks.status.running");
    expect(panel()!.textContent).toContain("1:05");
    expect(document.activeElement).toBe(panel());
    // The command stays hidden until the task is clicked, and survives pushes while shown.
    expect(panel()!.textContent).not.toContain("Compiling scripts");
    expect(summary().getAttribute("aria-expanded")).toBe("false");
    summary().click();
    await flush();
    expect(summary().getAttribute("aria-expanded")).toBe("true");
    const detail = panel()!.querySelector(".background-task-detail")!;
    expect(detail.textContent).toContain("Compiling scripts");
    expect(summary().getAttribute("aria-controls")).toBe(detail.id);
    await pushTasks([task()]);
    expect(panel()!.textContent).toContain("Compiling scripts");
    summary().click();
    await flush();
    expect(panel()!.textContent).not.toContain("Compiling scripts");

    await pushTasks([task({ status: "queued", progress: "Queued" })]);
    expect(panel()!.textContent).toContain("chat.backgroundTasks.status.queued");
    // Status-only progress has nothing to reveal.
    expect(summary().disabled).toBe(true);
    expect(summary().hasAttribute("aria-expanded")).toBe(false);
    await vi.advanceTimersByTimeAsync(20_000);
    // Seed and the panel load only.
    expect(mocks.list).toHaveBeenCalledTimes(2);
  });

  it("terminates only the selected task and prevents duplicate requests until it stops", async () => {
    const pending = deferred<BackgroundTaskInfo>();
    mocks.cancel.mockReturnValue(pending.promise);
    await mount();
    await open();
    stop().click();
    stop().click();
    await flush();
    expect(mocks.cancel).toHaveBeenCalledExactlyOnceWith("session-1", "internal-1");
    expect(stop().disabled).toBe(true);
    pending.resolve(task({ status: "cancelling" }));
    await flush();
    expect(stop().disabled).toBe(true);
    expect(panel()!.textContent).toContain("chat.backgroundTasks.status.cancelling");
    await pushTasks([]);
    expect(panel()!.querySelectorAll(".background-task-row")).toHaveLength(0);
    expect(panel()!.textContent).toContain("chat.backgroundTasks.empty");
  });

  it("stops every running agent at once and leaves other tasks running", async () => {
    mocks.list.mockResolvedValue([
      task(),
      task({ taskId: "internal-2", localId: "t2", toolName: "Agent", description: "Scan the repo" }),
      task({ taskId: "internal-3", localId: "t3", toolName: "subagent", description: "Research" }),
    ]);
    mocks.cancel.mockImplementation(async (_session: string, taskId: string) =>
      task({ taskId, localId: taskId === "internal-2" ? "t2" : "t3", status: "cancelling" }));
    await mount();
    await open();
    const stopAll = panel()!.querySelector<HTMLButtonElement>(".background-tasks-stop-all")!;
    expect(stopAll.textContent).toContain("chat.backgroundTasks.stopAllAgents 2");
    stopAll.click();
    await flush();
    expect(mocks.cancel.mock.calls).toEqual([["session-1", "internal-2"], ["session-1", "internal-3"]]);
    // Nothing more to stop together; the command keeps running.
    expect(panel()!.querySelector(".background-tasks-stop-all")).toBeNull();
    expect(panel()!.textContent).toContain("Build project");
  });

  it("does not overwrite a pushed snapshot or a cancellation with an older list response", async () => {
    await mount();
    const stale = deferred<BackgroundTaskInfo[]>();
    mocks.list.mockReturnValueOnce(stale.promise);
    await open();
    await pushTasks([task({ status: "cancelling" })]);
    stale.resolve([task()]);
    await flush();
    expect(stop().disabled).toBe(true);
    expect(panel()!.textContent).toContain("chat.backgroundTasks.status.cancelling");
  });

  it("shows errors and keeps cancellation retryable", async () => {
    await mount();
    mocks.list.mockRejectedValueOnce(new Error("List unavailable"));
    await open();
    expect(panel()!.querySelector('[role="alert"]')!.textContent).toContain("List unavailable");
    expect(panel()!.textContent).not.toContain("chat.backgroundTasks.empty");
    await pushTasks([task()]);
    expect(panel()!.querySelector('[role="alert"]')).toBeNull();
    mocks.cancel.mockRejectedValueOnce(new Error("Stop unavailable"));
    stop().click();
    await flush();
    expect(panel()!.textContent).toContain("Stop unavailable");
    expect(stop().disabled).toBe(false);
    mocks.cancel.mockResolvedValue(task({ status: "cancelling" }));
    stop().click();
    await flush();
    expect(panel()!.querySelector('[role="alert"]')).toBeNull();
  });

  it("closes on session switch and ignores late responses from the previous chat", async () => {
    const stale = deferred<BackgroundTaskInfo[]>();
    const state = await mount();
    mocks.list.mockReturnValueOnce(stale.promise);
    await open();
    state.sessionId = "session-2";
    await flush();
    expect(panel()).toBeNull();
    mocks.list.mockResolvedValue([task({ sessionId: "session-2", description: "Second chat" })]);
    await open();
    stale.resolve([task()]);
    await flush();
    expect(panel()!.textContent).toContain("Second chat");
    expect(panel()!.textContent).not.toContain("Build project");
    expect(mocks.list).toHaveBeenLastCalledWith("session-2");
    expect(countBadge()).toBe("1");
  });

  it("ignores late cancellation results after changing chats", async () => {
    const pending = deferred<BackgroundTaskInfo>();
    mocks.cancel.mockReturnValueOnce(pending.promise);
    const state = await mount();
    await open();
    stop().click();
    state.sessionId = "session-2";
    await flush();
    mocks.list.mockResolvedValue([]);
    await open();
    pending.reject(new Error("Old chat failure"));
    await flush();
    expect(panel()!.textContent).not.toContain("Old chat failure");
  });

  it("supports Escape and outside dismissal, and releases subscriptions on unmount", async () => {
    await mount();
    await open();
    panel()!.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
    await flush();
    expect(panel()).toBeNull();
    expect(document.activeElement).toBe(trigger());
    await open();
    document.body.dispatchEvent(new Event("pointerdown", { bubbles: true }));
    await flush();
    expect(panel()).toBeNull();
    await open();
    app!.unmount();
    app = undefined;
    await vi.advanceTimersByTimeAsync(60_000);
    // One seed plus one load per opening.
    expect(mocks.list).toHaveBeenCalledTimes(4);
    expect(mocks.unsubscribeUpdates).toHaveBeenCalledOnce();
    expect(mocks.unsubscribePushes).toHaveBeenCalledOnce();
    expect(panel()).toBeNull();
  });

  it("shows an empty state for a new chat without querying other sessions", async () => {
    await mount(null);
    await open();
    expect(mocks.list).not.toHaveBeenCalled();
    expect(panel()!.textContent).toContain("chat.backgroundTasks.empty");
  });

  it("keeps the panel within the viewport on narrow windows and repositions on resize", async () => {
    await mount();
    const anchor = host.querySelector<HTMLElement>(".background-tasks-indicator")!;
    vi.spyOn(anchor, "getBoundingClientRect").mockReturnValue({
      top: 650, bottom: 674, left: 950, right: 974, width: 24, height: 24,
    } as DOMRect);
    await open();
    expect(panel()!.style.left).toBe("656px");
    expect(panel()!.style.bottom).toBe("124px");
    expect(panel()!.style.maxHeight).toBe("420px");
    vi.spyOn(window, "innerWidth", "get").mockReturnValue(360);
    vi.spyOn(anchor, "getBoundingClientRect").mockReturnValue({
      top: 30, bottom: 54, left: 200, right: 224, width: 24, height: 24,
    } as DOMRect);
    window.dispatchEvent(new Event("resize"));
    await flush();
    expect(panel()!.style.width).toBe("344px");
    expect(panel()!.style.left).toBe("8px");
    expect(panel()!.style.top).toBe("60px");
    expect(panel()!.style.bottom).toBe("");
  });

  it("renders the trigger in the first render of a remounted chat surface", async () => {
    await mount();
    expect(trigger()).not.toBeNull();
    // A workbench session switch mounts the next surface while the previous one is still mounted.
    const nextHost = document.createElement("div");
    document.body.append(nextHost);
    const next = createApp({ render: () => h(BackgroundTasksIndicator, { sessionId: "session-2" }) });
    next.mount(nextHost);
    expect(nextHost.querySelector(".background-tasks-icon")).not.toBeNull();
    next.unmount();
    nextHost.remove();
  });

  it("loads once per opening and does not restart a slow load after closing", async () => {
    const pending = deferred<BackgroundTaskInfo[]>();
    mocks.list.mockReturnValue(pending.promise);
    await mount();
    await open();
    await vi.advanceTimersByTimeAsync(5000);
    // The pending seed and the pending panel load.
    expect(mocks.list).toHaveBeenCalledTimes(2);
    trigger().click();
    await flush();
    pending.resolve([task()]);
    await flush();
    await vi.advanceTimersByTimeAsync(5000);
    expect(mocks.list).toHaveBeenCalledTimes(2);
    expect(panel()).toBeNull();
  });
});

describe("background task trigger", () => {
  it("shows idle, running and stopping states from one seed request and task updates", async () => {
    mocks.list.mockResolvedValue([]);
    await mount();
    expect(trigger().classList).toContain("tone-idle");
    expect(countBadge()).toBeNull();
    expect(trigger().getAttribute("aria-label")).toContain("chat.backgroundTasks.summary.idle");
    expect(mocks.list).toHaveBeenCalledOnce();

    const tests = task({ taskId: "internal-2", localId: "tests", description: "Run tests", status: "queued" });
    mocks.list.mockResolvedValue([task(), tests]);
    await emit();
    await emit({ taskId: "tests", toolName: "unity_test_run", status: "queued" });
    expect(countBadge()).toBe("2");
    expect(trigger().classList).toContain("tone-running");
    expect(trigger().getAttribute("aria-label")).toContain("chat.backgroundTasks.summary.running 2");
    await emit({ sessionId: "session-2", taskId: "elsewhere" });
    await emit({ output: "more output" });
    expect(countBadge()).toBe("2");

    await emit({ status: "completed" });
    await emit({ taskId: "tests", status: "cancelling" });
    expect(countBadge()).toBe("1");
    expect(trigger().classList).toContain("tone-cancelling");
    await emit({ taskId: "tests", status: "cancelled" });
    expect(countBadge()).toBeNull();
    expect(trigger().classList).toContain("tone-idle");

    // Updates and pushes drive the trigger; nothing is listed again.
    await vi.advanceTimersByTimeAsync(120_000);
    expect(mocks.list).toHaveBeenCalledOnce();
    expect(countBadge()).toBeNull();
  });

  it("follows pushed snapshots without polling and reconciles on window focus", async () => {
    await mount();
    expect(countBadge()).toBe("1");
    await vi.advanceTimersByTimeAsync(120_000);
    expect(mocks.list).toHaveBeenCalledOnce();
    await pushTasks([task(), task({ taskId: "internal-2", localId: "tests", status: "queued" })]);
    expect(countBadge()).toBe("2");
    await pushTasks([], "session-2");
    expect(countBadge()).toBe("2");
    await pushTasks([]);
    expect(countBadge()).toBeNull();
    mocks.list.mockRejectedValueOnce(new Error("offline"));
    window.dispatchEvent(new Event("focus"));
    await vi.advanceTimersByTimeAsync(0);
    expect(countBadge()).toBeNull();
    mocks.list.mockResolvedValue([task()]);
    window.dispatchEvent(new Event("focus"));
    await vi.advanceTimersByTimeAsync(0);
    expect(mocks.list).toHaveBeenCalledTimes(3);
    expect(countBadge()).toBe("1");
  });

  it("ignores a list response that raced a newer pushed snapshot", async () => {
    const seed = deferred<BackgroundTaskInfo[]>();
    mocks.list.mockReturnValueOnce(seed.promise);
    await mount();
    await pushTasks([]);
    seed.resolve([task()]);
    await flush();
    expect(countBadge()).toBeNull();
  });

  it("defers reconciling a hidden workbench tab until it is shown", async () => {
    const hidden = ref(false);
    host = document.createElement("div");
    document.body.append(host);
    app = createApp(defineComponent({
      setup() {
        provideSurfaceHidden(hidden);
        return () => h(BackgroundTasksIndicator, { sessionId: "session-1" });
      },
    }));
    app.mount(host);
    await flush();
    expect(mocks.list).toHaveBeenCalledTimes(1);

    hidden.value = true;
    await flush();
    window.dispatchEvent(new Event("focus"));
    await vi.advanceTimersByTimeAsync(60_000);
    expect(mocks.list).toHaveBeenCalledTimes(1);

    hidden.value = false;
    await flush();
    await vi.advanceTimersByTimeAsync(0);
    expect(mocks.list).toHaveBeenCalledTimes(2);
  });

  it("replays updates that race a list request over the older snapshot", async () => {
    const seed = deferred<BackgroundTaskInfo[]>();
    mocks.list.mockReturnValueOnce(seed.promise);
    await mount();
    await emit({ taskId: "tests", status: "running" });
    await emit({ status: "completed" });
    seed.resolve([task()]);
    await flush();
    expect(countBadge()).toBe("1");
    await emit({ taskId: "tests", status: "completed" });
    expect(countBadge()).toBeNull();
  });

  it("marks unseen failures with a red dot that survives chat switches and clears once seen", async () => {
    const state = await mount();
    await emit({ status: "failed", output: "<persisted-output>\nExit code: 1\nerror CS1002" });
    expect(countBadge()).toBeNull();
    expect(failureDot()).not.toBeNull();
    expect(trigger().getAttribute("aria-label")).toContain("chat.backgroundTasks.summary.failed 1");

    state.sessionId = "session-2";
    await flush();
    expect(failureDot()).toBeNull();
    // Chat surfaces remount on session switches; unseen failures outlive them.
    app!.unmount();
    host.remove();
    await mount("session-1");
    expect(failureDot()).not.toBeNull();

    mocks.list.mockResolvedValue([]);
    await open();
    expect(failureDot()).toBeNull();
    const text = panel()!.textContent!;
    expect(text).toContain("chat.backgroundTasks.failedGroup 1");
    expect(text).toContain("Build project");
    expect(text).toContain("Exit code: 1");
    expect(text).not.toContain("persisted-output");
    expect(panel()!.querySelectorAll(".background-task-row")).toHaveLength(0);

    await emit({ taskId: "lint", toolName: "python", status: "failed", output: "Traceback" });
    expect(panel()!.textContent).toContain("chat.backgroundTasks.failedGroup 2");
    trigger().click();
    await flush();
    expect(failureDot()).toBeNull();
    await open();
    expect(panel()!.textContent).not.toContain("chat.backgroundTasks.failedGroup");
    trigger().click();
    await flush();

    await emit({ taskId: "lint", status: "failed" });
    expect(failureDot()).not.toBeNull();
    // Resuming the task under the same id makes the failure stale.
    await emit({ taskId: "lint", status: "queued" });
    expect(failureDot()).toBeNull();
  });

  it("is green when idle and blue while session or process tasks run", async () => {
    mocks.list.mockResolvedValue([]);
    await mount();
    expect(trigger().className).toContain("tone-idle");
    await emitApp();
    expect(trigger().className).toContain("tone-running");
    expect(countBadge()).toBe("1");
    await emitApp({ status: "completed" });
    expect(trigger().className).toContain("tone-idle");
    expect(countBadge()).toBeNull();
  });

  it("shows process tasks without a session and returns to the empty state once they finish", async () => {
    mocks.listApp.mockResolvedValue([appTask()]);
    await mount(null);
    expect(trigger().className).toContain("tone-running");
    expect(countBadge()).toBe("1");
    await open();
    const text = () => panel()!.textContent!;
    expect(text()).toContain("chat.backgroundTasks.appGroup");
    expect(text()).toContain("chat.backgroundTasks.app.worktree feat/x");
    expect(text()).toContain("worktrees.progress.creating");
    expect(text()).toContain("45% · 45/100");
    expect(text()).toContain("0:12");
    expect(text()).not.toContain("chat.backgroundTasks.empty");
    expect(panel()!.querySelector(".background-task-stop")).toBeNull();
    expect(panel()!.querySelector<HTMLProgressElement>("progress")!.value).toBe(45);
    expect(mocks.list).not.toHaveBeenCalled();
    await vi.advanceTimersByTimeAsync(2000);
    expect(text()).toContain("0:14");
    await emitApp({ progress: { phase: "finalizing", completed: 0, total: null } });
    expect(text()).toContain("worktrees.progress.finalizing");
    expect(text()).not.toContain("45%");
    await emitApp({ status: "completed" });
    // Background tasks are always available, so the trigger and panel stay.
    expect(text()).not.toContain("chat.backgroundTasks.appGroup");
    expect(text()).toContain("chat.backgroundTasks.empty");
    expect(trigger().className).toContain("tone-idle");
    expect(countBadge()).toBeNull();
  });

  it("lists process tasks next to the session's and reports their failures once", async () => {
    await mount();
    await emitApp();
    await open();
    expect(panel()!.querySelectorAll(".background-task-row")).toHaveLength(1);
    expect(panel()!.querySelectorAll(".background-task-app")).toHaveLength(1);
    expect(panel()!.textContent).toContain("chat.backgroundTasks.activeGroup");
    await emitApp({ status: "failed", error: "git worktree failed: disk full" });
    expect(panel()!.querySelectorAll(".background-task-app")).toHaveLength(0);
    const failure = panel()!.querySelector(".background-task-failure")!;
    expect(failure.textContent).toContain("chat.backgroundTasks.app.worktree feat/x");
    expect(failure.textContent).toContain("git worktree failed: disk full");
    expect(failure.querySelector(".background-task-id")).toBeNull();
    trigger().click();
    await flush();
    expect(failureDot()).toBeNull();
    await emitApp({ taskId: "app-2", status: "failed", error: "WORKTREE_NEW_PROJECT_REQUIRED" });
    expect(failureDot()).not.toBeNull();
    await open();
    expect(panel()!.textContent).toContain("worktrees.budget.retry");
  });

  it("does not let an older process snapshot revive a task that finished meanwhile", async () => {
    const snapshot = deferred<AppTaskInfo[]>();
    mocks.listApp.mockReturnValue(snapshot.promise);
    mocks.list.mockResolvedValue([]);
    await mount();
    await emitApp({ status: "completed" });
    await emitApp({ taskId: "app-2", title: "feat/y" });
    snapshot.resolve([appTask({ progress: { phase: "creating", completed: 90, total: 100 } })]);
    await flush();
    expect(countBadge()).toBe("1");
    await open();
    expect(panel()!.textContent).toContain("feat/y");
    expect(panel()!.textContent).not.toContain("feat/x");
  });
});
