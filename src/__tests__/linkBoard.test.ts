// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from "vitest";
import { createApp, h, nextTick, ref, type App } from "vue";
import { LinkBoard, type LinkBoardConnection, type LinkBoardEndpoint } from "../components/link-board";

let app: App | null = null;
afterEach(() => { app?.unmount(); app = null; document.body.innerHTML = ""; vi.unstubAllGlobals(); vi.restoreAllMocks(); });
async function flush() { await nextTick(); await nextTick(); }
async function mount(initial: LinkBoardConnection[] = [], multiple = false) {
  const links = ref(initial);
  const sources = ref<LinkBoardEndpoint[]>([{ id: "a", label: "Source A" }, { id: "b", label: "Source B" }]);
  const targets = ref<LinkBoardEndpoint[]>([{ id: "x", label: "Target X" }, { id: "y", label: "Target Y" }]);
  const readonly = ref(false);
  const changes = vi.fn((value: LinkBoardConnection[]) => { links.value = value; });
  const root = document.createElement("div"); document.body.append(root);
  app = createApp({ setup: () => () => h(LinkBoard, {
    sources: sources.value, targets: targets.value, modelValue: links.value, readonly: readonly.value, multiple,
    "onUpdate:modelValue": changes,
  }) });
  app.mount(root); await flush();
  const button = (label: string) => Array.from(root.querySelectorAll("button")).find((element) => element.textContent === label)!;
  const click = async (label: string) => { button(label).click(); await flush(); };
  return { root, links, sources, targets, readonly, changes, button, click };
}

describe("reusable LinkBoard", () => {
  it("connects, replaces one-to-one mappings, and unlinks through controlled model updates", async () => {
    const original = [{ source: "a", target: "x" }];
    const panel = await mount(original);
    await panel.click("Source B");
    expect(panel.button("Source B").getAttribute("aria-pressed")).toBe("true");
    await panel.click("Target X");
    expect(panel.links.value).toEqual([{ source: "b", target: "x" }]);
    expect(original).toEqual([{ source: "a", target: "x" }]);
    await panel.click("Source B"); await panel.click("Target Y");
    expect(panel.links.value).toEqual([{ source: "b", target: "y" }]);
    await panel.click("Target Y"); expect(panel.links.value).toEqual([]);
  });

  it("supports multiple links and toggles one pair without removing unrelated pairs", async () => {
    const panel = await mount([{ source: "a", target: "x" }], true);
    await panel.click("Source A"); await panel.click("Target Y");
    await panel.click("Source B"); await panel.click("Target X");
    expect(panel.links.value).toHaveLength(3);
    await panel.click("Source A"); await panel.click("Target X");
    expect(panel.links.value).toEqual([{ source: "a", target: "y" }, { source: "b", target: "x" }]);
  });

  it("cancels selection with Escape and respects readonly, disabled and removed endpoints", async () => {
    const panel = await mount();
    await panel.click("Source A");
    panel.button("Source A").dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
    await panel.click("Target X"); expect(panel.changes).not.toHaveBeenCalled();
    panel.targets.value[0]!.disabled = true; await flush();
    await panel.click("Source A"); await panel.click("Target X"); expect(panel.changes).not.toHaveBeenCalled();
    panel.sources.value = [{ id: "b", label: "Source B" }]; await flush();
    await panel.click("Target Y"); expect(panel.changes).not.toHaveBeenCalled();
    panel.readonly.value = true; await flush();
    expect(panel.button("Source B").disabled).toBe(true);
    await panel.click("Source B"); await panel.click("Target Y"); expect(panel.changes).not.toHaveBeenCalled();
  });

  it("reacts to parent data and resize notifications and disconnects observers on disposal", async () => {
    const disconnect = vi.fn();
    let resized!: ResizeObserverCallback;
    vi.stubGlobal("ResizeObserver", class { constructor(callback: ResizeObserverCallback) { resized = callback; } observe() {} unobserve() {} disconnect = disconnect; });
    const panel = await mount();
    const rect = (left: number, top: number, width: number, height: number) => ({ left, top, width, height, right: left + width, bottom: top + height, x: left, y: top, toJSON() {} });
    panel.root.querySelector<HTMLElement>(".link-board-content")!.getBoundingClientRect = () => rect(0, 0, 400, 120);
    panel.button("Source A").getBoundingClientRect = () => rect(10, 20, 100, 30);
    panel.button("Target X").getBoundingClientRect = () => rect(290, 20, 100, 30);
    panel.links.value = [{ source: "a", target: "x" }, { source: "missing", target: "x" }]; await flush();
    expect(panel.root.querySelectorAll("path")).toHaveLength(1);
    expect(panel.root.querySelector("path")!.getAttribute("d")).toContain("M 110 35");
    panel.button("Target X").getBoundingClientRect = () => rect(310, 50, 100, 30);
    resized([], {} as ResizeObserver); await flush();
    expect(panel.root.querySelector("path")!.getAttribute("d")).toContain("310 65");
    app!.unmount(); app = null; expect(disconnect).toHaveBeenCalledOnce();
    resized([], {} as ResizeObserver); await flush(); expect(panel.root.innerHTML).toBe("");
  });
});
