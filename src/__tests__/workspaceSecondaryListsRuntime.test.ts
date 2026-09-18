// @vitest-environment jsdom
import { createPinia } from "pinia";
import { computed, createApp, defineComponent, h, nextTick, ref, type App } from "vue";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import KnowledgeView from "../components/KnowledgeView.vue";
import AssetView from "../components/AssetView.vue";
import WorkbenchArchivedSessionsEditor from "../components/workbench/WorkbenchArchivedSessionsEditor.vue";
import type { KnowledgeDocumentSummary } from "../types";
import type { SessionSummary } from "../types";
import type { WorkspaceTreeItem } from "../components/explorer/WorkspaceTree.vue";

const mocks = vi.hoisted(() => ({
  knowledgeState: null as any,
  assetState: null as any,
  selectDocument: vi.fn(),
  selectNode: vi.fn(),
  listArchived: vi.fn(),
  loadSession: vi.fn(),
  storage: vi.fn(),
}));
vi.mock("../composables/useKnowledgeState", () => ({ useKnowledgeState: () => mocks.knowledgeState }));
vi.mock("../composables/useAssetState", () => ({ useAssetState: () => mocks.assetState }));
vi.mock("../services/session", async (original) => ({
  ...await original<typeof import("../services/session")>(),
  listArchivedCheckoutSessions: mocks.listArchived,
  loadSession: mocks.loadSession,
  getArchivedCheckoutStorageBytes: mocks.storage,
}));
vi.mock("../components/knowledge/KnowledgeExplorer.vue", () => ({
  default: defineComponent({
    emits: ["selectDocument", "selectPackage", "selectSearchResult", "selectTypeRoot", "selectFolderConfig"],
    setup(_, { emit }) {
      return () => h("div", { class: "knowledge-explorer" }, [
        h("button", { class: "open-doc", onClick: () => emit("selectDocument", mocks.knowledgeState.documents.value[0]) }, "doc"),
        h("button", { class: "open-package", onClick: () => emit("selectPackage", mocks.knowledgeState.documents.value[0]) }, "package"),
        h("button", { class: "open-search", onClick: () => emit("selectSearchResult", {}) }, "search"),
        h("button", { class: "open-config", onClick: () => emit("selectFolderConfig", "design", "combat") }, "config"),
      ]);
    },
  }),
}));
vi.mock("../components/asset/AssetLegacyExplorer.vue", () => ({
  default: defineComponent({
    emits: ["select"],
    setup(_, { emit }) {
      return () => h("button", { class: "open-file", onClick: () => emit("select", { kind: "file", path: "Assets/Test.cs", name: "Test.cs" }) }, "Test.cs");
    },
  }),
}));
vi.mock("../components/chat/ChatTranscript.vue", () => ({ default: defineComponent(() => () => h("div", { class: "test-transcript" })) }));

const workspaceRef = { checkoutId: "c", expectedGeneration: 1, expectedMaterializationEpoch: 2 };
const doc: KnowledgeDocumentSummary = {
  id: "doc-a", type: "design", path: "combat.md", title: "Combat", injectMode: "excerpt",
  effectiveInjectMode: "excerpt", readOnly: false, aiMaintained: false, effectiveAiMaintained: false, modifiedAt: 1,
};
const mounted: App[] = [];
function mount(component: any, props: Record<string, unknown>) {
  const host = document.createElement("div");
  document.body.appendChild(host);
  const app = createApp(component, props);
  app.use(createPinia());
  app.mount(host);
  mounted.push(app);
  return host;
}
async function flush() { for (let i = 0; i < 8; i++) await nextTick(); }

beforeEach(() => {
  vi.clearAllMocks();
  mocks.knowledgeState = {
    sidebarWidth: ref(272), documents: ref([doc]), activeType: ref("design"), selectedPath: ref(null),
    selectedDocument: ref(null), selectedPackageDocument: ref(null), selectedDocumentLoading: ref(false),
    selectedDirectoryConfig: ref(null), selectedDirectoryLoading: ref(false),
    visibleExplorerTree: ref([]), rootDirectoryConfigs: ref({}), loading: ref(false),
    searchQuery: ref(""), searchResults: ref([]), searching: ref(false), embeddingStatus: ref(null),
    selectDocument: mocks.selectDocument, endExplorerDrag: vi.fn(),
    clearSearch: vi.fn(), clearSelection: vi.fn(), refreshRetrievalState: vi.fn(),
    selectSearchResult: vi.fn(async () => { mocks.knowledgeState.selectedDocument.value = doc; }),
  };
  mocks.assetState = {
    error: ref(""), sidebarWidth: ref(240), directoryPaneWidth: ref(320), explorerTree: ref([]),
    selectedNode: ref(null), selectedFolderPath: ref("."), previewNode: ref(null), viewMode: ref("stats"),
    searchQuery: ref(""), searchScope: ref("global"), searching: ref(false),
    updateSearchScope: vi.fn(), selectNode: mocks.selectNode,
  };
  mocks.listArchived.mockResolvedValue([{ id: "archived-a", title: "Archived A", updatedAt: 1 }]);
  mocks.loadSession.mockResolvedValue({ id: "archived-a", title: "Archived A", messages: [] });
  mocks.storage.mockResolvedValue(2048);
  vi.stubGlobal("ResizeObserver", class { observe() {} disconnect() {} });
  vi.stubGlobal("requestAnimationFrame", (callback: FrameRequestCallback) => setTimeout(callback, 0));
  vi.stubGlobal("cancelAnimationFrame", (id: number) => clearTimeout(id));
});
afterEach(() => { mounted.splice(0).forEach((app) => app.unmount()); document.body.innerHTML = ""; });

describe("workspace secondary lists", () => {
  it("routes retrieval, injection and directory settings to the workbench without local details", () => {
    const open = vi.fn();
    const host = mount(KnowledgeView, { workingDir: "C:/project", workspaceRef, selectedModelId: "", modelDefaults: {}, listOnly: true, onOpenPage: open });
    const tools = host.querySelectorAll<HTMLButtonElement>(".kx-side-tool");
    tools[0]!.click();
    tools[1]!.click();
    host.querySelector<HTMLButtonElement>(".open-config")!.click();
    expect(open.mock.calls.map(([page]) => page)).toEqual([{ kind: "retrieval" }, { kind: "injection" }, { kind: "directory", type: "design", path: "combat" }]);
    expect(host.querySelector(".kx-right")).toBeNull();
    expect(document.querySelector('[role="dialog"]')).toBeNull();
    expect(mocks.knowledgeState.refreshRetrievalState).not.toHaveBeenCalled();
  });
  it("opens knowledge documents, packages and search hits without mounting a detail panel", async () => {
    const open = vi.fn();
    const host = mount(KnowledgeView, { workingDir: "C:/project", workspaceRef, selectedModelId: "", modelDefaults: {}, listOnly: true, onOpenDocument: open });
    expect(host.querySelector(".kx-right")).toBeNull();
    expect(host.querySelector(".resize-handle")).toBeNull();
    host.querySelector<HTMLButtonElement>(".open-doc")!.click();
    host.querySelector<HTMLButtonElement>(".open-package")!.click();
    expect(open).toHaveBeenNthCalledWith(1, doc);
    expect(open).toHaveBeenNthCalledWith(2, doc);
    expect(mocks.selectDocument).not.toHaveBeenCalled();
    host.querySelector<HTMLButtonElement>(".open-search")!.click();
    await flush();
    expect(open).toHaveBeenNthCalledWith(3, doc);
    expect(host.querySelector(".kx-right")).toBeNull();
  });

  it("opens files from the compact tree without loading the old preview", () => {
    const open = vi.fn();
    const host = mount(AssetView, { workingDir: "C:/project", workspaceRef, listOnly: true, onOpenFile: open });
    host.querySelector<HTMLButtonElement>(".open-file")!.click();
    expect(open).toHaveBeenCalledWith({ path: "Assets/Test.cs", name: "Test.cs" });
    expect(mocks.selectNode).not.toHaveBeenCalled();
    expect(host.querySelector(".ax-pane-preview")).toBeNull();
  });

});

function mountArchive() {
  const target = document.createElement("div");
  document.body.appendChild(target);
  const scope = ref<typeof workspaceRef | null>(workspaceRef);
  const sessions = ref<SessionSummary[]>([]);
  const activate = vi.fn();
  const contextmenu = vi.fn();
  const restore = vi.fn();
  const items = computed<WorkspaceTreeItem[]>(() => sessions.value.map((session) => ({
    key: session.id, treeRow: {
      key: session.id, name: session.title, depth: 0, kind: "file",
      session: { branch: "feature", updatedTime: "2m", unread: false, animated: false, pending: false, statusLabel: "", archived: true },
    },
  })));
  const host = mount(defineComponent(() => () => h(WorkbenchArchivedSessionsEditor, {
    projectId: "p", workspaceRef: scope.value, items: items.value, toolbarTarget: target,
    onSessionsChange: (value: SessionSummary[]) => { sessions.value = value; },
    onActivate: activate, onContextmenu: contextmenu, onSessionAction: restore,
  })), {});
  return { host, target, scope, activate, contextmenu, restore };
}

describe("archived session list", () => {
  it("allows opening and restoring sessions while storage is still pending", async () => {
    let finish!: (bytes: number) => void;
    mocks.storage.mockReturnValue(new Promise<number>((resolve) => { finish = resolve; }));
    const { host, target, activate, contextmenu, restore } = mountArchive();
    await flush();
    const row = host.querySelector<HTMLButtonElement>(".workspace-tree-row")!;
    expect(row.textContent).toContain("Archived A");
    expect(host.querySelector(".development-session-meta")?.textContent).toContain("feature");
    row.dispatchEvent(new MouseEvent("click", { bubbles: true, ctrlKey: true }));
    expect(activate).toHaveBeenCalledWith(expect.objectContaining({ key: "archived-a" }), expect.objectContaining({ ctrlKey: true }));
    row.dispatchEvent(new MouseEvent("contextmenu", { bubbles: true }));
    expect(contextmenu).toHaveBeenCalled();
    host.querySelector<HTMLButtonElement>(".development-session-archive-button")!.click();
    expect(restore).toHaveBeenCalledWith(expect.objectContaining({ key: "archived-a" }));
    expect(mocks.loadSession).not.toHaveBeenCalled();
    expect(target.textContent).toContain("1");
    finish(2048);
    await flush();
    expect(target.textContent).toContain("2.00 KB");
  });

  it("does not lose the list when storage fails and can retry independently", async () => {
    mocks.storage.mockRejectedValueOnce(new Error("Disk busy"));
    const { host, target } = mountArchive();
    await flush();
    expect(host.textContent).toContain("Archived A");
    expect(target.querySelector('[role="status"]')?.textContent).toBe("—");
    target.querySelector<HTMLButtonElement>("button")!.click();
    await flush();
    expect(host.textContent).toContain("Archived A");
    expect(target.textContent).toContain("2.00 KB");
  });

  it("discards old workspace results after clearing or switching scope", async () => {
    let finishList!: (value: SessionSummary[]) => void;
    let finishStorage!: (value: number) => void;
    mocks.listArchived.mockReturnValueOnce(new Promise<SessionSummary[]>((resolve) => { finishList = resolve; }));
    mocks.storage.mockReturnValueOnce(new Promise<number>((resolve) => { finishStorage = resolve; }));
    const { host, target, scope } = mountArchive();
    await flush();
    scope.value = null;
    await flush();
    finishList([{ id: "old", title: "Old workspace" } as SessionSummary]);
    finishStorage(999999);
    await flush();
    expect(host.textContent).not.toContain("Old workspace");
    expect(target.textContent).not.toContain("KB");
    expect(target.querySelector(".secondary-sidebar-count")?.textContent).toBe("0");
    scope.value = { ...workspaceRef, checkoutId: "other" };
    await flush();
    expect(host.textContent).toContain("Archived A");
    expect(target.textContent).toContain("2.00 KB");
  });
});
