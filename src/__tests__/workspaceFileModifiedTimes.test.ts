import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { effectScope, nextTick, ref } from "vue";
import { useWorkspaceFileModifiedTimes, type WorkspaceFileTimeTarget } from "../composables/useWorkspaceFileModifiedTimes";
import type { ProjectExplorerFileRevision } from "../types/workbench";

const mocks = vi.hoisted(() => ({ revision: vi.fn(), release: vi.fn(), handlers: [] as Array<(event: any) => void> }));
vi.mock("../services/workspaceExplorer", () => ({
  projectExplorerFileRevision: mocks.revision,
  subscribeWorkspaceFileChanges: vi.fn((handler) => { mocks.handlers.push(handler); return Promise.resolve(mocks.release); }),
}));
const cleanups: Array<() => void> = [];
const flush = async () => { for (let i = 0; i < 10; i++) await Promise.resolve(); };
const revision = (seconds: number, exists = true): ProjectExplorerFileRevision => ({ exists, size: 1, modifiedAtNanos: String(seconds * 1e9), key: String(seconds) });
const file = { projectId: "p", path: "C:/work/readme.md" };
beforeEach(() => { vi.clearAllMocks(); mocks.handlers.length = 0; mocks.revision.mockResolvedValue(revision(1_700_000_000)); });
afterEach(() => { cleanups.splice(0).forEach((dispose) => dispose()); });

function setup(initial: WorkspaceFileTimeTarget[]) {
  const files = ref(initial), tick = ref(0), scope = effectScope();
  const times = scope.run(() => useWorkspaceFileModifiedTimes(() => files.value, () => tick.value))!;
  cleanups.push(() => scope.stop());
  return { files, tick, times, scope };
}

describe("workspace file modified times", () => {
  it("converts metadata to seconds, reuses cached visible rows, and scopes by project", async () => {
    const fixture = setup([file, { ...file }]);
    await flush();
    expect(mocks.revision).toHaveBeenCalledTimes(1);
    expect(fixture.times.modifiedAt(file)).toBe(1_700_000_000);
    fixture.files.value = [];
    await nextTick();
    fixture.files.value = [file, { ...file, projectId: "another" }];
    await nextTick(); await flush();
    expect(mocks.revision).toHaveBeenCalledTimes(2);
    mocks.revision.mockResolvedValue(revision(1_700_000_030));
    fixture.tick.value++;
    await nextTick(); await flush();
    expect(fixture.times.modifiedAt(file)).toBe(1_700_000_030);
    expect(mocks.revision).toHaveBeenCalledTimes(4);
  });

  it("refreshes matching file events and clears unavailable timestamps", async () => {
    const fixture = setup([file]); await flush();
    const changed = (projectId: string, path: string) => mocks.handlers[0]!({ projectId, payload: { path } });
    changed("elsewhere", "readme.md"); changed("p", "other.md");
    await flush(); expect(mocks.revision).toHaveBeenCalledTimes(1);
    mocks.revision.mockResolvedValueOnce(revision(1_700_000_050));
    changed("p", "readme.md"); await flush();
    expect(fixture.times.modifiedAt(file)).toBe(1_700_000_050);
    mocks.revision.mockResolvedValueOnce(revision(0, false));
    changed("p", "C:\\work\\readme.md"); await flush();
    expect(fixture.times.modifiedAt(file)).toBeUndefined();
    mocks.revision.mockRejectedValueOnce(new Error("file unavailable"));
    fixture.tick.value++; await nextTick(); await flush();
    expect(fixture.times.modifiedAt(file)).toBeUndefined();
  });

  it("bounds concurrent reads and discards pending results after disposal", async () => {
    const resolves: Array<(value: ProjectExplorerFileRevision) => void> = [];
    mocks.revision.mockImplementation(() => new Promise((resolve) => resolves.push(resolve)));
    const files = Array.from({ length: 8 }, (_, index) => ({ ...file, path: `${file.path}.${index}` }));
    const fixture = setup(files);
    expect(mocks.revision).toHaveBeenCalledTimes(4);
    fixture.scope.stop();
    resolves.forEach((resolve) => resolve(revision(1_700_000_000)));
    await flush();
    expect(mocks.revision).toHaveBeenCalledTimes(4);
    expect(fixture.times.modifiedAt(files[0]!)).toBeUndefined();
    expect(mocks.release).toHaveBeenCalledTimes(1);
  });
});
