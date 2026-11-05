// @vitest-environment jsdom
import { createApp, h, nextTick } from "vue";
import { afterEach, describe, expect, it, vi } from "vitest";
import WorkbenchEditorTabs from "../components/workbench/WorkbenchEditorTabs.vue";
import { provideInternalDragController } from "../composables/useInternalDrag";
import { createWorkbenchEditorInput } from "../stores/workbench";
import type { WorkbenchEditorGroup, WorkbenchEditorInput } from "../types/workbench";

vi.mock("../i18n", () => ({ t: (key: string) => key }));
vi.mock("../stores/chat", () => ({
  useChatStore: () => ({ streamingSessionIds: new Set(), sessions: [] }),
}));

const cleanups: Array<() => void> = [];
afterEach(() => {
  cleanups.splice(0).reverse().forEach((cleanup) => cleanup());
  document.body.replaceChildren();
});

async function openMenu(
  tabCount: number,
  targetIndex: number,
  options: { tabs?: WorkbenchEditorInput[]; workspaceReveal?: boolean } = {},
) {
  const tabs = options.tabs ?? Array.from({ length: tabCount }, (_, index) => createWorkbenchEditorInput(
    { kind: "newSession", projectId: `project-${index}` }, `Tab ${index}`,
  ));
  const group: WorkbenchEditorGroup = {
    paneId: "pane", tabs, activeEditorId: tabs[0]!.editorId,
  };
  const activate = vi.fn();
  const closeMany = vi.fn();
  const revealInWorkspace = vi.fn();
  const openInFileExplorer = vi.fn();
  const host = document.createElement("div");
  document.body.append(host);
  const app = createApp({ setup() {
    const controller = provideInternalDragController();
    cleanups.push(() => controller.dispose());
    return () => h(WorkbenchEditorTabs, {
      windowId: "main", group, showSingleTab: true, workspaceReveal: options.workspaceReveal,
      onActivate: activate, "onClose-many": closeMany,
      "onReveal-in-workspace": revealInWorkspace, "onOpen-in-file-explorer": openInFileExplorer,
    });
  } });
  app.mount(host);
  cleanups.push(() => app.unmount());
  await nextTick();
  host.querySelector(`[data-workbench-tab-id="${tabs[targetIndex]!.editorId}"]`)!
    .dispatchEvent(new MouseEvent("contextmenu", { bubbles: true, cancelable: true, clientX: 100, clientY: 30 }));
  await nextTick();
  const menu = document.querySelector('[role="menu"]')!;
  const button = (label: string) => Array.from(menu.querySelectorAll<HTMLButtonElement>("button"))
    .find((candidate) => candidate.textContent?.trim() === label);
  const others = button("workbench.tabs.closeOthers")!;
  return { tabs, menu, others, button, activate, closeMany, revealInWorkspace, openInFileExplorer };
}

describe("workbench tab context menu", () => {
  it("keeps and activates the context tab before closing other tabs", async () => {
    const { tabs, others, activate, closeMany } = await openMenu(3, 1);
    expect(others.disabled).toBe(false);
    others.click();
    await nextTick();
    expect(activate).toHaveBeenCalledExactlyOnceWith(tabs[1]!.editorId);
    expect(closeMany).toHaveBeenCalledExactlyOnceWith([tabs[0]!.editorId, tabs[2]!.editorId]);
    expect(activate.mock.invocationCallOrder[0]).toBeLessThan(closeMany.mock.invocationCallOrder[0]!);
    expect(document.querySelector('[role="menu"]')).toBeNull();
  });

  it("disables Close Others when there is only one tab", async () => {
    const { others, activate, closeMany } = await openMenu(1, 0);
    expect(others.disabled).toBe(true);
    others.click();
    expect(activate).not.toHaveBeenCalled();
    expect(closeMany).not.toHaveBeenCalled();
  });

  it("offers workspace reveal for session tabs and emits the context tab", async () => {
    const tabs = [
      createWorkbenchEditorInput({ kind: "newSession", projectId: "project" }, "New"),
      createWorkbenchEditorInput({ kind: "session", projectId: "project", sessionId: "s1" }, "Session"),
    ];
    const { button, revealInWorkspace, openInFileExplorer } = await openMenu(0, 1, { tabs, workspaceReveal: true });
    expect(button("workbench.tabs.openInFileBrowser")).toBeUndefined();
    button("workbench.tabs.revealInWorkspace")!.click();
    await nextTick();
    expect(revealInWorkspace).toHaveBeenCalledExactlyOnceWith(tabs[1]!.editorId);
    expect(openInFileExplorer).not.toHaveBeenCalled();
    expect(document.querySelector('[role="menu"]')).toBeNull();
  });

  it("offers both actions for workspace files and opens the file browser", async () => {
    const tabs = [createWorkbenchEditorInput({ kind: "workspaceFile", projectId: "project", path: "docs/01_I.md" }, "01_I.md")];
    const { button, revealInWorkspace, openInFileExplorer } = await openMenu(0, 0, { tabs, workspaceReveal: true });
    expect(button("workbench.tabs.revealInWorkspace")).toBeDefined();
    button("workbench.tabs.openInFileBrowser")!.click();
    await nextTick();
    expect(openInFileExplorer).toHaveBeenCalledExactlyOnceWith(tabs[0]!.editorId);
    expect(revealInWorkspace).not.toHaveBeenCalled();
  });

  it("hides workspace reveal without a workspace tree or for files outside the workspace", async () => {
    const session = [createWorkbenchEditorInput({ kind: "session", projectId: "project", sessionId: "s1" }, "Session")];
    const withoutTree = await openMenu(0, 0, { tabs: session });
    expect(withoutTree.button("workbench.tabs.revealInWorkspace")).toBeUndefined();
    expect(withoutTree.menu.querySelectorAll('[role="separator"]')).toHaveLength(1);
    cleanups.splice(0).reverse().forEach((cleanup) => cleanup());
    document.body.replaceChildren();

    const external = [createWorkbenchEditorInput({ kind: "workspaceFile", projectId: "project", path: "D:/notes/a.md" }, "a.md")];
    const outside = await openMenu(0, 0, { tabs: external, workspaceReveal: true });
    expect(outside.button("workbench.tabs.revealInWorkspace")).toBeUndefined();
    expect(outside.button("workbench.tabs.openInFileBrowser")).toBeDefined();
  });
});
