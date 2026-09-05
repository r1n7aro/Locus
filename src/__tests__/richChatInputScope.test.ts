// @vitest-environment jsdom
import { createPinia } from "pinia";
import {
  createApp,
  defineComponent,
  h,
  nextTick,
  reactive,
  type App,
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
  assetDropHandler: null as null | ((payload: Record<string, unknown>) => void),
  fileDropHandler: null as null | ((payload: Record<string, unknown>) => void),
  textDropHandler: null as null | ((payload: Record<string, unknown>) => void),
}));

vi.mock("../services/asset", () => ({
  searchWorkspaceAssets: mocks.searchWorkspaceAssets,
  searchWorkspaceSceneObjects: mocks.searchWorkspaceSceneObjects,
}));

vi.mock("../services/project", () => ({
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
  useNotificationStore: () => ({ addNotice: mocks.addNotice }),
}));

import RichChatInput from "../components/chat/RichChatInput.vue";

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
  host: HTMLElement;
  state: HarnessState;
  requestPlanMode: ReturnType<typeof vi.fn>;
  requestNewSession: ReturnType<typeof vi.fn>;
  fork: ReturnType<typeof vi.fn>;
  undo: ReturnType<typeof vi.fn>;
  compact: ReturnType<typeof vi.fn>;
  exportContext: ReturnType<typeof vi.fn>;
  reviewContext: ReturnType<typeof vi.fn>;
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
  const Root = defineComponent({
    setup() {
      return () => h(RichChatInput, {
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
      });
    },
  });
  const host = document.createElement("div");
  document.body.appendChild(host);
  const app = createApp(Root);
  app.use(createPinia());
  app.mount(host);
  const mounted = {
    app,
    host,
    state,
    requestPlanMode,
    requestNewSession,
    fork,
    undo,
    compact,
    exportContext,
    reviewContext,
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
