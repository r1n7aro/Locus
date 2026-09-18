// @vitest-environment jsdom
import { createApp, h, nextTick, ref, type App } from "vue";
import { afterEach, describe, expect, it, vi } from "vitest";
import { useFileChangeRevalidation } from "../composables/useFileChangeRevalidation";
import type { ProjectExplorerFileRevision } from "../types/workbench";

const events = vi.hoisted(() => ({ listeners: new Set<(event: never) => void>() }));
vi.mock("../services/workspaceExplorer", () => ({
  subscribeWorkspaceFileChanges: vi.fn(async (listener: (event: never) => void) => {
    events.listeners.add(listener); return () => events.listeners.delete(listener);
  }),
}));
const revision = (key: string): ProjectExplorerFileRevision => ({ key, exists: true, size: 10, modifiedAtNanos: key });
function emit(path = "Assets/items.csv", checkoutId = "a", workspaceGeneration = 1) {
  for (const listener of events.listeners) listener({ checkoutId, workspaceGeneration, payload: { path } } as never);
}
const apps: App[] = [];
async function mount() {
  vi.useFakeTimers();
  const active = ref(true), path = ref("Assets/items.csv"), current = ref(revision("before"));
  const probe = vi.fn<() => Promise<ProjectExplorerFileRevision>>().mockResolvedValue(revision("after"));
  const onChanged = vi.fn((next: ProjectExplorerFileRevision) => { current.value = next; });
  let checks!: ReturnType<typeof useFileChangeRevalidation>;
  const app = createApp({ setup() {
    checks = useFileChangeRevalidation({ active: () => active.value, currentRevision: () => current.value,
      workspaceRef: () => ({ checkoutId: "a", expectedGeneration: 1 }),
      workspacePaths: () => [path.value, `${path.value}.view`], debounceMs: 80, maxWaitMs: 240, probe, onChanged });
    return () => h("div");
  } });
  apps.push(app); app.mount(document.createElement("div")); await nextTick();
  return { active, path, current, probe, onChanged, checks, app };
}
afterEach(() => { for (const app of apps.splice(0)) app.unmount(); events.listeners.clear(); vi.useRealTimers(); });

describe("file revalidation scheduling", () => {
  it("coalesces data/sidecar events while bounding a continuous stream to 240 ms", async () => {
    const { probe, onChanged } = await mount();
    for (let i = 0; i < 4; i++) { emit(i % 2 ? "Assets/items.csv.view" : "Assets/items.csv"); await vi.advanceTimersByTimeAsync(60); }
    expect(probe).toHaveBeenCalledTimes(1);
    expect(onChanged).toHaveBeenCalledTimes(1);
    await vi.advanceTimersByTimeAsync(500);
    expect(probe).toHaveBeenCalledTimes(1);
  });

  it("keeps one request in flight and checks the newest write after it finishes", async () => {
    const { probe, onChanged, current } = await mount();
    let resolve!: (value: ProjectExplorerFileRevision) => void;
    probe.mockReturnValueOnce(new Promise((done) => { resolve = done; }));
    emit(); await vi.advanceTimersByTimeAsync(80);
    for (let i = 0; i < 10; i++) emit();
    await vi.advanceTimersByTimeAsync(240);
    expect(probe).toHaveBeenCalledTimes(1);
    resolve(revision("intermediate")); await vi.advanceTimersByTimeAsync(80);
    expect(probe).toHaveBeenCalledTimes(2);
    expect(onChanged).toHaveBeenCalledTimes(2);
    expect(current.value.key).toBe("after");
  });

  it("manual checks consume a scheduled check instead of duplicating it", async () => {
    const { probe, checks } = await mount();
    emit(); await checks.checkNow(); await vi.advanceTimersByTimeAsync(500);
    expect(probe).toHaveBeenCalledTimes(1);
  });

  it("ignores other resources/scopes and checks inactive tabs only on activation", async () => {
    const { probe, active } = await mount();
    emit("Assets/other.csv"); emit("Assets/items.csv", "b"); emit("Assets/items.csv", "a", 2);
    await vi.advanceTimersByTimeAsync(500); expect(probe).not.toHaveBeenCalled();
    active.value = false; await nextTick();
    emit(); window.dispatchEvent(new Event("focus"));
    await vi.advanceTimersByTimeAsync(500); expect(probe).not.toHaveBeenCalled();
    active.value = true; await nextTick(); await vi.advanceTimersByTimeAsync(80);
    expect(probe).toHaveBeenCalledTimes(1);
    await vi.advanceTimersByTimeAsync(10_000); expect(probe).toHaveBeenCalledTimes(1);
  });

  it("does not apply a probe from a previously selected file", async () => {
    const { probe, checks, path, onChanged } = await mount();
    let resolve!: (value: ProjectExplorerFileRevision) => void;
    probe.mockReturnValueOnce(new Promise((done) => { resolve = done; }));
    const pending = checks.checkNow(); path.value = "Assets/new.csv"; await nextTick();
    resolve(revision("stale")); await pending;
    expect(onChanged).not.toHaveBeenCalled();
  });
});
