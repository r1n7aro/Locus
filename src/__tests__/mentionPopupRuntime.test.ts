// @vitest-environment jsdom
import { createApp, h, nextTick, reactive, type App } from "vue";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import MentionPopup, { type MentionDisplayEntry } from "../components/chat/MentionPopup.vue";

vi.mock("../i18n", () => ({ t: (key: string) => key }));

let app: App;
let host: HTMLElement;
let availableBottom = 700;
let viewportHeight = 480;
const entries = (count: number): MentionDisplayEntry[] => Array.from({ length: count }, (_, index) => ({
  relPath: `docs/Document${index}.md`, name: `Document ${index}`, parentPath: "docs", isDir: false,
}));

async function mountPopup(count = 2000) {
  const state = reactive({
    visible: true, mode: "search" as "search" | "browse", entries: entries(count),
    selectedIndex: 0, breadcrumbs: [] as string[], query: "Document", loading: false, showEmpty: false,
  });
  const select = vi.fn();
  const navigateRoot = vi.fn();
  host = document.createElement("div");
  document.body.append(host);
  app = createApp({ render: () => h(MentionPopup, {
    ...state, onSelect: select, onNavigateRoot: navigateRoot,
    "onUpdate:selectedIndex": (index: number) => { state.selectedIndex = index; },
  }) });
  app.mount(host);
  await nextTick();
  await nextTick();
  const list = host.querySelector<HTMLElement>(".mention-results")!;
  return { state, select, navigateRoot, list };
}

beforeEach(() => {
  availableBottom = 700;
  viewportHeight = 480;
  vi.spyOn(HTMLElement.prototype, "clientHeight", "get").mockImplementation(function (this: HTMLElement) {
    return this.classList.contains("mention-results") ? viewportHeight : 0;
  });
  vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockImplementation(function (this: HTMLElement) {
    return { bottom: this.classList.contains("mention-popup") ? availableBottom : 0 } as DOMRect;
  });
});

afterEach(() => {
  app?.unmount();
  host?.remove();
  vi.restoreAllMocks();
});

describe("mention popup viewport", () => {
  it("bounds DOM size for large directories and reveals offscreen keyboard selections", async () => {
    const { state, list } = await mountPopup();
    expect(host.querySelectorAll(".mention-item").length).toBeLessThan(30);
    expect(host.querySelector(".mention-item")?.getAttribute("aria-setsize")).toBe("2000");
    state.selectedIndex = 1999;
    await nextTick();
    await nextTick();
    expect(list.scrollTop).toBe(2000 * 48 - viewportHeight);
    expect(host.querySelector(".highlighted .mention-name")?.textContent).toBe("Document 1999");
    expect(host.querySelectorAll(".mention-item").length).toBeLessThan(30);
    state.selectedIndex = 0;
    await nextTick();
    await nextTick();
    expect(list.scrollTop).toBe(0);
    expect(host.querySelector(".highlighted .mention-name")?.textContent).toBe("Document 0");
  });

  it("resets a scrolled list when a query changes even if result count stays the same", async () => {
    const { state, list } = await mountPopup(60);
    list.scrollTop = 1800;
    state.query = "Updated";
    state.entries = entries(60).map((entry) => ({ ...entry, name: `Updated ${entry.name}` }));
    await nextTick();
    await nextTick();
    expect(list.scrollTop).toBe(0);
    expect(host.querySelector(".highlighted .mention-name")?.textContent).toBe("Updated Document 0");
  });

  it("uses a taller viewport without extending above a short window", async () => {
    await mountPopup();
    const popup = host.querySelector<HTMLElement>(".mention-popup")!;
    expect(popup.style.maxHeight).toBe("520px");
    availableBottom = 360;
    window.dispatchEvent(new Event("resize"));
    await nextTick();
    expect(popup.style.maxHeight).toBe("352px");
    availableBottom = 800;
    window.dispatchEvent(new Event("resize"));
    await nextTick();
    expect(popup.style.maxHeight).toBe("520px");
  });

  it("keeps pointer hover from scrolling or moving selection under a stationary pointer", async () => {
    const { state, list } = await mountPopup(50);
    const rows = host.querySelectorAll<HTMLElement>(".mention-item");
    rows[11]!.dispatchEvent(new MouseEvent("mousemove", { clientX: 30, clientY: 200 }));
    await nextTick();
    await nextTick();
    expect(state.selectedIndex).toBe(11);
    expect(list.scrollTop).toBe(0);
    rows[12]!.dispatchEvent(new MouseEvent("mousemove", { clientX: 30, clientY: 200 }));
    expect(state.selectedIndex).toBe(11);
  });

  it("does not pull the list back to the initial selection when more results arrive", async () => {
    const { state, list } = await mountPopup(100);
    list.scrollTop = 1200;
    state.entries = entries(200);
    await nextTick();
    await nextTick();
    expect(list.scrollTop).toBe(1200);
  });

  it("refreshes cached fragments when a loaded candidate is renamed in place", async () => {
    const { state } = await mountPopup(1);
    state.entries[0]!.name = "Renamed document";
    state.entries[0]!.parentPath = "archive";
    await nextTick();
    expect(host.querySelector(".mention-name")?.textContent).toBe("Renamed document");
    expect(host.querySelector(".mention-path")?.textContent).toBe("archive");
  });

  it("supports button activation without a mouse and preserves text highlights on selection changes", async () => {
    const { state, select, navigateRoot } = await mountPopup(3);
    const highlightedText = host.querySelector(".mention-name-fragment.is-match");
    expect(highlightedText?.textContent).toBe("Document");
    state.selectedIndex = 1;
    await nextTick();
    host.querySelector<HTMLButtonElement>(".highlighted .mention-select")!.click();
    expect(select).toHaveBeenCalledExactlyOnceWith(state.entries[1]);
    expect(host.querySelector(".mention-name-fragment.is-match")).toBe(highlightedText);
    state.mode = "browse";
    await nextTick();
    host.querySelector<HTMLButtonElement>(".mention-crumb")!.click();
    expect(navigateRoot).toHaveBeenCalledOnce();
  });
});
