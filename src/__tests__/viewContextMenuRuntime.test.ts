// @vitest-environment jsdom
import { createPinia } from "pinia";
import { createApp, h, nextTick } from "vue";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import ViewPackageView from "../components/ViewPackageView.vue";
import { t } from "../i18n";
import { viewDeleteEntry, viewTree, type ViewPackageSummary } from "../services/view";
import { useNotificationStore } from "../stores/notification";

vi.mock("../services/view", async (importOriginal) => ({
  ...await importOriginal<typeof import("../services/view")>(),
  viewTree: vi.fn(),
  viewDeleteEntry: vi.fn(),
}));
vi.mock("../services/locusRuntime", () => ({
  getLocusRuntime: () => ({ subscribe: vi.fn(async () => () => {}) }),
}));
vi.mock("../components/view/viewCompilation", () => ({ compileViewPackage: vi.fn() }));
vi.mock("../composables/useInternalDrag", () => ({
  useInternalDragController: () => ({
    registerTarget: () => () => {},
    isDraggingType: () => false,
  }),
}));

const workspaceRef = { checkoutId: "workspace", expectedGeneration: 2 };
const cleanups: Array<() => void> = [];

function view(overrides: Partial<ViewPackageSummary> = {}): ViewPackageSummary {
  return {
    id: "panel", name: "Panel", version: "1.0.0", apiVersion: "1",
    displayPath: "Tools/panel", packageRelPath: "panel",
    packageRoot: "F:/workspace/Locus/View/panel", manifestPath: "F:/workspace/Locus/View/panel/view.json",
    updatedAt: 1, capabilities: { unity: false }, requirements: { unityConnection: false }, source: "project",
    ...overrides,
  };
}

async function flush() {
  for (let index = 0; index < 8; index++) await nextTick();
}

async function mountList(summary: ViewPackageSummary) {
  vi.mocked(viewTree).mockResolvedValue({ views: [summary], folders: [], order: [] });
  const host = document.createElement("div");
  document.body.append(host);
  const app = createApp({ render: () => h(ViewPackageView, {
    workingDir: "F:/workspace", workspaceRef, listOnly: true,
  }) });
  const pinia = createPinia();
  app.use(pinia);
  app.mount(host);
  cleanups.push(() => { app.unmount(); host.remove(); });
  await flush();
  return { host, notices: useNotificationStore(pinia) };
}

async function openMenu(host: HTMLElement, kind = "view") {
  const row = host.querySelector<HTMLElement>(`[data-view-node-kind="${kind}"]`);
  expect(row).not.toBeNull();
  row!.dispatchEvent(new MouseEvent("contextmenu", { bubbles: true, clientX: 100, clientY: 100 }));
  await flush();
  return document.querySelector<HTMLElement>(".view-ctx-menu")!;
}

function button(container: ParentNode, label: string) {
  return [...container.querySelectorAll<HTMLButtonElement>("button")]
    .find((element) => element.textContent?.trim() === label);
}

beforeEach(() => {
  vi.clearAllMocks();
  localStorage.clear();
  vi.mocked(viewDeleteEntry).mockResolvedValue({ views: [], folders: [], order: [] });
});

afterEach(() => {
  cleanups.splice(0).reverse().forEach((cleanup) => cleanup());
  document.body.innerHTML = "";
});

describe("View list context menu", () => {
  it("deletes a local View only after confirmation and refreshes the list", async () => {
    const { host } = await mountList(view());
    const menu = await openMenu(host);
    expect(menu.querySelector(".base-context-menu-label")).toBeNull();
    button(menu, t("view.tree.delete"))!.click();
    await flush();
    expect(viewDeleteEntry).not.toHaveBeenCalled();
    const confirm = document.querySelector(".view-delete-confirm")!;
    expect(confirm.textContent).toContain("Panel");
    button(confirm, t("common.confirm"))!.click();
    await flush();
    expect(viewDeleteEntry).toHaveBeenCalledExactlyOnceWith(workspaceRef, { relPath: "Tools/panel" });
    expect(host.querySelector('[data-view-node-kind="view"]')).toBeNull();
    expect(document.querySelector(".view-delete-confirm")).toBeNull();
  });

  it("leaves the View intact when deletion is cancelled or fails", async () => {
    const { host, notices } = await mountList(view());
    button(await openMenu(host), t("view.tree.delete"))!.click();
    await flush();
    button(document.querySelector(".view-delete-confirm")!, t("common.cancel"))!.click();
    await flush();
    expect(viewDeleteEntry).not.toHaveBeenCalled();
    expect(host.querySelector('[data-view-node-kind="view"]')).not.toBeNull();

    vi.mocked(viewDeleteEntry).mockRejectedValueOnce(new Error("Delete failed"));
    const notice = vi.spyOn(notices, "addNotice").mockReturnValue("notice");
    button(await openMenu(host), t("view.tree.delete"))!.click();
    await flush();
    button(document.querySelector(".view-delete-confirm")!, t("common.confirm"))!.click();
    await flush();
    expect(notice).toHaveBeenCalledWith("error", "Delete failed", expect.any(Object));
    expect(host.querySelector('[data-view-node-kind="view"]')).not.toBeNull();
  });

  it.each(["pluginProject", "pluginApp"])("attributes %s Views and protects them and their parent folders", async (source) => {
    const { host } = await mountList(view({ source, pluginId: "com.example.tools", pluginName: "Example Tools" }));
    const menu = await openMenu(host);
    expect(menu.querySelector(".base-context-menu-label")?.textContent?.trim())
      .toBe(t("view.tree.pluginProvidedBy", "Example Tools"));
    expect(button(menu, t("view.tree.delete"))).toBeUndefined();
    expect(button(menu, t("view.tree.rename"))).toBeUndefined();
    expect(button(menu, t("view.action.export"))).toBeUndefined();
    const folderMenu = await openMenu(host, "folder");
    expect(button(folderMenu, t("view.tree.delete"))).toBeUndefined();
    expect(viewDeleteEntry).not.toHaveBeenCalled();
  });

  it("uses the plugin ID if a display name is unavailable", async () => {
    const { host } = await mountList(view({ source: "pluginProject", pluginId: "com.example.tools" }));
    expect((await openMenu(host)).textContent).toContain(t("view.tree.pluginProvidedBy", "com.example.tools"));
  });
});
