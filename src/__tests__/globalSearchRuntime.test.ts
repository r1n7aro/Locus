// @vitest-environment jsdom
import { createApp, h, nextTick, reactive, type App } from "vue";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import GlobalSearch from "../components/workbench/GlobalSearch.vue";
import { useGlobalSearchSettings } from "../composables/useGlobalSearchSettings";
import { searchGlobalPage, type GlobalSearchRequest } from "../services/globalSearch";

vi.mock("../services/globalSearch", async (original) => ({ ...await original<typeof import("../services/globalSearch")>(), searchGlobalPage: vi.fn() }));
vi.mock("../i18n", () => ({ locale: { value: "zh" }, t: (key: string, ...args: string[]) => [key, ...args].join(" ") }));
const apps: App[] = [];
function mount() {
  const host = document.createElement("div");
  document.body.append(host);
  const props = reactive({ active: true, targets: [{ projectId: "a", projectName: "Project A", workspaceRef: { checkoutId: "a" } }], openResult: vi.fn(async () => {}) });
  const app = createApp({ render: () => h(GlobalSearch, props) });
  apps.push(app);
  app.mount(host);
  return props;
}
function key(key: string, options: KeyboardEventInit = {}) {
  const event = new KeyboardEvent("keydown", { key, bubbles: true, cancelable: true, ...options });
  (document.activeElement ?? window).dispatchEvent(event);
  return event;
}
beforeEach(() => {
  vi.useFakeTimers();
  vi.mocked(searchGlobalPage).mockReset().mockImplementation(async (request: GlobalSearchRequest) => ({
    matches: request.source === "knowledgeTitle" ? [{ kind: "knowledge", id: "doc", title: "Shader <img src=x>", excerpt: "Shader content", field: "title", docType: "design", path: "shader.md", messageId: null, archived: false, modifiedAt: 1_700_000_000_000 }] : [], nextCursor: null,
  }));
  const { set } = useGlobalSearchSettings();
  for (const field of ["enabled", "knowledgeTitle", "knowledgeContent", "sessionTitle", "sessionContent"] as const) set(field, true);
});
afterEach(() => { apps.splice(0).forEach((app) => app.unmount()); document.body.replaceChildren(); vi.useRealTimers(); });

describe("global search keyboard interaction", () => {
  it("identifies sessions and every document category with text, including untyped documents", async () => {
    const types = ["design", "plan", "memory", "skill", "reference", null] as const;
    vi.mocked(searchGlobalPage).mockImplementation(async (request) => ({
      matches: request.source === "knowledgeTitle" ? types.map((docType) => ({
        kind: "knowledge" as const, id: String(docType), title: `Shader ${docType}`, excerpt: "", field: "title" as const,
        docType, path: `${docType}.md`, messageId: null, archived: false, modifiedAt: 1_700_000_000_000,
      })) : request.source === "sessionTitle" && !request.archived ? [{
        kind: "session" as const, id: "session", title: "Shader session", excerpt: "", field: "title" as const,
        docType: null, path: null, messageId: null, archived: false, modifiedAt: 1_700_000_000_000,
      }] : [], nextCursor: null,
    }));
    mount(); key("f", { ctrlKey: true }); await nextTick();
    const input = document.querySelector<HTMLInputElement>(".search-input")!;
    input.value = "shader"; input.dispatchEvent(new Event("input", { bubbles: true }));
    await vi.advanceTimersByTimeAsync(120); await nextTick();
    const rows = [...document.querySelectorAll<HTMLElement>(".search-result")];
    expect(rows.map((row) => row.querySelector(".search-result-category")?.textContent)).toEqual([
      "knowledge.type.design", "knowledge.type.plan", "knowledge.type.memory", "knowledge.type.skill",
      "REF", "globalSearch.knowledge", "globalSearch.session",
    ]);
    expect(rows.every((row) => !row.querySelector("svg"))).toBe(true);
    expect(rows[3]?.getAttribute("aria-label")).toContain("knowledge.type.skill");
  });

  it("pages through rows with and without content, keeps selection visible, and preserves native text navigation", async () => {
    useGlobalSearchSettings().set("knowledgeTitle", false);
    vi.mocked(searchGlobalPage).mockImplementation(async (request) => ({
      matches: request.source === "knowledgeContent" ? Array.from({ length: 50 }, (_, index) => ({
        kind: "knowledge" as const, id: `doc-${index}`, title: `Document ${index}`, excerpt: index % 2 ? "Shader body" : "",
        field: "content" as const, docType: "design" as const, path: `${index}.md`, messageId: null, archived: false, modifiedAt: 1_700_000_000_000,
      })) : [], nextCursor: null,
    }));
    const props = mount(); key("f", { ctrlKey: true }); await nextTick();
    const input = document.querySelector<HTMLInputElement>(".search-input")!;
    input.value = "shader"; input.dispatchEvent(new Event("input", { bubbles: true }));
    await vi.advanceTimersByTimeAsync(120); await nextTick();
    const scroller = document.querySelector<HTMLElement>(".search-scroll")!;
    Object.defineProperty(scroller, "clientHeight", { configurable: true, value: 180 });
    const selectedRow = () => document.getElementById(input.getAttribute("aria-activedescendant") ?? "");
    expect(key("Home").defaultPrevented).toBe(false);
    expect(key("End").defaultPrevented).toBe(false);
    expect(key("PageDown").defaultPrevented).toBe(true); await nextTick();
    expect(selectedRow()?.textContent).toContain("Document 3");
    expect(scroller.scrollTop).toBe(36);
    key("PageUp"); await nextTick();
    expect(selectedRow()?.textContent).toContain("Document 0");
    key("End", { ctrlKey: true }); await nextTick();
    expect(selectedRow()?.textContent).toContain("Document 49");
    key("PageDown"); await nextTick();
    expect(selectedRow()?.textContent).toContain("Document 49");
    key("Home", { ctrlKey: true }); await nextTick();
    expect(selectedRow()?.textContent).toContain("Document 0");
    // Wheel scrolling can unmount the active row; never reference a missing option.
    scroller.scrollTop = 1500; scroller.dispatchEvent(new Event("scroll")); await nextTick();
    expect(input.hasAttribute("aria-activedescendant")).toBe(false);
    key("ArrowDown"); await nextTick();
    expect(selectedRow()?.textContent).toContain("Document 1");
    expect(document.activeElement).toBe(input);
    key("Enter"); await vi.advanceTimersByTimeAsync(0);
    expect(props.openResult).toHaveBeenCalledWith(expect.objectContaining({ id: "doc-1" }));
  });

  it("keeps input focus for pointer selection and allows keyboard retry after an open failure", async () => {
    const props = mount();
    props.openResult.mockRejectedValueOnce(new Error("Could not open document"));
    key("f", { ctrlKey: true }); await nextTick();
    const input = document.querySelector<HTMLInputElement>(".search-input")!;
    input.value = "shader"; input.dispatchEvent(new Event("input", { bubbles: true }));
    await vi.advanceTimersByTimeAsync(120); await nextTick();
    const row = document.querySelector<HTMLButtonElement>(".search-result")!;
    const down = new MouseEvent("mousedown", { bubbles: true, cancelable: true, button: 0 });
    row.dispatchEvent(down);
    expect(down.defaultPrevented).toBe(true);
    row.click(); await vi.advanceTimersByTimeAsync(0); await nextTick();
    expect(document.activeElement).toBe(input);
    expect(document.querySelector(".search-error")?.textContent).toContain("Could not open document");
    key("Enter"); await vi.advanceTimersByTimeAsync(0); await nextTick();
    expect(props.openResult).toHaveBeenCalledTimes(2);
    expect(document.querySelector(".global-search")).toBeNull();
  });

  it("shows modification times for both result kinds and refreshes relative time while open", async () => {
    const now = Date.UTC(2026, 8, 18, 10, 0);
    vi.setSystemTime(now);
    vi.mocked(searchGlobalPage).mockImplementation(async (request) => ({
      matches: request.source === "knowledgeTitle" ? [{
        kind: "knowledge", id: "doc", title: "Shader doc", excerpt: "", field: "title", docType: "design", path: "shader.md",
        messageId: null, archived: false, modifiedAt: now - 30_000,
      }] : request.source === "sessionTitle" && !request.archived ? [{
        kind: "session", id: "session", title: "Shader session", excerpt: "", field: "title", docType: null, path: null,
        messageId: null, archived: false, modifiedAt: now - 3_600_000,
      }] : [], nextCursor: null,
    }));
    mount(); key("f", { ctrlKey: true }); await nextTick();
    const input = document.querySelector<HTMLInputElement>(".search-input")!;
    input.value = "shader"; input.dispatchEvent(new Event("input", { bubbles: true }));
    await vi.advanceTimersByTimeAsync(120); await nextTick();
    const times = [...document.querySelectorAll<HTMLTimeElement>(".search-result-time")];
    expect(times.map((time) => time.dateTime)).toEqual([new Date(now - 30_000).toISOString(), new Date(now - 3_600_000).toISOString()]);
    expect(times.map((time) => time.textContent)).toEqual(["time.justNow", "time.hoursAgo 1"]);
    expect(times.every((time) => time.title.includes("2026"))).toBe(true);
    await vi.advanceTimersByTimeAsync(60_000); await nextTick();
    expect(document.querySelector(".search-result-time")?.textContent).toBe("time.minutesAgo 1");
    key("Escape"); await nextTick();
    expect(vi.getTimerCount()).toBe(0);
  });
  it("renders a bounded number of rows and keeps arrow selection available after scrolling", async () => {
    vi.mocked(searchGlobalPage).mockImplementation(async (request) => ({
      matches: request.source === "knowledgeTitle" ? Array.from({ length: 500 }, (_, index) => ({
        kind: "knowledge" as const, id: `doc-${index}`, title: `Shader ${index}`, excerpt: "shader", field: "title" as const,
        docType: "design" as const, path: `shader-${index}.md`, messageId: null, archived: false, modifiedAt: 1_700_000_000_000,
      })) : [], nextCursor: null,
    }));
    const props = mount(); key("f", { ctrlKey: true }); await nextTick();
    const input = document.querySelector<HTMLInputElement>(".search-input")!;
    input.value = "shader"; input.dispatchEvent(new Event("input", { bubbles: true }));
    await vi.advanceTimersByTimeAsync(120); await nextTick();
    expect(document.querySelectorAll(".search-result").length).toBeLessThanOrEqual(18);
    key("ArrowUp"); await nextTick();
    expect(document.querySelector('[aria-selected="true"]')?.textContent).toContain("Shader 499");
    key("Enter"); await vi.advanceTimersByTimeAsync(0);
    expect(props.openResult).toHaveBeenCalledWith(expect.objectContaining({ id: "doc-499" }));
  });

  it("always uses two lines with content matches, previews or an empty-content label", async () => {
    vi.mocked(searchGlobalPage).mockImplementation(async (request) => ({
      matches: request.source === "knowledgeTitle" ? [
        { kind: "knowledge", id: "short", title: "Shader", excerpt: "Shader body also matches", field: "content", docType: "design", path: "short.md", messageId: null, archived: false, modifiedAt: 1_700_000_000_000 },
        { kind: "knowledge", id: "description", title: "Document", excerpt: "Shader content", field: "content", docType: "design", path: "description.md", messageId: null, archived: false, modifiedAt: 1_700_000_000_000 },
        { kind: "knowledge", id: "preview", title: "Shader overview", excerpt: "Document introduction", field: "title", docType: "design", path: "overview.md", messageId: null, archived: false, modifiedAt: 1_700_000_000_000 },
        { kind: "session", id: "empty", title: "Shader chat", excerpt: "", field: "title", docType: null, path: null, messageId: null, archived: false, modifiedAt: 1_700_000_000_000 },
      ] : [], nextCursor: null,
    }));
    mount(); key("f", { ctrlKey: true }); await nextTick();
    const input = document.querySelector<HTMLInputElement>(".search-input")!;
    input.value = "shader"; input.dispatchEvent(new Event("input", { bubbles: true }));
    await vi.advanceTimersByTimeAsync(120); await nextTick();
    const rows = [...document.querySelectorAll<HTMLElement>(".search-result")];
    expect(rows.every((row) => row.style.height === "54px")).toBe(true);
    expect(rows[0]?.querySelector(".search-result-snippet")?.textContent).toBe("Shader body also matches");
    expect(rows[0]?.querySelector(".search-result-snippet mark")?.textContent).toBe("Shader");
    expect(rows[0]?.querySelector(".search-result-meta")).toBeNull();
    expect(rows[0]?.title).toContain("short.md");
    expect(rows[0]?.title).toContain("Project A");
    expect(rows[1]?.style.height).toBe("54px");
    expect(rows[1]?.style.transform).toBe("translateY(54px)");
    expect(rows[1]?.querySelector(".search-result-snippet")?.textContent).toBe("Shader content");
    expect(rows[2]?.querySelector(".search-result-snippet")?.textContent).toBe("Document introduction");
    expect(rows[3]?.querySelector(".search-result-snippet")?.textContent).toBe("globalSearch.noPreview");
    expect(rows.every((row) => document.getElementById(row.getAttribute("aria-describedby") ?? "")?.classList.contains("search-result-snippet"))).toBe(true);
  });

  it("shows only project names as metadata when results span multiple projects", async () => {
    const props = mount();
    props.targets.push({ projectId: "b", projectName: "Project B", workspaceRef: { checkoutId: "b" } });
    key("f", { ctrlKey: true }); await nextTick();
    const input = document.querySelector<HTMLInputElement>(".search-input")!;
    input.value = "shader"; input.dispatchEvent(new Event("input", { bubbles: true }));
    await vi.advanceTimersByTimeAsync(120); await nextTick();
    expect([...document.querySelectorAll(".search-result-meta")].map((item) => item.textContent))
      .toEqual(["Project A", "Project B"]);
    expect(document.querySelector(".search-result-heading .search-result-meta")).not.toBeNull();
  });

  it("retains matching content when the user has disabled title search", async () => {
    const { set } = useGlobalSearchSettings();
    set("knowledgeTitle", false); set("sessionTitle", false); set("sessionContent", false);
    vi.mocked(searchGlobalPage).mockResolvedValue({ matches: [{
      kind: "knowledge", id: "body", title: "Shader", excerpt: "Matching Shader body", field: "content", docType: "design", path: "shader.md", messageId: null, archived: false, modifiedAt: 1_700_000_000_000,
    }], nextCursor: null });
    mount(); key("f", { ctrlKey: true }); await nextTick();
    const input = document.querySelector<HTMLInputElement>(".search-input")!;
    input.value = "shader"; input.dispatchEvent(new Event("input", { bubbles: true }));
    await vi.advanceTimersByTimeAsync(120); await nextTick();
    expect(document.querySelector(".search-result-snippet")?.textContent).toBe("Matching Shader body");
  });

  it("loads the next batch on bottom scroll without a button or overlapping requests", async () => {
    const { set } = useGlobalSearchSettings();
    for (const field of ["knowledgeContent", "sessionTitle", "sessionContent"] as const) set(field, false);
    const page = (number: number) => ({ matches: Array.from({ length: 20 }, (_, index) => ({
      kind: "knowledge" as const, id: `${number}-${index}`, title: `Shader ${number}-${index}`, excerpt: "Shader", field: "title" as const,
      docType: "design" as const, path: `${number}-${index}.md`, messageId: null, archived: false, modifiedAt: 1_700_000_000_000,
    })), nextCursor: number < 3 ? String(number + 1) : null });
    let release!: (value: ReturnType<typeof page>) => void;
    vi.mocked(searchGlobalPage).mockImplementation(async (request) => {
      const number = Number(request.cursor ?? 0);
      if (number === 2) return new Promise((resolve) => { release = resolve; });
      return page(number);
    });
    mount(); key("f", { ctrlKey: true }); await nextTick();
    const input = document.querySelector<HTMLInputElement>(".search-input")!;
    input.value = "shader"; input.dispatchEvent(new Event("input", { bubbles: true }));
    await vi.advanceTimersByTimeAsync(120); await nextTick();
    expect(searchGlobalPage).toHaveBeenCalledTimes(2);
    expect(document.querySelector(".search-footer")).toBeNull();
    const scroller = document.querySelector<HTMLElement>(".search-scroll")!;
    Object.defineProperty(scroller, "clientHeight", { configurable: true, value: 432 });
    scroller.scrollTop = 40 * 54 - 432;
    scroller.dispatchEvent(new Event("scroll"));
    await vi.advanceTimersByTimeAsync(0);
    expect(searchGlobalPage).toHaveBeenCalledTimes(3);
    for (let i = 0; i < 10; i++) scroller.dispatchEvent(new Event("scroll"));
    await vi.advanceTimersByTimeAsync(0);
    expect(searchGlobalPage).toHaveBeenCalledTimes(3);
    release(page(2)); await vi.advanceTimersByTimeAsync(0); await nextTick();
    expect(searchGlobalPage).toHaveBeenCalledTimes(4);
    expect(document.querySelector<HTMLElement>(".search-list")?.style.height).toBe("4320px");
    expect(scroller.scrollTop).toBe(1728);
    expect(document.querySelectorAll(".search-result").length).toBeLessThanOrEqual(18);
  });
  it("opens with Ctrl+F, highlights safe text, selects with Enter, and restores focus on Escape", async () => {
    const previous = document.createElement("input"); document.body.append(previous); previous.focus();
    const props = mount();
    expect(key("f", { ctrlKey: true }).defaultPrevented).toBe(true);
    await nextTick();
    const input = document.querySelector<HTMLInputElement>(".search-input")!;
    expect(document.activeElement).toBe(input);
    input.value = "shader"; input.dispatchEvent(new Event("input", { bubbles: true }));
    await vi.advanceTimersByTimeAsync(120); await nextTick();
    expect(document.querySelector("mark")?.textContent).toBe("Shader");
    expect(document.querySelector(".search-result img")).toBeNull();
    key("Enter"); await vi.advanceTimersByTimeAsync(0); await nextTick();
    expect(props.openResult).toHaveBeenCalledWith(expect.objectContaining({ id: "doc", target: expect.objectContaining({ projectId: "a" }) }));
    expect(document.querySelector(".global-search")).toBeNull();
    previous.focus(); key("f", { ctrlKey: true }); await nextTick();
    expect(key("Escape").defaultPrevented).toBe(true); await nextTick();
    expect(document.activeElement).toBe(previous);
  });

  it("ignores inactive views, disabled search and IME confirmation", async () => {
    const props = mount(); props.active = false; await nextTick();
    expect(key("f", { ctrlKey: true }).defaultPrevented).toBe(false);
    props.active = true; useGlobalSearchSettings().set("enabled", false); await nextTick();
    expect(key("f", { ctrlKey: true }).defaultPrevented).toBe(false);
    useGlobalSearchSettings().set("enabled", true); key("f", { ctrlKey: true }); await nextTick();
    const input = document.querySelector<HTMLInputElement>(".search-input")!;
    input.dispatchEvent(new CompositionEvent("compositionstart", { bubbles: true }));
    input.value = "中"; input.dispatchEvent(new Event("input", { bubbles: true }));
    await vi.advanceTimersByTimeAsync(200);
    expect(searchGlobalPage).not.toHaveBeenCalled();
    expect(key("Enter", { isComposing: true }).defaultPrevented).toBe(false);
    expect(props.openResult).not.toHaveBeenCalled();
    input.dispatchEvent(new CompositionEvent("compositionend", { bubbles: true }));
    await vi.advanceTimersByTimeAsync(120);
    expect(searchGlobalPage).toHaveBeenCalled();
    props.active = false; await nextTick();
    expect(document.querySelector(".global-search")).toBeNull();
  });
});
