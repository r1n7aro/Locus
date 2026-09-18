// @vitest-environment jsdom
import { createApp, nextTick, type App, type Component } from "vue";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { t } from "../i18n";
import ResourceFileMenuItems from "../components/explorer/ResourceFileMenuItems.vue";
import ExplorerResourceActions from "../components/explorer/ExplorerResourceActions.vue";
import AssetDirectoryList from "../components/asset/AssetDirectoryList.vue";
import KnowledgeExplorer from "../components/knowledge/KnowledgeExplorer.vue";
import { explorerFileKey, explorerKnowledgeKey, useExplorerPathDisplay } from "../composables/useExplorerPathDisplay";
import { resourceRelativePath, type ExplorerResourceTarget } from "../components/explorer/explorerResourceActions";
import type { KnowledgeDocumentSummary } from "../types";

const mocks = vi.hoisted(() => ({
  fileAction: vi.fn(), knowledgeMove: vi.fn(), knowledgeDelete: vi.fn(), confirm: vi.fn(), notice: vi.fn(),
  refresh: vi.fn(), load: vi.fn(), copy: vi.fn(), loadMount: vi.fn(),
  snapshots: {} as Record<string, { nodes: { nodeId: string; nodeKind: string; sourcePath: string }[] }>,
}));
vi.mock("@tauri-apps/plugin-dialog", () => ({ confirm: mocks.confirm, save: vi.fn() }));
vi.mock("../stores/notification", () => ({ useNotificationStore: () => ({ addNotice: mocks.notice }) }));
vi.mock("../stores/workspaceExplorer", () => ({ useWorkspaceExplorerStore: () => ({
  refreshProjectKnowledge: mocks.refresh, loadProject: mocks.load, snapshots: mocks.snapshots, loadMount: mocks.loadMount,
}) }));
vi.mock("../services/workspaceExplorer", () => ({ explorerFileAction: mocks.fileAction }));
vi.mock("../services/knowledge", () => ({ knowledgeMove: mocks.knowledgeMove, knowledgeDelete: mocks.knowledgeDelete,
  deleteSkillPackage: vi.fn(), exportSkillPackage: vi.fn(), knowledgeRevealTarget: vi.fn(),
}));
vi.mock("../composables/useKnowledgeState", () => ({
  isSkillPackageRootDocument: (document: KnowledgeDocumentSummary) => document.externalSource?.provider === "package" && document.path.endsWith("/SKILL.md"),
  skillPackageIdForDocument: (document: KnowledgeDocumentSummary) => document.externalSource?.sourceId,
}));

const workspaceRef = { checkoutId: "checkout-a", expectedGeneration: 7, expectedMaterializationEpoch: 2 };
const file: ExplorerResourceTarget = { projectId: "project-a", root: "F:/Project", path: "F:/Project/docs/design.md", workspaceRef };
const document: KnowledgeDocumentSummary = {
  id: "doc-a", type: "design", path: "combat/design.md", title: "Combat", injectMode: "excerpt",
  effectiveInjectMode: "excerpt", readOnly: false, aiMaintained: false, effectiveAiMaintained: false, modifiedAt: 1,
};
const knowledge: ExplorerResourceTarget = { ...file, path: document.path, document };
const apps: App[] = [];
function mount(component: Component, props: Record<string, unknown> = {}) {
  const host = window.document.createElement("div");
  window.document.body.appendChild(host);
  const app = createApp(component, props);
  const instance = app.mount(host);
  apps.push(app);
  return { host, instance: instance as unknown as InstanceType<typeof ExplorerResourceActions> };
}
async function flush() { for (let i = 0; i < 10; i++) await nextTick(); }
function button(host: Element, label: string) {
  const found = [...host.querySelectorAll<HTMLButtonElement>("button")].find((item) => item.textContent?.trim() === label);
  expect(found, label).toBeDefined();
  return found!;
}
function submit(name: string) {
  const input = window.document.querySelector<HTMLInputElement>(".resource-rename-dialog input")!;
  input.value = name;
  input.dispatchEvent(new Event("input", { bubbles: true }));
  input.form!.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true }));
}
beforeEach(() => {
  vi.clearAllMocks();
  delete mocks.snapshots["project-a"];
  localStorage.removeItem("locus:explorerFullPaths");
  window.dispatchEvent(new StorageEvent("storage", { key: "locus:explorerFullPaths" }));
  Object.defineProperty(navigator, "clipboard", { configurable: true, value: { writeText: mocks.copy } });
  mocks.fileAction.mockResolvedValue("F:/Project/docs/renamed.md");
  mocks.knowledgeMove.mockResolvedValue({});
});
afterEach(() => { apps.splice(0).forEach((app) => app.unmount()); window.document.body.innerHTML = ""; });

describe("explorer resource menus", () => {
  it("changes only the rendered file label, preserving the file used by list actions", async () => {
    const selected = vi.fn();
    const nodes = ["design.md", "other.md"].map((name) => ({ kind: "file", path: `docs/${name}`, name, depth: 1 }));
    const list = mount(AssetDirectoryList, { workingDir: file.root, items: nodes, loading: false, loaded: true, hasMore: false, emptyLabel: "", selectedPath: null, onSelect: selected });
    useExplorerPathDisplay().togglePath(explorerFileKey(file.path));
    await flush();
    const rows = list.host.querySelectorAll<HTMLButtonElement>(".adl-row");
    expect(rows[0]!.textContent).toContain(file.path);
    expect(rows[1]!.textContent).not.toContain("F:/Project");
    rows[0]!.click();
    expect(selected).toHaveBeenCalledWith(nodes[0]);
    expect(nodes[0]!.name).toBe("design.md");
  });

  it("renames through the knowledge list menu even when that document displays its full path", async () => {
    const renamed = vi.fn();
    useExplorerPathDisplay().togglePath(explorerKnowledgeKey(file.root, document.id));
    const list = mount(KnowledgeExplorer, {
      workingDir: file.root, tree: [{ kind: "document", type: "design", path: "design/combat/design.md", name: "design.md", depth: 0, document }],
      activeType: "design", rootDirectoryConfigs: {}, externalDirectorySources: {}, folderStats: {}, selectedPath: null,
      isPathExpanded: () => true, rootContentsLoaded: () => true, hasMoreRootDocuments: () => false, rootDocumentsLoading: () => false,
      hasMoreFolderDocuments: () => false, folderDocumentsLoaded: () => true, folderDocumentsLoading: () => false,
      loading: false, searchQuery: "", searchResults: [], searching: false, onRenameDocument: renamed,
    });
    await flush();
    const row = list.host.querySelector<HTMLElement>(".workspace-tree-row-shell")!;
    expect(row.textContent).toContain("design/combat/design.md");
    row.dispatchEvent(new MouseEvent("contextmenu", { bubbles: true, cancelable: true }));
    await flush();
    button(window.document.body, t("knowledge.explorer.rename")).click();
    await flush();
    const input = list.host.querySelector<HTMLInputElement>(".kx-rename-input")!;
    expect(input.value).toBe("design.md");
    input.value = "renamed.md";
    input.dispatchEvent(new Event("input", { bubbles: true }));
    input.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true }));
    await flush();
    expect(renamed).toHaveBeenCalledWith("combat/design.md", "renamed.md", "design");
  });

  it("offers the same actions for ordinary files and knowledge documents", () => {
    const fileMenu = mount(ResourceFileMenuItems, { target: file });
    const knowledgeMenu = mount(ResourceFileMenuItems, { target: knowledge });
    const labels = (host: Element) => [...host.querySelectorAll("button")].map((item) => item.textContent?.trim());
    expect(labels(fileMenu.host)).toEqual(labels(knowledgeMenu.host));
    for (const label of ["knowledge.explorer.rename", "knowledge.explorer.copyRelativePath", "knowledge.explorer.openInFileSystem", "knowledge.explorer.delete"])
      expect(button(fileMenu.host, t(label)).disabled).toBe(false);
  });

  it("toggles only the selected document and persists across list instances", async () => {
    const menu = mount(ResourceFileMenuItems, { target: knowledge });
    button(menu.host, t("development.showFullPath")).click();
    await flush();
    const { showsFullPath } = useExplorerPathDisplay();
    expect(showsFullPath(explorerKnowledgeKey(file.root, document.id))).toBe(true);
    expect(showsFullPath(explorerKnowledgeKey(file.root, "doc-b"))).toBe(false);
    expect(showsFullPath(explorerFileKey(file.path))).toBe(false);
    window.dispatchEvent(new StorageEvent("storage", { key: "locus:explorerFullPaths" }));
    const secondMenu = mount(ResourceFileMenuItems, { target: knowledge });
    button(secondMenu.host, t("development.showFileName")).click();
    await flush();
    expect(showsFullPath(explorerKnowledgeKey(file.root, document.id))).toBe(false);
  });

  it.each(["plugin://sample", "external://sample"])("retains managed-source restrictions for %s", (locator) => {
    const target = { ...knowledge, document: { ...document, externalSource: { provider: "package", locator } } };
    const menu = mount(ResourceFileMenuItems, { target });
    expect(button(menu.host, t("knowledge.explorer.rename")).disabled).toBe(true);
    expect(button(menu.host, t("knowledge.explorer.delete")).disabled).toBe(true);
    expect(button(menu.host, t("knowledge.explorer.copyRelativePath")).disabled).toBe(false);
  });

  it("renames in the captured checkout, retaining path display preference", async () => {
    const changed = vi.fn();
    useExplorerPathDisplay().togglePath(explorerFileKey(file.path));
    const actions = mount(ExplorerResourceActions, { onChanged: changed });
    await actions.instance.run("rename", file);
    const input = window.document.querySelector<HTMLInputElement>(".resource-rename-dialog input")!;
    expect(input.value).toBe("design.md");
    expect(input.selectionEnd).toBe("design".length);
    submit("renamed.md");
    await flush();
    expect(mocks.fileAction).toHaveBeenCalledWith("project-a", file.path, "renamed.md", workspaceRef);
    expect(changed).toHaveBeenCalledWith(file, "F:/Project/docs/renamed.md");
    expect(useExplorerPathDisplay().showsFullPath(explorerFileKey("F:/Project/docs/renamed.md"))).toBe(true);
    expect(useExplorerPathDisplay().showsFullPath(explorerFileKey(file.path))).toBe(false);
    expect(window.document.querySelector("[role=dialog]")).toBeNull();
  });

  it("keeps knowledge identity and parent directory when renaming", async () => {
    const actions = mount(ExplorerResourceActions);
    await actions.instance.run("rename", knowledge);
    submit("renamed.md");
    await flush();
    expect(mocks.knowledgeMove).toHaveBeenCalledWith({ kind: "document", type: "design", path: "combat/design.md", newPath: "combat/renamed.md" }, workspaceRef);
    expect(mocks.fileAction).not.toHaveBeenCalled();
    expect(mocks.refresh).toHaveBeenCalledWith(file.projectId);
  });

  it("forces mounted lists to refresh and closes a successful rename even if refresh fails", async () => {
    mocks.snapshots["project-a"] = { nodes: [{ nodeId: "mount", nodeKind: "folder", sourcePath: "F:/Project/docs" }] };
    mocks.loadMount.mockRejectedValueOnce(new Error("Listing unavailable"));
    const actions = mount(ExplorerResourceActions);
    await actions.instance.run("rename", file);
    submit("renamed.md");
    await flush();
    expect(mocks.loadMount).toHaveBeenCalledWith("project-a", "mount", true);
    expect(mocks.fileAction).toHaveBeenCalledTimes(1);
    expect(window.document.querySelector("[role=dialog]")).toBeNull();
    expect(mocks.notice).toHaveBeenCalledWith("warning", "Listing unavailable");
  });

  it("rejects path traversal and keeps a failed rename available for correction", async () => {
    const actions = mount(ExplorerResourceActions);
    await actions.instance.run("rename", file);
    submit("../oops.md");
    await flush();
    expect(mocks.fileAction).not.toHaveBeenCalled();
    expect(window.document.querySelector("[role=alert]")?.textContent).toContain(t("development.invalidFileName"));
    mocks.fileAction.mockRejectedValueOnce(new Error("Target already exists"));
    submit("duplicate.md");
    await flush();
    expect(window.document.querySelector("[role=alert]")?.textContent).toContain("Target already exists");
    expect(window.document.querySelector<HTMLInputElement>(".resource-rename-dialog input")?.value).toBe("duplicate.md");
  });

  it("does not delete when confirmation is cancelled", async () => {
    mocks.confirm.mockResolvedValue(false);
    const actions = mount(ExplorerResourceActions);
    await actions.instance.run("delete", file);
    expect(mocks.fileAction).not.toHaveBeenCalled();
    expect(mocks.knowledgeDelete).not.toHaveBeenCalled();
  });

  it("copies scoped paths and preserves external mount paths", async () => {
    const actions = mount(ExplorerResourceActions);
    await actions.instance.run("copy", knowledge);
    expect(mocks.copy).toHaveBeenCalledWith("design/combat/design.md");
    expect(resourceRelativePath(file)).toBe("docs/design.md");
    expect(resourceRelativePath({ ...file, path: "F:/Project-other/design.md" })).toBe("F:/Project-other/design.md");
  });
});
