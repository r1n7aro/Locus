// @vitest-environment jsdom
import { createPinia, setActivePinia } from "pinia";
import { createApp, h, nextTick } from "vue";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import WorkbenchSecondarySidebar from "../components/workbench/WorkbenchSecondarySidebar.vue";
import { workspaceSecondaryResourceSection, workspaceSecondarySection } from "../components/workbench/workspaceSecondaryNavigation";
import { createWorkbenchEditorInput, useWorkbenchStore, workbenchResourceKey } from "../stores/workbench";

beforeEach(() => {
  localStorage.clear();
  setActivePinia(createPinia());
});
afterEach(() => { document.body.innerHTML = ""; });

describe("workspace secondary navigation", () => {
  it("routes workspace list nodes to secondary navigation", () => {
    expect(["knowledgeRoot", "assetsRoot", "viewsRoot", "archivedRoot", "agentsRoot"].map(workspaceSecondarySection))
      .toEqual(["knowledge", "assets", "views", "archived", "agents"]);
    for (const kind of ["collaboration", "checkout", "session", "knowledge", "localFile", "folder", "newSession"]) {
      expect(workspaceSecondarySection(kind)).toBeNull();
    }
  });

  it("restores individual Agent editors without turning them into list tabs", () => {
    const store = useWorkbenchStore();
    const list = { kind: "section", projectId: "p", section: "agents" } as const;
    const agent = { ...list, agentId: "unity" };
    expect(workspaceSecondaryResourceSection(list)).toBe("agents");
    expect(workspaceSecondaryResourceSection(agent)).toBeNull();
    expect(workbenchResourceKey(agent)).not.toBe(workbenchResourceKey({ ...agent, agentId: "explorer" }));
    store.openEditor("main", createWorkbenchEditorInput(agent, "Unity", { pinned: true, preview: false }));
    store.persist("main");
    setActivePinia(createPinia());
    expect(useWorkbenchStore().ensureWindow("main").groups.main!.tabs[0]?.resource).toEqual(agent);
  });

  it("keeps archived conversations distinct from the old archived list tab across restore", () => {
    const store = useWorkbenchStore();
    const resource = { kind: "section", projectId: "p", section: "archived", sessionId: "a" } as const;
    const list = { kind: "section", projectId: "p", section: "archived" } as const;
    expect(workspaceSecondaryResourceSection(list)).toBe("archived");
    expect(workspaceSecondaryResourceSection(resource)).toBeNull();
    expect(workbenchResourceKey(resource)).not.toBe(workbenchResourceKey(list));
    store.openEditor("main", createWorkbenchEditorInput(resource, "Archived A", { pinned: true, preview: false }));
    store.openEditor("main", createWorkbenchEditorInput({ ...resource, sessionId: "b" }, "Archived B", { pinned: true, preview: false }));
    store.persist("main");
    setActivePinia(createPinia());
    const tabs = useWorkbenchStore().ensureWindow("main").groups.main!.tabs;
    expect(tabs.map((tab) => tab.resource)).toEqual([resource, { ...resource, sessionId: "b" }]);
  });

  it("renders a separate resizable column and closes without editing the tab store", async () => {
    const store = useWorkbenchStore();
    const state = store.ensureWindow("main");
    const session = store.openEditor("main", createWorkbenchEditorInput({ kind: "session", projectId: "p", sessionId: "s" }, "Session"));
    const before = JSON.stringify(state);
    const close = vi.fn();
    const host = document.createElement("div");
    document.body.appendChild(host);
    const app = createApp(() => h(WorkbenchSecondarySidebar, { title: "知识", onClose: close }, () => h("div", "Document list")));
    app.mount(host);
    expect(host.querySelector("aside")?.textContent).toContain("Document list");
    const separator = host.querySelector<HTMLElement>('[role="separator"]')!;
    separator.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowRight", bubbles: true }));
    await nextTick();
    expect(separator.getAttribute("aria-valuenow")).toBe("320");
    host.querySelector<HTMLButtonElement>("button")!.click();
    expect(close).toHaveBeenCalledOnce();
    expect(JSON.stringify(state)).toBe(before);
    expect(store.activeEditor("main")?.editorId).toBe(session.editorId);
    app.unmount();
  });

  it("coalesces pointer moves and commits the final width even before the frame paints", async () => {
    const frames: FrameRequestCallback[] = [];
    const raf = vi.spyOn(window, "requestAnimationFrame").mockImplementation((callback) => { frames.push(callback); return frames.length; });
    const cancel = vi.spyOn(window, "cancelAnimationFrame").mockImplementation(() => {});
    const host = document.createElement("div");
    const app = createApp(WorkbenchSecondarySidebar, { title: "知识" });
    app.mount(host);
    const handle = host.querySelector<HTMLElement>('[role="separator"]')!;
    handle.dispatchEvent(new MouseEvent("mousedown", { button: 0, clientX: 300 }));
    for (let i = 1; i <= 80; i++) document.dispatchEvent(new MouseEvent("mousemove", { clientX: 300 + i }));
    expect(raf).toHaveBeenCalledOnce();
    expect(host.querySelector("aside")!.style.width).toBe("300px");
    document.dispatchEvent(new MouseEvent("mouseup"));
    await nextTick();
    expect(host.querySelector("aside")!.style.width).toBe("380px");
    expect(handle.getAttribute("aria-valuenow")).toBe("380");
    expect(localStorage.getItem("locus:workspaceSecondarySidebarWidth")).toBe("380");
    expect(cancel).toHaveBeenCalledOnce();
    app.unmount();
    raf.mockRestore();
    cancel.mockRestore();
  });

  it("restores independent knowledge settings tabs without treating them as navigation lists", () => {
    const store = useWorkbenchStore();
    const pages = [{kind: "retrieval"}, {kind: "injection"}, {kind: "directory", type: "design", path: "combat"}] as const;
    for (const page of pages) {
      const resource = { kind: "section", projectId: "p", section: "knowledge", knowledgePage: page } as const;
      expect(workspaceSecondaryResourceSection(resource)).toBeNull();
      store.openEditor("main", createWorkbenchEditorInput(resource, page.kind, { preview: false, pinned: true }));
    }
    store.persist("main");
    setActivePinia(createPinia());
    const tabs = useWorkbenchStore().ensureWindow("main").groups.main!.tabs;
    expect(tabs).toHaveLength(3);
    expect(tabs.map((tab) => tab.resource.kind === "section" && tab.resource.knowledgePage)).toEqual(pages);
  });
});
