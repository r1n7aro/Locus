import { beforeEach, describe, expect, it, vi } from "vitest";
import { ref } from "vue";
import { useGarbageCollection } from "../composables/useGarbageCollection";
import { useCommandRegistry } from "../composables/useCommandRegistry";
import { findSkillCommandConflict } from "../composables/skillCommands";
import type { DatabaseCollection } from "../services/garbageCollection";

const mocks = vi.hoisted(() => ({
  collect: vi.fn(), addNotice: vi.fn(), removeNotice: vi.fn(),
}));
vi.mock("../services/garbageCollection", () => ({ garbageCollection: mocks.collect }));
vi.mock("../stores/notification", () => ({ useNotificationStore: () => mocks }));

beforeEach(() => {
  vi.clearAllMocks();
  mocks.addNotice.mockReturnValue("progress");
});

const space = (bytes: number) => ({ databaseBytes: bytes, walBytes: 0, logicalBytes: bytes, freePageBytes: 0 });
const complete: DatabaseCollection = {
  kind: "session", path: "sessions/locus.db", skipped: false, error: null,
  result: { before: space(4096), after: space(2048), reclaimedBytes: 2048, warning: null },
};

describe("database garbage collection", () => {
  it("reserves the builtin and only offers it as a standalone action", () => {
    const registry = useCommandRegistry(ref([]), ref("unity"));
    expect(registry.findExactAvailableCommand("/garbage-collection")?.commandKind).toBe("action");
    expect(registry.filteredCommands("/garbage")).toEqual([]);
    expect(registry.filteredCommands("/garbage", { includeActions: true })[0]?.commandType).toBe("garbage-collection");
    expect(findSkillCommandConflict("/garbage-collection", [])?.type).toBe("builtin");
  });

  it("captures the checkout, prevents overlapping runs, and removes progress after partial failure", async () => {
    let resolve!: (value: DatabaseCollection[]) => void;
    mocks.collect.mockReturnValue(new Promise<DatabaseCollection[]>((done) => { resolve = done; }));
    const { collect } = useGarbageCollection();
    const workspace = { checkoutId: "a", expectedGeneration: 1 };
    const first = collect(workspace);
    workspace.checkoutId = "b";
    await collect(workspace);
    expect(mocks.collect).toHaveBeenCalledExactlyOnceWith({ checkoutId: "a", expectedGeneration: 1 });
    resolve([complete, { kind: "project", path: "project/locus.db", skipped: false, result: null, error: "busy" }]);
    await first;
    expect(mocks.addNotice).toHaveBeenCalledWith("success", expect.stringContaining("2.00 KiB"), expect.anything());
    expect(mocks.addNotice).toHaveBeenCalledWith("error", expect.stringContaining("busy"), expect.anything());
    expect(mocks.removeNotice).toHaveBeenCalledWith("progress");
  });

  it("supports no project, reports checkpoint warnings and missing databases", async () => {
    mocks.collect.mockResolvedValue([
      { ...complete, result: { ...complete.result!, warning: "checkpoint busy" } },
      { kind: "project", path: "project/locus.db", skipped: true, result: null, error: null },
    ]);
    await useGarbageCollection().collect(null);
    expect(mocks.collect).toHaveBeenCalledWith(null);
    expect(mocks.addNotice).toHaveBeenCalledWith("warning", expect.stringContaining("checkpoint busy"), expect.anything());
    expect(mocks.addNotice.mock.calls.filter(([level]) => level === "success")).toHaveLength(0);
  });

  it("reports command failure and permits a retry", async () => {
    mocks.collect.mockRejectedValueOnce(new Error("disk full")).mockResolvedValueOnce([complete]);
    const { collect } = useGarbageCollection();
    await collect(null);
    expect(mocks.addNotice).toHaveBeenCalledWith("error", expect.stringContaining("disk full"), expect.anything());
    await collect(null);
    expect(mocks.collect).toHaveBeenCalledTimes(2);
    expect(mocks.removeNotice).toHaveBeenCalledTimes(2);
  });
});
