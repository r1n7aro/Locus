// @vitest-environment jsdom

import { afterEach, describe, expect, it, vi } from "vitest";
import { createApp, defineComponent, h, nextTick, onMounted, onUnmounted, ref } from "vue";
import WorkbenchSplitHost from "../components/workbench/WorkbenchSplitHost.vue";
import type { WorkbenchEditorGroup, WorkbenchSplitNode } from "../types/workbench";

const cleanups: Array<() => void> = [];
afterEach(() => {
  cleanups.splice(0).reverse().forEach((cleanup) => cleanup());
  document.body.replaceChildren();
  vi.restoreAllMocks();
});

const leaf = (paneId: string): WorkbenchSplitNode => ({ kind: "group", paneId });
const split = (
  splitId: string,
  first: WorkbenchSplitNode,
  second: WorkbenchSplitNode,
  orientation: "horizontal" | "vertical" = "horizontal",
): WorkbenchSplitNode => ({ kind: "split", splitId, first, second, orientation, ratio: 0.5 });

async function mountHost(initial = leaf("a"), ownerDocument = document) {
  const mounts: string[] = [];
  const unmounts: string[] = [];
  const connectedOnMount: boolean[] = [];
  const Probe = defineComponent({
    props: { paneId: { type: String, required: true }, focused: Boolean, activeEditorId: String },
    setup(props) {
      const paneId = props.paneId;
      const input = ref<HTMLInputElement | null>(null);
      onMounted(() => {
        mounts.push(paneId);
        connectedOnMount.push(input.value?.isConnected === true);
      });
      onUnmounted(() => unmounts.push(paneId));
      return () => h("input", {
        ref: input,
        "data-probe": props.paneId,
        "data-focused": props.focused,
        "data-active-editor": props.activeEditorId,
      });
    },
  });
  const node = ref(initial);
  const focusedPaneId = ref("a");
  const activeDropKey = ref<string | null>(null);
  const groups = ref<Record<string, WorkbenchEditorGroup>>(
    Object.fromEntries(["a", "b", "c"].map((paneId) => [paneId, { paneId, tabs: [], activeEditorId: null }])),
  );
  const focusPane = vi.fn();
  const resize = vi.fn();
  const host = ownerDocument.createElement("div");
  ownerDocument.body.append(host);
  const app = createApp({
    setup: () => () => h(WorkbenchSplitHost, {
      node: node.value,
      groups: groups.value,
      focusedPaneId: focusedPaneId.value,
      activeDropKey: activeDropKey.value,
      "onFocus-pane": focusPane,
      onResize: resize,
    }, {
      group: ({ paneId, focused, group }: { paneId: string; focused: boolean; group?: WorkbenchEditorGroup }) => (
        h(Probe, { key: paneId, paneId, focused, activeEditorId: group?.activeEditorId ?? undefined })
      ),
    }),
  });
  app.mount(host);
  let disposed = false;
  const dispose = () => {
    if (disposed) return;
    disposed = true;
    app.unmount();
    host.remove();
  };
  cleanups.push(dispose);
  await nextTick();
  const input = (paneId: string) => host.querySelector<HTMLInputElement>(`[data-probe="${paneId}"]`)!;
  return { node, groups, focusedPaneId, activeDropKey, host, input, mounts, unmounts, connectedOnMount, focusPane, resize, dispose };
}

describe("workbench split editor lifecycle", () => {
  it("preserves existing editors through splitting, nesting, reordering and collapse", async () => {
    const fixture = await mountHost();
    const original = fixture.input("a");
    original.value = "unsaved editor state";
    original.setSelectionRange(2, 7);
    expect(fixture.mounts).toEqual(["a"]);

    fixture.node.value = split("root", leaf("a"), leaf("b"));
    await nextTick();
    const second = fixture.input("b");
    fixture.node.value = split("root", split("nested", leaf("c"), leaf("a"), "vertical"), leaf("b"));
    await nextTick();
    fixture.node.value = split("root", leaf("b"), split("nested", leaf("a"), leaf("c"), "vertical"));
    await nextTick();
    expect(fixture.input("a")).toBe(original);
    expect(fixture.input("b")).toBe(second);
    expect(fixture.mounts).toEqual(["a", "b", "c"]);
    expect(fixture.unmounts).toEqual([]);
    expect(fixture.connectedOnMount).toEqual([true, true, true]);
    expect(original.value).toBe("unsaved editor state");
    expect([original.selectionStart, original.selectionEnd]).toEqual([2, 7]);

    fixture.node.value = leaf("a");
    await nextTick();
    expect(fixture.input("a")).toBe(original);
    expect(fixture.unmounts.slice().sort()).toEqual(["b", "c"]);
    expect(fixture.host.querySelectorAll("input")).toHaveLength(1);
    fixture.dispose();
    expect(fixture.unmounts.slice().sort()).toEqual(["a", "b", "c"]);
  });

  it("updates slot props and drop previews without recreating editor content", async () => {
    const fixture = await mountHost(split("root", leaf("a"), leaf("b")));
    const original = fixture.input("b");
    fixture.groups.value.b = { paneId: "b", tabs: [], activeEditorId: "editor-b" };
    fixture.focusedPaneId.value = "b";
    fixture.activeDropKey.value = "editor:b:right";
    await nextTick();
    expect(fixture.input("b")).toBe(original);
    expect(original.dataset.focused).toBe("true");
    expect(original.dataset.activeEditor).toBe("editor-b");
    expect(original.closest("section")?.querySelector(".workbench-editor-split-preview.is-right")).not.toBeNull();
    fixture.activeDropKey.value = null;
    await nextTick();
    expect(fixture.host.querySelector(".workbench-editor-split-preview")).toBeNull();
    fixture.input("a").dispatchEvent(new MouseEvent("pointerdown", { bubbles: true }));
    expect(fixture.focusPane).toHaveBeenCalledExactlyOnceWith("a");
    expect(fixture.mounts).toEqual(["a", "b"]);
    expect(fixture.unmounts).toEqual([]);
  });

  it("restores recorded scroll offsets after reparenting and respects scrolling back to zero", async () => {
    const fixture = await mountHost();
    const editor = fixture.input("a");
    editor.scrollTop = 320;
    editor.scrollLeft = 24;
    editor.dispatchEvent(new Event("scroll"));
    // Model Chromium's offset reset during a DOM move (jsdom has no layout).
    editor.scrollTop = 0;
    editor.scrollLeft = 0;
    fixture.node.value = split("root", leaf("a"), leaf("b"));
    await nextTick();
    await nextTick();
    expect(fixture.input("a")).toBe(editor);
    expect([editor.scrollLeft, editor.scrollTop]).toEqual([24, 320]);

    editor.scrollTop = 0;
    editor.scrollLeft = 0;
    editor.dispatchEvent(new Event("scroll"));
    fixture.node.value = leaf("a");
    await nextTick();
    await nextTick();
    expect([editor.scrollLeft, editor.scrollTop]).toEqual([0, 0]);
  });

  it("forwards resize events through nested layout branches", async () => {
    const fixture = await mountHost(split("root", leaf("a"), split("nested", leaf("b"), leaf("c"), "vertical")));
    const separators = fixture.host.querySelectorAll<HTMLElement>('[role="separator"]');
    separators[1]!.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowDown", bubbles: true }));
    expect(fixture.resize).toHaveBeenLastCalledWith("nested", 0.52, true);
    separators[0]!.dispatchEvent(new KeyboardEvent("keydown", { key: "Home", bubbles: true }));
    expect(fixture.resize).toHaveBeenLastCalledWith("root", 0.18, true);
  });

  it("isolates matching pane IDs across hosts and owner documents", async () => {
    const main = await mountHost();
    const frame = document.createElement("iframe");
    document.body.append(frame);
    const child = await mountHost(leaf("a"), frame.contentDocument!);
    const mainInput = main.input("a");
    const childInput = child.input("a");
    child.node.value = split("root", leaf("a"), leaf("b"));
    await nextTick();
    expect(main.input("a")).toBe(mainInput);
    expect(child.input("a")).toBe(childInput);
    expect(childInput.ownerDocument).toBe(frame.contentDocument);
    expect(childInput.closest('[data-workbench-pane-id="a"]')?.ownerDocument).toBe(frame.contentDocument);
    expect(main.mounts).toEqual(["a"]);
    expect(child.mounts).toEqual(["a", "b"]);
  });
});
