// @vitest-environment jsdom
import { createPinia, type Pinia } from "pinia";
import {
  createApp,
  defineComponent,
  h,
  nextTick,
  reactive,
  ref,
  type App,
  type Ref,
} from "vue";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { WorkspaceRef } from "../services/project";

const mocks = vi.hoisted(() => ({
  searchWorkspaceAssets: vi.fn(),
  searchWorkspaceSceneObjects: vi.fn(),
  listDirEntriesPage: vi.fn(),
  searchWorkspaceEntries: vi.fn(),
  knowledgeQuery: vi.fn(),
  checkUnityConnectionStatus: vi.fn(),
  getUnityConsoleText: vi.fn(),
  validateUnitySceneObject: vi.fn(),
  getCachedFileToolWorkspaceBoundary: vi.fn(),
  getFileToolWorkspaceBoundary: vi.fn(),
  addNotice: vi.fn(),
  removeNotice: vi.fn(),
  garbageCollection: vi.fn(),
  assetDropHandler: null as null | ((payload: Record<string, unknown>) => void),
  fileDropHandler: null as null | ((payload: Record<string, unknown>) => void),
  textDropHandler: null as null | ((payload: Record<string, unknown>) => void),
}));

vi.mock("../services/asset", () => ({
  searchWorkspaceAssets: mocks.searchWorkspaceAssets,
  searchWorkspaceSceneObjects: mocks.searchWorkspaceSceneObjects,
}));

vi.mock("../services/project", async (importOriginal) => ({
  ...await importOriginal<typeof import("../services/project")>(),
  listDirEntriesPage: mocks.listDirEntriesPage,
  searchWorkspaceEntries: mocks.searchWorkspaceEntries,
}));

vi.mock("../services/knowledge", () => ({
  knowledgeQuery: mocks.knowledgeQuery,
}));

vi.mock("../services/permissions", () => ({
  getCachedFileToolWorkspaceBoundary: mocks.getCachedFileToolWorkspaceBoundary,
  getFileToolWorkspaceBoundary: mocks.getFileToolWorkspaceBoundary,
}));

vi.mock("../services/unity", () => ({
  checkUnityConnectionStatus: mocks.checkUnityConnectionStatus,
  classifyUnitySceneObjectError: () => "unknown",
  filterUnityConsoleErrorPayload: (payload: unknown) => payload,
  getUnityConsoleText: mocks.getUnityConsoleText,
  isUnityConsoleErrorLevel: (level: string) => level.toLowerCase().includes("error"),
  subscribeLocusFileDrop: vi.fn(async (handler: (payload: Record<string, unknown>) => void) => {
    mocks.fileDropHandler = handler;
    return () => {};
  }),
  subscribeLocusFileDragState: vi.fn(async () => () => {}),
  subscribeUnityEmbedAssetDrop: vi.fn(async (handler: (payload: Record<string, unknown>) => void) => {
    mocks.assetDropHandler = handler;
    return () => {};
  }),
  subscribeUnityEmbedTextDrop: vi.fn(async (handler: (payload: Record<string, unknown>) => void) => {
    mocks.textDropHandler = handler;
    return () => {};
  }),
  validateUnitySceneObject: mocks.validateUnitySceneObject,
}));

vi.mock("../stores/notification", () => ({
  useNotificationStore: () => ({ addNotice: mocks.addNotice, removeNotice: mocks.removeNotice }),
}));

vi.mock("../services/garbageCollection", () => ({ garbageCollection: mocks.garbageCollection }));

import RichChatInput from "../components/chat/RichChatInput.vue";
import { workbenchComposerTreeFileAttachment } from "../components/workbench/workbenchComposerDrop";
import { createWorkbenchEditorInput, useWorkbenchStore } from "../stores/workbench";
import { useWorkspaceExplorerStore } from "../stores/workspaceExplorer";
import { useWorkspaceContextStore } from "../stores/workspaceContext";

const CHECKOUT_A: WorkspaceRef = { checkoutId: "checkout-a", expectedGeneration: 3 };
const CHECKOUT_B: WorkspaceRef = { checkoutId: "checkout-b", expectedGeneration: 8 };

interface HarnessState {
  text: string;
  workspaceRef: WorkspaceRef | null;
  workspaceRoot: string;
  planModeActive: boolean;
}

interface MountedInput {
  app: App;
  pinia: Pinia;
  host: HTMLElement;
  state: HarnessState;
  requestPlanMode: ReturnType<typeof vi.fn>;
  requestNewSession: ReturnType<typeof vi.fn>;
  fork: ReturnType<typeof vi.fn>;
  undo: ReturnType<typeof vi.fn>;
  compact: ReturnType<typeof vi.fn>;
  exportContext: ReturnType<typeof vi.fn>;
  reviewContext: ReturnType<typeof vi.fn>;
  input: Ref<InstanceType<typeof RichChatInput> | null>;
  send: ReturnType<typeof vi.fn>;
}

const mountedInputs: MountedInput[] = [];

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason?: unknown) => void;
  const promise = new Promise<T>((resolvePromise, rejectPromise) => {
    resolve = resolvePromise;
    reject = rejectPromise;
  });
  return { promise, resolve, reject };
}

async function flushAsync() {
  await Promise.resolve();
  await nextTick();
  await Promise.resolve();
  await nextTick();
}

async function mountInput(options: Partial<HarnessState> & { managedNativeDrops?: boolean } = {}) {
  const state = reactive<HarnessState>({
    text: "",
    workspaceRef: CHECKOUT_A,
    workspaceRoot: "C:/projects/a",
    planModeActive: false,
    ...options,
  });
  const requestPlanMode = vi.fn();
  const requestNewSession = vi.fn();
  const fork = vi.fn();
  const undo = vi.fn();
  const compact = vi.fn();
  const exportContext = vi.fn();
  const reviewContext = vi.fn();
  const input = ref<InstanceType<typeof RichChatInput> | null>(null);
  const send = vi.fn();
  const Root = defineComponent({
    setup() {
      return () => h(RichChatInput, {
        ref: input,
        modelValue: state.text,
        "onUpdate:modelValue": (value: string) => { state.text = value; },
        selectedAgentId: "unity",
        workspaceRef: state.workspaceRef,
        workspaceRoot: state.workspaceRoot,
        planModeActive: state.planModeActive,
        managedNativeDrops: options.managedNativeDrops ?? true,
        onRequestPlanMode: requestPlanMode,
        onRequestNewSession: requestNewSession,
        onFork: fork,
        onUndo: undo,
        onCompact: compact,
        onExportContext: exportContext,
        onReviewContext: reviewContext,
        onSend: send,
      });
    },
  });
  const host = document.createElement("div");
  document.body.appendChild(host);
  const app = createApp(Root);
  const pinia = createPinia();
  app.use(pinia);
  app.mount(host);
  const mounted = {
    app,
    pinia,
    host,
    state,
    requestPlanMode,
    requestNewSession,
    fork,
    undo,
    compact,
    exportContext,
    reviewContext,
    input,
    send,
  };
  mountedInputs.push(mounted);
  await flushAsync();
  return mounted;
}

async function setComposerText(host: HTMLElement, value: string) {
  const textarea = host.querySelector<HTMLTextAreaElement>("textarea.chat-composer-input");
  if (!textarea) throw new Error("composer textarea was not mounted");
  textarea.value = value;
  textarea.setSelectionRange(value.length, value.length);
  textarea.dispatchEvent(new Event("input", { bubbles: true }));
  await nextTick();
  await nextTick();
  return textarea;
}

function clickCommand(host: HTMLElement, name: string) {
  const command = Array.from(host.querySelectorAll<HTMLButtonElement>("button.command-item"))
    .find((item) => item.querySelector(".command-name")?.textContent === name);
  if (!command) throw new Error(`command ${name} was not rendered`);
  command.dispatchEvent(new MouseEvent("click", { bubbles: true }));
  return command;
}

function seedMentionWorkspace(mounted: MountedInput) {
  const context = useWorkspaceContextStore(mounted.pinia);
  const explorer = useWorkspaceExplorerStore(mounted.pinia);
  const workbench = useWorkbenchStore(mounted.pinia);
  context.checkoutsById = {
    "checkout-a": { checkoutId: "checkout-a", projectId: "project-a", root: "C:/projects/a", normalizedRoot: "C:/projects/a", lastOpenedAt: 0 },
    "checkout-b": { checkoutId: "checkout-b", projectId: "project-a", root: "D:/projects/b", normalizedRoot: "D:/projects/b", lastOpenedAt: 0 },
  };
  const tab = createWorkbenchEditorInput(
    { kind: "workspaceFile", projectId: "project-a", path: "docs/Hero.md" },
    "Hero.md", { checkoutBinding: CHECKOUT_A },
  );
  workbench.windows.main = {
    schemaVersion: 1, windowId: "main", sidebar: { width: 300, collapsed: false },
    layout: { kind: "group", paneId: "main" }, focusedPaneId: "main",
    groups: { main: { paneId: "main", tabs: [tab], activeEditorId: tab.editorId } },
  };
  explorer.snapshots["project-a"] = {
    projectId: "project-a", presetId: "preset", presetName: "Default", manifestPath: "", revision: 1, presets: [],
    nodes: [{ nodeId: "docs", projectId: "project-a", nodeKind: "folder", sourcePath: "C:/projects/a/docs", hidden: false, position: 0 }],
  };
  explorer.mountListings["project-a:preset:docs"] = {
    nodeId: "docs", rootPath: "C:/projects/a/docs", truncated: false,
    entries: [
      { nodeId: "docs", relativePath: "Hero.md", absolutePath: "C:/projects/a/docs/Hero.md", name: "Hero.md", isDir: false, depth: 0 },
      { nodeId: "docs", relativePath: "HeroTree.md", absolutePath: "C:/projects/a/docs/HeroTree.md", name: "HeroTree.md", isDir: false, depth: 0 },
    ],
  };
  return { context, explorer, workbench, tab };
}

function mentionNames(mounted: MountedInput) {
  return Array.from(mounted.host.querySelectorAll(".mention-name"), (entry) => entry.textContent);
}

beforeEach(() => {
  vi.useFakeTimers();
  vi.stubGlobal("BroadcastChannel", undefined);
  mocks.searchWorkspaceAssets.mockReset().mockResolvedValue([]);
  mocks.searchWorkspaceSceneObjects.mockReset().mockResolvedValue([]);
  mocks.listDirEntriesPage.mockReset().mockResolvedValue({ entries: [], nextOffset: 0, hasMore: false });
  mocks.searchWorkspaceEntries.mockReset().mockResolvedValue([]);
  mocks.knowledgeQuery.mockReset().mockResolvedValue([]);
  mocks.checkUnityConnectionStatus.mockReset().mockResolvedValue({ connected: false });
  mocks.getUnityConsoleText.mockReset().mockResolvedValue({ text: "", entries: [] });
  mocks.validateUnitySceneObject.mockReset().mockResolvedValue(undefined);
  mocks.getCachedFileToolWorkspaceBoundary.mockReset().mockReturnValue(null);
  mocks.getFileToolWorkspaceBoundary.mockReset().mockResolvedValue(true);
  mocks.addNotice.mockReset();
  mocks.removeNotice.mockReset();
  mocks.garbageCollection.mockReset().mockResolvedValue([]);
  mocks.assetDropHandler = null;
  mocks.fileDropHandler = null;
  mocks.textDropHandler = null;
});

afterEach(() => {
  for (const mounted of mountedInputs.splice(0)) {
    mounted.app.unmount();
    mounted.host.remove();
  }
  vi.clearAllTimers();
  vi.useRealTimers();
  vi.unstubAllGlobals();
});

describe("RichChatInput scoped controller", () => {
  it("keeps matching slow-provider candidates visible while the next query fills in", async () => {
    mocks.searchWorkspaceEntries.mockResolvedValue([
      { relPath: "docs/Hero.md", name: "Hero.md", parentPath: "docs", isDir: false, matchScore: 1 },
    ]);
    const mounted = await mountInput();
    await setComposerText(mounted.host, "@He");
    await vi.advanceTimersByTimeAsync(60);
    expect(mentionNames(mounted)).toEqual(["Hero.md"]);
    const pending = deferred<[]>();
    mocks.searchWorkspaceEntries.mockReturnValueOnce(pending.promise);
    const textarea = await setComposerText(mounted.host, "@Hero");
    await vi.advanceTimersByTimeAsync(60);
    expect(mentionNames(mounted)).toEqual(["Hero.md"]);
    expect(mounted.host.querySelector(".mention-loading-status")).not.toBeNull();
    // IME navigation belongs to the composition candidate window.
    const composingEscape = new KeyboardEvent("keydown", { key: "Escape", isComposing: true, bubbles: true, cancelable: true });
    textarea.dispatchEvent(composingEscape);
    await flushAsync();
    expect(composingEscape.defaultPrevented).toBe(false);
    expect(mounted.host.querySelector(".mention-popup")).not.toBeNull();
    pending.resolve([]);
    await flushAsync();
    expect(mentionNames(mounted)).toEqual([]);
  });

  it("pages through candidates and reveals the selected item in the virtual list", async () => {
    mocks.searchWorkspaceEntries.mockResolvedValue(Array.from({ length: 100 }, (_, index) => ({
      relPath: `docs/Hero${String(index).padStart(2, "0")}.md`, name: `Hero${String(index).padStart(2, "0")}.md`,
      parentPath: "docs", isDir: false, matchScore: 1,
    })));
    const mounted = await mountInput();
    const textarea = await setComposerText(mounted.host, "@Hero");
    await vi.advanceTimersByTimeAsync(60);
    for (let page = 0; page < 4; page += 1) {
      textarea.dispatchEvent(new KeyboardEvent("keydown", { key: "PageDown", bubbles: true, cancelable: true }));
      await flushAsync();
    }
    expect(mounted.host.querySelector(".highlighted .mention-name")?.textContent).toBe("Hero40.md");
    textarea.dispatchEvent(new KeyboardEvent("keydown", { key: "PageUp", bubbles: true, cancelable: true }));
    await flushAsync();
    expect(mounted.host.querySelector(".highlighted .mention-name")?.textContent).toBe("Hero30.md");
    textarea.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true, cancelable: true }));
    await flushAsync();
    expect(mounted.state.text).toBe("");
    expect(mounted.input.value!.exportDraft().localFiles).toMatchObject([
      { path: "C:/projects/a/docs/Hero30.md", isDir: false },
    ]);
  });

  it("shows tabs and loaded tree entries immediately, deduplicates remote results, and attaches generic files", async () => {
    const mounted = await mountInput();
    seedMentionWorkspace(mounted);
    mocks.searchWorkspaceEntries.mockResolvedValue([
      { relPath: "docs/Hero.md", name: "Hero.md", parentPath: "docs", isDir: false, matchScore: 900 },
      { relPath: "docs/HeroRemote.md", name: "HeroRemote.md", parentPath: "docs", isDir: false, matchScore: 900 },
    ]);
    await setComposerText(mounted.host, "@");
    expect(mentionNames(mounted).slice(0, 2)).toEqual(["Hero.md", "docs"]);
    await setComposerText(mounted.host, "@Hero");
    expect(mentionNames(mounted)).toEqual(["Hero.md", "HeroTree.md"]);
    expect(mocks.searchWorkspaceEntries).not.toHaveBeenCalled();
    await vi.advanceTimersByTimeAsync(60);
    await flushAsync();
    expect(mentionNames(mounted)).toEqual(["Hero.md", "HeroTree.md", "HeroRemote.md"]);
    mounted.host.querySelector<HTMLButtonElement>(".mention-select")!.dispatchEvent(
      new MouseEvent("mousedown", { bubbles: true, cancelable: true }),
    );
    await flushAsync();
    expect(mounted.state.text).toBe("");
    expect(mounted.input.value!.exportDraft().localFiles).toMatchObject([
      { path: "C:/projects/a/docs/Hero.md", name: "Hero.md", isDir: false },
    ]);
  });

  it("filters cached candidates on each keystroke and searches only the latest debounced query", async () => {
    const mounted = await mountInput();
    seedMentionWorkspace(mounted);
    await setComposerText(mounted.host, "@H");
    await setComposerText(mounted.host, "@He");
    await setComposerText(mounted.host, "@HeroTree");
    expect(mentionNames(mounted)).toEqual(["HeroTree.md"]);
    await vi.advanceTimersByTimeAsync(60);
    expect(mocks.searchWorkspaceEntries).toHaveBeenCalledExactlyOnceWith("HeroTree", CHECKOUT_A);
  });

  it("preserves the keyboard-selected result when another provider inserts an earlier match", async () => {
    const pendingAssets = deferred<Array<{ path: string; name: string; matchScore: number }>>();
    mocks.searchWorkspaceAssets.mockReturnValue(pendingAssets.promise);
    mocks.searchWorkspaceEntries.mockResolvedValue([
      { relPath: "docs/HeroAlpha.md", name: "HeroAlpha.md", parentPath: "docs", isDir: false, matchScore: 1 },
      { relPath: "docs/HeroBeta.md", name: "HeroBeta.md", parentPath: "docs", isDir: false, matchScore: 1 },
    ]);
    const mounted = await mountInput();
    const textarea = await setComposerText(mounted.host, "@Hero");
    await vi.advanceTimersByTimeAsync(60);
    await flushAsync();
    textarea.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowDown", bubbles: true }));
    await flushAsync();
    const selected = mounted.host.querySelector(".mention-item.highlighted .mention-name")!.textContent;
    pendingAssets.resolve([{ path: "Assets/Hero", name: "Hero", matchScore: 1200 }]);
    await flushAsync();
    expect(mentionNames(mounted)[0]).toBe("Hero");
    expect(mounted.host.querySelector(".mention-item.highlighted .mention-name")!.textContent).toBe(selected);
  });

  it("drops stale query results during the debounce interval and recovers when typing back", async () => {
    const old = deferred<Array<{ path: string; name: string; matchScore: number }>>();
    mocks.searchWorkspaceAssets.mockReturnValueOnce(old.promise);
    const mounted = await mountInput();
    await setComposerText(mounted.host, "@Hero");
    await vi.advanceTimersByTimeAsync(60);
    await setComposerText(mounted.host, "@HeroNew");
    old.resolve([{ path: "Assets/HeroNewStale", name: "HeroNewStale", matchScore: 1000 }]);
    await flushAsync();
    expect(mentionNames(mounted)).toEqual([]);
    await vi.advanceTimersByTimeAsync(60);
    mocks.searchWorkspaceAssets.mockResolvedValue([{ path: "Assets/Hero", name: "Hero", matchScore: 1000 }]);
    await setComposerText(mounted.host, "@Hero");
    await vi.advanceTimersByTimeAsync(60);
    await flushAsync();
    expect(mentionNames(mounted)).toEqual(["Hero"]);
  });

  it("does not leak another checkout's tabs or mounted files into local suggestions", async () => {
    const mounted = await mountInput();
    const { workbench, explorer } = seedMentionWorkspace(mounted);
    workbench.windows.main!.groups.main!.tabs.push(createWorkbenchEditorInput(
      { kind: "asset", projectId: "project-a", path: "Assets/HeroOther.prefab" },
      "HeroOther.prefab", { checkoutBinding: CHECKOUT_B },
    ));
    explorer.mountListings["project-a:preset:docs"]!.entries.push({
      nodeId: "docs", relativePath: "HeroWrong.md", absolutePath: "D:/projects/b/HeroWrong.md",
      name: "HeroWrong.md", isDir: false, depth: 0,
    });
    await setComposerText(mounted.host, "@Hero");
    expect(mentionNames(mounted)).toEqual(["Hero.md", "HeroTree.md"]);
    mounted.state.workspaceRef = CHECKOUT_B;
    mounted.state.workspaceRoot = "D:/projects/b";
    await flushAsync();
    await setComposerText(mounted.host, "@HeroOther");
    expect(mentionNames(mounted)).toEqual(["HeroOther.prefab"]);
  });

  it("keeps immediate local results when providers fail and settles without a workspace", async () => {
    mocks.searchWorkspaceAssets.mockRejectedValue(new Error("offline"));
    mocks.searchWorkspaceEntries.mockRejectedValue(new Error("offline"));
    mocks.knowledgeQuery.mockRejectedValue(new Error("offline"));
    const mounted = await mountInput();
    seedMentionWorkspace(mounted);
    await setComposerText(mounted.host, "@Hero");
    await vi.advanceTimersByTimeAsync(60);
    await flushAsync();
    expect(mentionNames(mounted)).toEqual(["Hero.md", "HeroTree.md"]);
    expect(mounted.host.querySelector(".mention-search-header")).not.toBeNull();
    expect(mounted.host.querySelector(".mention-loading-status")).toBeNull();
    mounted.state.workspaceRef = null;
    await flushAsync();
    await setComposerText(mounted.host, "@Missing");
    await vi.advanceTimersByTimeAsync(60);
    expect(mounted.host.querySelector(".mention-empty")).not.toBeNull();
  });

  it("coalesces slow filesystem queries while letting other providers publish independently", async () => {
    const old = deferred<[]>();
    mocks.searchWorkspaceEntries.mockReturnValueOnce(old.promise);
    const mounted = await mountInput();
    await setComposerText(mounted.host, "@H");
    await vi.advanceTimersByTimeAsync(60);
    await setComposerText(mounted.host, "@He");
    await vi.advanceTimersByTimeAsync(60);
    mocks.searchWorkspaceAssets.mockResolvedValue([{ path: "Assets/Hero", name: "Hero", matchScore: 1000 }]);
    await setComposerText(mounted.host, "@Hero");
    await vi.advanceTimersByTimeAsync(60);
    expect(mentionNames(mounted)).toEqual(["Hero"]);
    expect(mocks.searchWorkspaceEntries).toHaveBeenCalledTimes(1);
    old.resolve([]);
    await flushAsync();
    await flushAsync();
    expect(mocks.searchWorkspaceEntries.mock.calls.map((call) => call[0])).toEqual(["H", "Hero"]);
  });

  it("reads the cached directory immediately and cancels pending search on Escape", async () => {
    const mounted = await mountInput();
    seedMentionWorkspace(mounted);
    mocks.listDirEntriesPage.mockReturnValue(new Promise(() => {}));
    await setComposerText(mounted.host, "@docs/Hero");
    expect(mentionNames(mounted)).toEqual(["docs/", "Hero.md", "HeroTree.md"]);
    expect(mocks.listDirEntriesPage).toHaveBeenCalledWith("docs", CHECKOUT_A, 0, 200, false);
    const textarea = await setComposerText(mounted.host, "@Hero");
    textarea.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true, cancelable: true }));
    await vi.advanceTimersByTimeAsync(100);
    expect(mocks.searchWorkspaceAssets).not.toHaveBeenCalled();
    expect(mounted.host.querySelector(".mention-popup")).toBeNull();
  });

  it("updates suggestions when tabs close or tree nodes hide, and attaches external mounted files", async () => {
    const mounted = await mountInput();
    const { workbench, explorer } = seedMentionWorkspace(mounted);
    await setComposerText(mounted.host, "@Hero");
    workbench.windows.main!.groups.main!.tabs = [];
    explorer.snapshots["project-a"]!.nodes[0]!.hidden = true;
    await flushAsync();
    expect(mentionNames(mounted)).toEqual([]);
    explorer.snapshots["project-a"]!.nodes.push({
      nodeId: "external", projectId: "project-a", nodeKind: "resource", resourceKind: "local_file",
      sourcePath: "E:/Reference/Hero.txt", hidden: false, position: 1,
    });
    await flushAsync();
    expect(mentionNames(mounted)).toEqual(["Hero.txt"]);
    mounted.host.querySelector<HTMLButtonElement>(".mention-select")!.dispatchEvent(
      new MouseEvent("mousedown", { bubbles: true, cancelable: true }),
    );
    await flushAsync();
    expect(mounted.input.value!.exportDraft().localFiles).toMatchObject([{ path: "E:/Reference/Hero.txt", isDir: false }]);
    expect(mounted.state.text).toBe("");
  });

  it("uses cached project knowledge and skips tabs from an obsolete materialization", async () => {
    const mounted = await mountInput();
    const { explorer, workbench } = seedMentionWorkspace(mounted);
    explorer.resources["project-a"] = {
      sessions: [], collaboration: null,
      knowledge: [{
        id: "hero-design", type: "design", path: "hero.md", title: "Hero design",
        injectMode: "inherit", effectiveInjectMode: "path", readOnly: false,
        aiMaintained: "inherit", effectiveAiMaintained: true, modifiedAt: 0,
        sourceCheckoutId: "checkout-a", sourceRoot: "C:/projects/a", availableCheckoutIds: ["checkout-a"],
      }],
    };
    explorer.snapshots["project-a"]!.nodes.push({
      nodeId: "knowledge", projectId: "project-a", nodeKind: "resource", resourceKind: "knowledge",
      resourceId: "hero-design", hidden: false, position: 2,
    });
    workbench.windows.main!.groups.main!.tabs.push(createWorkbenchEditorInput(
      { kind: "workspaceFile", projectId: "project-a", path: "HeroObsolete.md" }, "HeroObsolete.md",
      { checkoutBinding: { ...CHECKOUT_A, expectedMaterializationEpoch: 5 } },
    ));
    await setComposerText(mounted.host, "@Hero");
    expect(mentionNames(mounted)).toEqual(["Hero.md", "Hero design", "HeroTree.md"]);
    const document = Array.from(mounted.host.querySelectorAll<HTMLButtonElement>(".mention-select"))
      .find((entry) => entry.textContent?.includes("Hero design"))!;
    document.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, cancelable: true }));
    await flushAsync();
    expect(mounted.input.value!.exportDraft().assetRefs).toMatchObject([{ path: "design/hero.md", kind: "knowledge" }]);
  });


  it("runs garbage collection for the input checkout without sending an agent message", async () => {
    const mounted = await mountInput();
    const textarea = await setComposerText(mounted.host, "/garbage-collection");
    textarea.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true, cancelable: true }));
    await flushAsync();
    expect(mocks.garbageCollection).toHaveBeenCalledExactlyOnceWith(CHECKOUT_A);
    expect(mounted.send).not.toHaveBeenCalled();
    expect(mounted.state.text).toBe("");
  });

  it.each(["mouse", "Enter", "Tab"])("attaches a searched knowledge folder with %s and preserves existing attachments", async (selection) => {
    const path = "Locus/knowledge/Design/Characters/C100_猫头鹰";
    mocks.searchWorkspaceEntries.mockResolvedValue([{
      relPath: path, name: "C100_猫头鹰", parentPath: "Locus/knowledge/Design/Characters",
      isDir: true, matchScore: 1000,
    }]);
    const mounted = await mountInput({ workspaceRoot: "C:\\projects\\a\\" });
    const input = mounted.input.value!;
    const existingAssetRef = { path: "Assets/Characters/C7100Genichiro", kind: "asset" as const, source: "manual" as const };
    await input.appendDraft({ ...input.exportDraft(), assetRefs: [existingAssetRef] });

    for (let attempt = 0; attempt < 2; attempt += 1) {
      const textarea = await setComposerText(mounted.host, "结合弦一郎的动画表，@猫头鹰");
      await vi.advanceTimersByTimeAsync(60);
      if (selection === "mouse") {
        mounted.host.querySelector<HTMLButtonElement>(".mention-select")!.dispatchEvent(
          new MouseEvent("mousedown", { bubbles: true, cancelable: true }),
        );
      } else {
        textarea.dispatchEvent(new KeyboardEvent("keydown", { key: selection, bubbles: true, cancelable: true }));
      }
      await flushAsync();
      await vi.advanceTimersByTimeAsync(50);
      expect(mounted.state.text).toBe("结合弦一郎的动画表，");
      expect(mounted.host.querySelector(".mention-popup")).toBeNull();
      expect(mounted.host.querySelector(".local-file-chip")?.textContent).toContain("C100_猫头鹰");
      expect(input.exportDraft().localFiles).toMatchObject([
        { path: `C:/projects/a/${path}`, name: "C100_猫头鹰", isDir: true },
      ]);
      expect(input.exportDraft().assetRefs).toEqual([existingAssetRef]);
      expect(textarea.selectionStart).toBe(mounted.state.text.length);
    }

    expect(mocks.addNotice).not.toHaveBeenCalled();
    const textarea = mounted.host.querySelector<HTMLTextAreaElement>("textarea.chat-composer-input")!;
    textarea.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true, cancelable: true }));
    await flushAsync();
    expect(mounted.send).toHaveBeenCalledOnce();
    const payload = mounted.send.mock.calls[0]![0];
    expect(payload.text).toContain(`- folder: \`C:/projects/a/${path}\``);
    expect(payload.text).toContain("`list` for folders");
    expect(payload.assetRefs).toEqual([existingAssetRef]);
  });

  it.each([
    { query: "@docs/", target: "docs/", path: "docs", isDir: true },
    { query: "@docs/", target: "Characters", path: "docs/Characters", isDir: true },
    { query: "@docs/", target: "Notes.txt", path: "docs/Notes.txt", isDir: false },
  ])("attaches the browsed workspace entry $target", async ({ query, target, path, isDir }) => {
    mocks.listDirEntriesPage.mockResolvedValue({
      entries: [
        { relPath: "docs/Characters", name: "Characters", isDir: true },
        { relPath: "docs/Notes.txt", name: "Notes.txt", isDir: false },
      ],
      nextOffset: 2, hasMore: false,
    });
    const mounted = await mountInput();
    const textarea = await setComposerText(mounted.host, query);
    await flushAsync();
    const index = mentionNames(mounted).indexOf(target);
    expect(index).toBeGreaterThanOrEqual(0);
    for (let step = 0; step < index; step += 1) {
      textarea.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowDown", bubbles: true, cancelable: true }));
    }
    textarea.dispatchEvent(new KeyboardEvent("keydown", { key: "Tab", bubbles: true, cancelable: true }));
    await flushAsync();
    expect(mounted.input.value!.exportDraft().localFiles).toMatchObject([
      { path: `C:/projects/a/${path}`, isDir },
    ]);
    expect(mounted.state.text).toBe("");
  });

  it("keeps a workspace knowledge document as a knowledge attachment", async () => {
    mocks.searchWorkspaceEntries.mockResolvedValue([{
      relPath: "Locus/knowledge/Design/Characters/猫头鹰.md", name: "猫头鹰.md",
      parentPath: "Locus/knowledge/Design/Characters", isDir: false, matchScore: 1000,
    }]);
    const mounted = await mountInput();
    const textarea = await setComposerText(mounted.host, "@猫头鹰");
    await vi.advanceTimersByTimeAsync(60);
    textarea.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true, cancelable: true }));
    await flushAsync();
    expect(mounted.state.text).toBe("");
    expect(mounted.input.value!.exportDraft().assetRefs).toMatchObject([
      { path: "design/Characters/猫头鹰.md", kind: "knowledge", source: "manual" },
    ]);
    expect(mounted.input.value!.exportDraft().localFiles).toEqual([]);
  });

  it.each(["workspace", "knowledge"])("shows one knowledge reference when the %s provider finishes first", async (first) => {
    const name = "《尘之回声》战斗策划案";
    const workspaceResults = [{
      relPath: `Locus/knowledge/Design/${name}.md`, name: `${name}.md`,
      parentPath: "Locus/knowledge/Design", isDir: false, matchScore: 1000,
    }];
    const knowledgeResults = [{ type: "design", path: `${name}.md`, title: name, score: 1 }];
    const workspace = deferred<typeof workspaceResults>();
    const knowledge = deferred<typeof knowledgeResults>();
    mocks.searchWorkspaceEntries.mockReturnValue(workspace.promise);
    mocks.knowledgeQuery.mockReturnValue(knowledge.promise);
    const mounted = await mountInput();
    const textarea = await setComposerText(mounted.host, "@战斗策划");
    await vi.advanceTimersByTimeAsync(60);

    if (first === "workspace") workspace.resolve(workspaceResults);
    else knowledge.resolve(knowledgeResults);
    await flushAsync();
    expect(mentionNames(mounted)).toEqual([first === "workspace" ? `${name}.md` : name]);
    textarea.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowDown", bubbles: true }));
    await flushAsync();

    if (first === "workspace") knowledge.resolve(knowledgeResults);
    else workspace.resolve(workspaceResults);
    await flushAsync();
    expect(mentionNames(mounted)).toEqual([name]);
    expect(mounted.host.querySelector(".highlighted .mention-name")?.textContent).toBe(name);

    textarea.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true, cancelable: true }));
    await flushAsync();
    expect(mounted.state.text).toBe("");
    expect(mounted.input.value!.exportDraft().assetRefs).toMatchObject([
      { path: `design/${name}.md`, kind: "knowledge", source: "manual" },
    ]);
    expect(mounted.input.value!.exportDraft().localFiles).toEqual([]);
  });

  it("renders and submits a workspace knowledge folder as a directory reference", async () => {
    const mounted = await mountInput();
    const attachment = workbenchComposerTreeFileAttachment({
      kind: "folder",
      explorerNode: {
        sourcePath: "C:/projects/a/Locus/knowledge/reference/Unity Manual",
        sourceKind: "knowledge",
      },
      name: "Unity Manual",
    }, mounted.state.workspaceRoot);
    expect(attachment?.localFile).toBeDefined();
    const input = mounted.input.value!;
    const draft = {
      ...input.exportDraft(),
      localFiles: [attachment!.localFile!],
    };
    await input.appendDraft(draft);
    await input.appendDraft(draft);
    await flushAsync();

    expect(mounted.host.querySelector(".local-file-chip")?.textContent).toContain("Unity Manual");
    expect(input.exportDraft().localFiles).toHaveLength(1);
    expect(input.exportDraft().localFiles[0]).toMatchObject({ isDir: true, source: "knowledge" });
    expect(input.exportDraft().assetRefs).toEqual([]);

    const textarea = await setComposerText(mounted.host, "查看这个目录");
    textarea.dispatchEvent(new KeyboardEvent("keydown", {
      key: "Enter", bubbles: true, cancelable: true,
    }));
    await flushAsync();
    expect(mounted.send).toHaveBeenCalledOnce();
    expect(mounted.send.mock.calls[0]?.[0].text).toContain(
      "- folder: `C:/projects/a/Locus/knowledge/reference/Unity Manual`",
    );
    expect(mounted.send.mock.calls[0]?.[0].text).toContain("`list` for folders");
  });

  it("requests sticky Plan mode through its owner and keeps a message fallback until confirmed", async () => {
    const mounted = await mountInput();
    await setComposerText(mounted.host, "/plan");

    const command = clickCommand(mounted.host, "/plan");
    await flushAsync();

    expect(command.tagName).toBe("BUTTON");
    expect(command.getAttribute("role")).toBe("option");
    expect(mounted.requestPlanMode).toHaveBeenCalledOnce();
    expect(mounted.requestPlanMode).toHaveBeenCalledWith(true);
    expect(mounted.state.text).toBe("");
    expect(mounted.host.querySelector(".composer-badge.plan")).not.toBeNull();

    mounted.state.planModeActive = true;
    await nextTick();
    expect(mounted.host.querySelector(".composer-badge.plan")).toBeNull();
  });

  it("executes action commands from the keyboard through component events", async () => {
    const mounted = await mountInput();
    let textarea = await setComposerText(mounted.host, "/fork");
    textarea.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true, cancelable: true }));
    await flushAsync();
    expect(mounted.fork).toHaveBeenCalledOnce();

    textarea = await setComposerText(mounted.host, "/undo");
    textarea.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true, cancelable: true }));
    await flushAsync();
    expect(mounted.undo).toHaveBeenCalledOnce();

    textarea = await setComposerText(mounted.host, "/compact");
    textarea.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true, cancelable: true }));
    await flushAsync();
    expect(mounted.compact).toHaveBeenCalledOnce();

    textarea = await setComposerText(mounted.host, "/export-context");
    textarea.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true, cancelable: true }));
    await flushAsync();
    expect(mounted.exportContext).toHaveBeenCalledOnce();

    textarea = await setComposerText(mounted.host, "/review-context");
    textarea.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true, cancelable: true }));
    await flushAsync();
    expect(mounted.reviewContext).toHaveBeenCalledOnce();
  });

  it("binds @ search and scene validation to the editor checkout and discards stale results", async () => {
    const firstStatus = deferred<Record<string, unknown>>();
    mocks.checkUnityConnectionStatus.mockReturnValueOnce(firstStatus.promise);
    const mounted = await mountInput();

    await setComposerText(mounted.host, "@Hero");
    await vi.advanceTimersByTimeAsync(160);
    await flushAsync();

    expect(mocks.searchWorkspaceAssets).toHaveBeenCalledWith(
      "Hero",
      ["Assets", "Packages", "ProjectSettings"],
      undefined,
      CHECKOUT_A,
    );
    expect(mocks.searchWorkspaceEntries).toHaveBeenCalledWith("Hero", CHECKOUT_A);
    expect(mocks.knowledgeQuery).toHaveBeenCalledWith(
      expect.objectContaining({ query: "Hero" }),
      CHECKOUT_A,
    );
    expect(mocks.checkUnityConnectionStatus).toHaveBeenCalledWith(CHECKOUT_A);

    mounted.state.workspaceRef = CHECKOUT_B;
    mounted.state.workspaceRoot = "D:/projects/b";
    await nextTick();
    firstStatus.resolve({
      connected: true,
      scenePath: "Assets/Scenes/A.unity",
      scenePaths: ["Assets/Scenes/A.unity"],
    });
    await flushAsync();
    expect(mocks.searchWorkspaceSceneObjects).not.toHaveBeenCalled();

    mocks.checkUnityConnectionStatus.mockResolvedValue({
      connected: true,
      scenePath: "Assets/Scenes/B.unity",
      scenePaths: ["Assets/Scenes/B.unity"],
    });
    mocks.searchWorkspaceSceneObjects.mockResolvedValue([{
      scenePath: "Assets/Scenes/B.unity",
      objectPath: "Root/Enemy",
      name: "Enemy",
      matchScore: 100,
    }]);
    await setComposerText(mounted.host, "@Enemy");
    await vi.advanceTimersByTimeAsync(160);
    await flushAsync();

    const latestAssetSearch = mocks.searchWorkspaceAssets.mock.calls[
      mocks.searchWorkspaceAssets.mock.calls.length - 1
    ];
    expect(latestAssetSearch?.[3]).toEqual(CHECKOUT_B);
    expect(mocks.searchWorkspaceSceneObjects).toHaveBeenCalledWith(
      "Assets/Scenes/B.unity",
      "Enemy",
      160,
      CHECKOUT_B,
    );

    const enemy = Array.from(mounted.host.querySelectorAll<HTMLButtonElement>(".mention-select"))
      .find((item) => item.textContent?.includes("Enemy"));
    expect(enemy).toBeDefined();
    enemy!.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, cancelable: true }));
    await flushAsync();
    expect(mocks.validateUnitySceneObject).toHaveBeenCalledWith(
      CHECKOUT_B,
      "Assets/Scenes/B.unity",
      "Root/Enemy",
    );
  });

  it("cancels an in-flight Unity Console read when the editor checkout changes", async () => {
    const firstStatus = deferred<Record<string, unknown>>();
    mocks.checkUnityConnectionStatus.mockReturnValueOnce(firstStatus.promise);
    const mounted = await mountInput();

    let textarea = await setComposerText(mounted.host, "/unity-console");
    textarea.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true, cancelable: true }));
    await flushAsync();
    expect(mocks.checkUnityConnectionStatus).toHaveBeenCalledWith(CHECKOUT_A);

    mounted.state.workspaceRef = CHECKOUT_B;
    mounted.state.workspaceRoot = "D:/projects/b";
    await nextTick();
    firstStatus.resolve({ connected: true });
    await flushAsync();
    expect(mocks.getUnityConsoleText).not.toHaveBeenCalled();

    mocks.checkUnityConnectionStatus.mockResolvedValue({ connected: true });
    mocks.getUnityConsoleText.mockResolvedValue({ text: "Checkout B error", title: "B Console" });
    await setComposerText(mounted.host, "");
    textarea = await setComposerText(mounted.host, "/unity-console");
    textarea.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true, cancelable: true }));
    await flushAsync();

    expect(mocks.getUnityConsoleText).toHaveBeenCalledWith(CHECKOUT_B);
    const consoleGroup = mounted.host.querySelector<HTMLButtonElement>(".console-text-group-button");
    expect(consoleGroup).not.toBeNull();
    consoleGroup!.click();
    await nextTick();
    expect(mounted.host.querySelector(".console-text-detail-title")?.textContent).toBe("B Console");
  });

  it("accepts pushed Unity Console text only from the exact checkout generation", async () => {
    const mounted = await mountInput();
    expect(mocks.textDropHandler).not.toBeNull();

    mocks.textDropHandler!({ text: "missing scope" });
    mocks.textDropHandler!({ workspaceRef: CHECKOUT_B, text: "other checkout" });
    mocks.textDropHandler!({
      workspaceRef: { checkoutId: CHECKOUT_A.checkoutId, expectedGeneration: 2 },
      text: "stale generation",
    });
    await flushAsync();
    expect(mounted.host.querySelector(".console-text-group")).toBeNull();

    mocks.textDropHandler!({
      workspaceRef: CHECKOUT_A,
      text: "matching checkout",
      title: "Scoped Console",
    });
    await flushAsync();
    expect(mounted.host.querySelector(".console-text-group")).not.toBeNull();
  });

  it("accepts native file drops without creating sessions and evaluates file boundaries", async () => {
    const mounted = await mountInput({ managedNativeDrops: false });
    expect(mocks.fileDropHandler).not.toBeNull();

    mocks.fileDropHandler!({
      files: [{ path: "C:/projects/a/inside.txt", isDir: false }],
    });
    await flushAsync();
    expect(mounted.requestNewSession).not.toHaveBeenCalled();
    expect(mounted.host.querySelector(".local-file-chip")).not.toBeNull();
    expect(mocks.getFileToolWorkspaceBoundary).not.toHaveBeenCalled();

    mocks.fileDropHandler!({
      files: [{ path: "C:/outside/external.txt", isDir: false }],
    });
    await flushAsync();
    expect(mocks.getFileToolWorkspaceBoundary).toHaveBeenCalledOnce();
    expect(mocks.addNotice).toHaveBeenCalledWith(
      "warning",
      expect.any(String),
      expect.objectContaining({ operation: "local-file-boundary-warning" }),
    );
  });
});
