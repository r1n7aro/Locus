// @vitest-environment jsdom
import { afterEach, describe, expect, it } from "vitest";
import { createApp, h, nextTick, ref } from "vue";
import WorkspaceTree, { type WorkspaceTreeItem } from "../components/explorer/WorkspaceTree.vue";
import FileTreeList from "../components/explorer/FileTreeList.vue";

const dispose: Array<() => void> = [];
afterEach(() => { dispose.splice(0).forEach((cleanup) => cleanup()); document.body.replaceChildren(); });

describe("workspace pinned separator layout", () => {
  it("keeps the viewport with stable siblings when a visible session is promoted", async () => {
    const items = ref(Array.from({ length: 100 }, (_, index) => ({ key: String(index) })));
    const list = ref<InstanceType<typeof FileTreeList>>();
    const host = document.createElement("div"); document.body.append(host);
    const app = createApp(() => h(FileTreeList, { ref: list, items: items.value, rowHeight: 30 }, {
      item: ({ item }: { item: { key: string } }) => h("div", { "data-row": item.key }, item.key),
    }));
    app.mount(host); dispose.push(() => app.unmount());
    const scroll = host.querySelector<HTMLElement>(".file-tree-list")!;
    Object.defineProperty(scroll, "clientHeight", { value: 90 });
    scroll.scrollTop = 1_500;
    const moved = items.value[50]!;
    items.value = [moved, ...items.value.filter((item) => item !== moved)];
    await nextTick(); await nextTick();
    expect(scroll.scrollTop).toBe(1_500);
    scroll.scrollTop = 0;
    items.value = [items.value[80]!, ...items.value.filter((_, index) => index !== 80)];
    await nextTick(); await nextTick();
    expect(scroll.scrollTop).toBe(0);
  });

  it("removes the divider and its spacing when the last pin is cleared", async () => {
    const items = ref<WorkspaceTreeItem[]>([
      { key: "session", treeRow: { key: "session", name: "Session", depth: 0, kind: "file", pinned: true, pinnedSectionEnd: true, starred: true } },
      { key: "file", treeRow: { key: "file", name: "File", depth: 0, kind: "file" } },
    ]);
    const host = document.createElement("div"); document.body.append(host);
    const app = createApp(() => h(WorkspaceTree, { items: items.value, rowTabIndex: 0 }));
    app.mount(host); dispose.push(() => app.unmount());
    expect(host.querySelectorAll('[role="separator"]')).toHaveLength(1);
    expect(host.querySelector('[data-pinned-tree-key="session"]')).not.toBeNull();
    items.value[0]!.treeRow!.pinned = false;
    items.value[0]!.treeRow!.pinnedSectionEnd = false;
    await nextTick();
    expect(host.querySelector('[role="separator"]')).toBeNull();
    expect(host.querySelector('.is-pinned')).toBeNull();
    expect(host.querySelector('.workspace-tree-star')).not.toBeNull();
    expect(host.querySelector<HTMLButtonElement>('.workspace-tree-row')?.tabIndex).toBe(0);
  });

  it("includes divider spacing in virtual scroll and removes it from row index calculations", async () => {
    const gapAfterIndex = ref(0);
    const list = ref<InstanceType<typeof FileTreeList>>();
    const items = Array.from({ length: 100 }, (_, index) => ({ key: String(index) }));
    const host = document.createElement("div"); document.body.append(host);
    const app = createApp(() => h(FileTreeList, { ref: list, items, rowHeight: 30, overscan: 0, gapAfterIndex: gapAfterIndex.value, gapHeight: 16 }, {
      item: ({ item }: { item: { key: string } }) => h("div", { "data-row": item.key, style: "height:30px" }, item.key),
      gap: () => h("div", { "data-gap": "", style: "height:16px" }),
    }));
    app.mount(host); dispose.push(() => app.unmount());
    const scroll = host.querySelector<HTMLElement>('.file-tree-list')!;
    Object.defineProperty(scroll, "clientHeight", { value: 60 });
    list.value!.scrollToIndex(50, { align: "center" });
    await nextTick();
    expect(scroll.scrollTop).toBe(1501);
    expect(host.querySelector('[data-row]')?.getAttribute('data-row')).toBe("49");
    expect(host.querySelector<HTMLElement>('.file-tree-list-spacer')?.style.height).toBe("1486px");
    expect(host.querySelector('[data-gap]')).toBeNull();
    gapAfterIndex.value = -1;
    await nextTick();
    list.value!.scrollToIndex(50, { align: "center" });
    await nextTick();
    expect(scroll.scrollTop).toBe(1485);
    expect(host.querySelector<HTMLElement>('.file-tree-list-spacer')?.style.height).toBe("1470px");
  });
});
