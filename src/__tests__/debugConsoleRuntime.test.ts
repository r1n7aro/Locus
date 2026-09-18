// @vitest-environment jsdom
import { afterAll, afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { DebugConsoleEntry } from "../types";
import {
  clearDebugConsole,
  getDebugConsoleSnapshot,
  initDebugConsole,
  refreshDebugConsole,
  saveDebugConsoleLogExport,
  subscribeDebugConsole,
  teardownDebugConsole,
} from "../services/debugConsole";

const mocks = vi.hoisted(() => {
  const originalInfo = console.info;
  console.info = vi.fn();
  return {
  originalInfo,
  invoke: vi.fn(),
  listen: vi.fn(),
  unlisten: vi.fn(),
  onBatch: null as null | ((event: { payload: { entries: DebugConsoleEntry[]; droppedCount: number } }) => void),
}; });
vi.mock("../services/locusRuntime", () => ({ invokeLocusRuntime: mocks.invoke }));
vi.mock("../services/tauriRuntime", () => ({ hasTauriWindowRuntime: () => true }));
vi.mock("@tauri-apps/api/event", () => ({ listen: mocks.listen }));

const savedConsole = { log: console.log, info: mocks.originalInfo, warn: console.warn, error: console.error, debug: console.debug };
const cleanups: Array<() => void> = [];

function entry(index: number, message = `log ${index}`): DebugConsoleEntry {
  return { id: `backend-${index}`, timestampMs: index, level: "debug", source: "backend", module: "test", target: "test", message };
}

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((done) => { resolve = done; });
  return { promise, resolve };
}

async function settle() {
  for (let index = 0; index < 12; index += 1) await Promise.resolve();
}

function snapshotCalls() {
  return mocks.invoke.mock.calls.filter(([command]) => command === "get_log_entries");
}

beforeEach(async () => {
  vi.useFakeTimers();
  mocks.invoke.mockReset().mockResolvedValue([]);
  mocks.unlisten.mockReset();
  mocks.listen.mockReset().mockImplementation(async (_name, callback) => {
    mocks.onBatch = callback;
    return mocks.unlisten;
  });
  await clearDebugConsole();
  mocks.invoke.mockClear();
});

afterEach(() => {
  cleanups.splice(0).forEach((cleanup) => cleanup());
  teardownDebugConsole();
  mocks.onBatch = null;
  vi.useRealTimers();
});
afterAll(() => Object.assign(console, savedConsole));

describe("debug console runtime", () => {
  it("shares bridge setup and in-flight snapshots, and initializes only once", async () => {
    const pending = deferred<DebugConsoleEntry[]>();
    mocks.invoke.mockReturnValue(pending.promise);
    const requests = [initDebugConsole(), initDebugConsole(), refreshDebugConsole()];
    await settle();
    expect(mocks.listen).toHaveBeenCalledTimes(1);
    expect(snapshotCalls()).toHaveLength(1);
    pending.resolve([entry(1)]);
    await Promise.all(requests);
    await initDebugConsole();
    expect(snapshotCalls()).toHaveLength(1);
    mocks.invoke.mockResolvedValue([entry(1), entry(2)]);
    await Promise.all([refreshDebugConsole(), refreshDebugConsole()]);
    expect(snapshotCalls()).toHaveLength(2);
    expect(getDebugConsoleSnapshot().map((item) => item.id)).toEqual(["backend-1", "backend-2"]);
  });

  it("retries failed snapshots without registering another live listener", async () => {
    mocks.invoke.mockRejectedValueOnce(new Error("offline"));
    await expect(refreshDebugConsole()).rejects.toThrow("offline");
    mocks.invoke.mockResolvedValue([entry(2)]);
    await refreshDebugConsole();
    expect(mocks.listen).toHaveBeenCalledTimes(1);
    expect(snapshotCalls()).toHaveLength(2);
    expect(getDebugConsoleSnapshot()).toEqual([entry(2)]);
  });

  it("releases a listener that finishes registering after teardown", async () => {
    const pending = deferred<() => void>();
    mocks.listen.mockReturnValueOnce(pending.promise);
    const request = initDebugConsole();
    teardownDebugConsole();
    pending.resolve(mocks.unlisten);
    await request;
    expect(mocks.unlisten).toHaveBeenCalledTimes(1);
    expect(snapshotCalls()).toHaveLength(0);
  });

  it("does not restore logs from a snapshot that finishes after clearing", async () => {
    const pending = deferred<DebugConsoleEntry[]>();
    mocks.invoke.mockReturnValueOnce(pending.promise);
    const request = refreshDebugConsole();
    await settle();
    await clearDebugConsole();
    pending.resolve([entry(1)]);
    await request;
    expect(getDebugConsoleSnapshot()).toEqual([]);
    mocks.invoke.mockResolvedValue([entry(2)]);
    await refreshDebugConsole();
    expect(getDebugConsoleSnapshot()).toEqual([entry(2)]);
  });

  it("ignores snapshots and events belonging to a torn-down bridge", async () => {
    const pending = deferred<DebugConsoleEntry[]>();
    mocks.invoke.mockReturnValueOnce(pending.promise);
    const request = refreshDebugConsole();
    await settle();
    const oldOnBatch = mocks.onBatch!;
    teardownDebugConsole();
    pending.resolve([entry(1)]);
    await request;
    oldOnBatch({ payload: { entries: [entry(2)], droppedCount: 0 } });
    expect(getDebugConsoleSnapshot()).toEqual([]);
    await refreshDebugConsole();
    expect(mocks.listen).toHaveBeenCalledTimes(2);
  });

  it("bounds display text and preserves Unicode", async () => {
    mocks.invoke.mockResolvedValue([entry(1, "𠮷".repeat(20_000)), entry(2, "x".repeat(5000))]);
    await refreshDebugConsole();
    const snapshot = getDebugConsoleSnapshot();
    expect(snapshot[0]!.message.length).toBeLessThanOrEqual(16_000);
    expect(snapshot[0]!.message).toMatch(/^(𠮷)+ …\(truncated\)$/u);
    expect(snapshot[1]!.message).toHaveLength(5000);
    mocks.invoke.mockResolvedValue(Array.from({ length: 2000 }, (_, index) => entry(index + 3, "x".repeat(16_000))));
    await refreshDebugConsole();
    const bounded = getDebugConsoleSnapshot();
    expect(bounded.length).toBeLessThan(100);
    expect(bounded.reduce((sum, row) => sum + row.message.length, 0)).toBeLessThanOrEqual(1024 * 1024);
    expect(bounded[bounded.length - 1]?.id).toBe("backend-2002");
    await clearDebugConsole();
    mocks.invoke.mockResolvedValue([entry(2003)]);
    await refreshDebugConsole();
    expect(getDebugConsoleSnapshot()).toEqual([entry(2003)]);
  });

  it("coalesces log bursts with a byte budget and reports dropped live entries", async () => {
    await initDebugConsole();
    const listener = vi.fn();
    cleanups.push(subscribeDebugConsole(listener));
    for (let index = 0; index < 500; index += 1) {
      mocks.onBatch!({ payload: { entries: [entry(index, "x".repeat(16_000))], droppedCount: 0 } });
    }
    expect(listener).not.toHaveBeenCalled();
    await vi.advanceTimersByTimeAsync(16);
    expect(listener).toHaveBeenCalledTimes(1);
    const snapshot = getDebugConsoleSnapshot();
    expect(snapshot.length).toBeLessThan(100);
    expect(snapshot.some((item) => item.id === "backend-499")).toBe(true);
    expect(snapshot[snapshot.length - 1]?.message).toContain("Live console dropped");
  });

  it("exports from disk after forwarding every full record even when the display ring evicts them", async () => {
    await initDebugConsole();
    const fullMessage = `${"正文".repeat(700_000)}完整尾部`;
    console.info("[test]", fullMessage);
    for (let index = 0; index < 2500; index += 1) console.info(`[test] record ${index}`);
    expect(getDebugConsoleSnapshot().length).toBeLessThanOrEqual(2000);
    await clearDebugConsole();
    await saveDebugConsoleLogExport("all.log");
    const forwarded = mocks.invoke.mock.calls
      .filter(([command]) => command === "append_frontend_logs")
      .flatMap(([, args]) => args.entries);
    expect(forwarded).toHaveLength(2501);
    expect(forwarded[0].message).toBe(fullMessage);
    expect(forwarded[2500].message).toBe("record 2499");
    expect(mocks.invoke).toHaveBeenLastCalledWith("save_log_export", { filePath: "all.log" });
    expect(getDebugConsoleSnapshot()).toEqual([]);
  });

  it("waits for an in-flight write before exporting and retries a failed write without losing text", async () => {
    await initDebugConsole();
    console.info("[test] pending full log");
    const pending = deferred<unknown>();
    mocks.invoke.mockReturnValueOnce(pending.promise);
    const exporting = saveDebugConsoleLogExport("pending.log");
    await settle();
    expect(mocks.invoke.mock.calls.some(([command]) => command === "save_log_export")).toBe(false);
    pending.resolve(undefined);
    await exporting;
    console.info("[test] retry full log");
    mocks.invoke.mockRejectedValueOnce(new Error("write failed"));
    await expect(saveDebugConsoleLogExport("retry.log")).rejects.toThrow("write failed");
    await saveDebugConsoleLogExport("retry.log");
    const writes = mocks.invoke.mock.calls.filter(([command]) => command === "append_frontend_logs");
    expect(writes[writes.length - 1]![1].entries[0].message).toBe("retry full log");
    expect(mocks.invoke).toHaveBeenLastCalledWith("save_log_export", { filePath: "retry.log" });
  });
});
