// @vitest-environment jsdom
import { createApp, h, nextTick, reactive } from "vue";
import { afterEach, describe, expect, it } from "vitest";
import WorkbenchEditorStack from "../components/workbench/WorkbenchEditorStack.vue";
import {
  isWorkbenchSessionEditorInput,
  selectMountedEditorIds,
} from "../components/workbench/workbenchEditorMounting";
import type { WorkbenchEditorGroup, WorkbenchEditorInput } from "../types/workbench";

function editor(id: string, kind: "session" | "knowledge" = "session"): WorkbenchEditorInput {
  return {
    editorId: id,
    resource: kind === "knowledge"
      ? { kind, projectId: "project", documentId: id }
      : { kind, projectId: "project", sessionId: id },
    title: id, preview: false, pinned: true, dirty: false,
    capabilities: { split: true, detach: true, duplicate: true },
    availability: "available",
  };
}

describe("selectMountedEditorIds", () => {
  const tabs = ["a", "b", "c", "d", "e"].map((id) => editor(id));
  const base = {
    tabs,
    activeEditorId: "e",
    presentedEditorId: "e",
    recentEditorIds: ["e", "d", "c", "b", "a"],
    limit: 3,
    isEvictable: isWorkbenchSessionEditorInput,
    retainedEditorIds: new Set<string>(),
  };

  it("never unloads without a positive limit but still mounts lazily", () => {
    expect([...selectMountedEditorIds({ ...base, limit: 0 })!].sort()).toEqual(["a", "b", "c", "d", "e"]);
    const mounted = selectMountedEditorIds({ ...base, limit: 0, mountedEditorIds: new Set(["e", "b"]) })!;
    expect([...mounted].sort()).toEqual(["b", "e"]);
  });

  it("keeps the active editor and the most recent ones up to the limit", () => {
    expect([...selectMountedEditorIds(base)!]).toEqual(["e", "d", "c"]);
  });

  it("keeps the presented surface while the active editor loads", () => {
    const mounted = selectMountedEditorIds({ ...base, activeEditorId: "a", limit: 1 })!;
    expect([...mounted].sort()).toEqual(["a", "e"]);
  });

  it("keeps retained editors beyond the limit and counts them first", () => {
    const mounted = selectMountedEditorIds({ ...base, retainedEditorIds: new Set(["a", "b"]) })!;
    expect([...mounted].sort()).toEqual(["a", "b", "e"]);
  });

  it("never unmounts editors that are not evictable and does not count them", () => {
    const mixed = [editor("doc", "knowledge"), ...tabs];
    const mounted = selectMountedEditorIds({ ...base, tabs: mixed, limit: 2 })!;
    expect([...mounted].sort()).toEqual(["d", "doc", "e"]);
  });

  it("fills free slots only from editors that are still mounted", () => {
    const mounted = selectMountedEditorIds({ ...base, mountedEditorIds: new Set(["e", "b"]) })!;
    expect([...mounted].sort()).toEqual(["b", "e"]);
  });

  it("ignores recent ids of tabs that are no longer open", () => {
    const mounted = selectMountedEditorIds({ ...base, recentEditorIds: ["e", "gone", "d"], limit: 2 })!;
    expect([...mounted].sort()).toEqual(["d", "e"]);
  });
});

const cleanups: Array<() => void> = [];
afterEach(() => cleanups.splice(0).forEach((cleanup) => cleanup()));

function mountGroups(groups: WorkbenchEditorGroup[], limit = 3) {
  const state = reactive({ groups, limit, retained: new Set<string>() });
  const ready = new Map<string, () => void>();
  const evicted: Array<{ editorId: string; mountedWhenEvicted: boolean }> = [];
  const host = document.createElement("div");
  document.body.appendChild(host);
  const app = createApp({
    render: () => state.groups.map((group) => h(WorkbenchEditorStack, {
      key: group.paneId,
      group,
      mountLimit: state.limit,
      isEvictable: isWorkbenchSessionEditorInput,
      retainedEditorIds: state.retained,
      onEvict: (target: WorkbenchEditorInput) => evicted.push({
        editorId: target.editorId,
        mountedWhenEvicted: !!host.querySelector(`[data-editor-id="${target.editorId}"]`),
      }),
    }, {
      default: (slot: { editor: WorkbenchEditorInput; ready: () => void }) => {
        ready.set(slot.editor.editorId, slot.ready);
        return h("div", slot.editor.editorId);
      },
    })),
  });
  app.mount(host);
  cleanups.push(() => { app.unmount(); host.remove(); });
  const mountedIds = () => Array.from(host.querySelectorAll<HTMLElement>(".workbench-editor-instance"))
    .map((element) => element.dataset.editorId!)
    .sort();
  const activate = async (group: WorkbenchEditorGroup, id: string) => {
    group.activeEditorId = id;
    await nextTick();
    ready.get(id)?.();
    await nextTick();
  };
  return { state, host, ready, evicted, mountedIds, activate };
}

function group(paneId: string, ids: string[]): WorkbenchEditorGroup {
  return reactive<WorkbenchEditorGroup>({
    paneId,
    tabs: ids.map((id) => editor(id)),
    activeEditorId: ids[0] ?? null,
  });
}

describe("workbench editor group mount limit", () => {
  it("unmounts the least recently used session tab and reports it while still mounted", async () => {
    const pane = group("pane", ["a", "b", "c", "d"]);
    const view = mountGroups([pane]);
    await view.activate(pane, "a");
    expect(view.mountedIds()).toEqual(["a"]);
    await view.activate(pane, "b");
    await view.activate(pane, "c");
    expect(view.mountedIds()).toEqual(["a", "b", "c"]);

    await view.activate(pane, "d");
    expect(view.mountedIds()).toEqual(["b", "c", "d"]);
    expect(view.evicted).toEqual([{ editorId: "a", mountedWhenEvicted: true }]);
  });

  it("shows the previous surface until a reloaded tab is ready again", async () => {
    const pane = group("pane", ["a", "b", "c", "d"]);
    const view = mountGroups([pane]);
    for (const id of ["a", "b", "c", "d"]) await view.activate(pane, id);
    const previous = view.host.querySelector<HTMLElement>('[data-editor-id="d"]')!;

    pane.activeEditorId = "a";
    await nextTick();
    expect(view.host.querySelector('[data-editor-id="a"]')?.classList.contains("is-preparing")).toBe(true);
    expect(previous.style.display).not.toBe("none");

    view.ready.get("a")!();
    await nextTick();
    expect(view.mountedIds()).toEqual(["a", "c", "d"]);
    expect(view.evicted.map((entry) => entry.editorId)).toEqual(["a", "b"]);
  });

  it("keeps retained sessions beyond the limit and evicts them once they become idle", async () => {
    const pane = group("pane", ["a", "b", "c"]);
    const view = mountGroups([pane], 1);
    await view.activate(pane, "a");
    view.state.retained.add("a");
    await view.activate(pane, "b");
    expect(view.mountedIds()).toEqual(["a", "b"]);

    view.state.retained.delete("a");
    await nextTick();
    expect(view.mountedIds()).toEqual(["b"]);
    expect(view.evicted.map((entry) => entry.editorId)).toEqual(["a"]);
  });

  it("does not remount unloaded tabs in the background when a slot frees up", async () => {
    const pane = group("pane", ["a", "b", "c", "d"]);
    const view = mountGroups([pane], 2);
    for (const id of ["a", "b", "c", "d"]) await view.activate(pane, id);
    expect(view.mountedIds()).toEqual(["c", "d"]);

    view.state.limit = 3;
    await nextTick();
    expect(view.mountedIds()).toEqual(["c", "d"]);
    await view.activate(pane, "a");
    expect(view.mountedIds()).toEqual(["a", "c", "d"]);
  });

  it("counts each editor group separately", async () => {
    const left = group("left", ["a", "b", "c"]);
    const right = group("right", ["x", "y", "z"]);
    const view = mountGroups([left, right], 2);
    for (const id of ["x", "y"]) await view.activate(right, id);
    for (const id of ["a", "b", "c"]) await view.activate(left, id);
    expect(view.mountedIds()).toEqual(["b", "c", "x", "y"]);
    expect(view.evicted.map((entry) => entry.editorId)).toEqual(["a"]);
  });

  it("mounts restored tabs only when they are first shown, also without a limit", async () => {
    const pane = group("pane", ["a", "b", "c", "d"]);
    const view = mountGroups([pane], 0);
    await view.activate(pane, "a");
    expect(view.mountedIds()).toEqual(["a"]);
    await view.activate(pane, "c");
    expect(view.mountedIds()).toEqual(["a", "c"]);
    expect(view.evicted).toEqual([]);
  });

  it("does not report closed tabs as evicted and keeps every shown tab with no limit", async () => {
    const pane = group("pane", ["a", "b", "c", "d"]);
    const view = mountGroups([pane], 0);
    for (const id of ["a", "b", "c", "d"]) await view.activate(pane, id);
    expect(view.mountedIds()).toEqual(["a", "b", "c", "d"]);

    view.state.limit = 2;
    await nextTick();
    expect(view.mountedIds()).toEqual(["c", "d"]);
    expect(view.evicted.map((entry) => entry.editorId).sort()).toEqual(["a", "b"]);

    pane.tabs = pane.tabs.filter((candidate) => candidate.editorId !== "c");
    await nextTick();
    expect(view.mountedIds()).toEqual(["d"]);
    expect(view.evicted.map((entry) => entry.editorId).sort()).toEqual(["a", "b"]);
  });
});
