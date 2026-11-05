// @vitest-environment jsdom
import { EditorView } from "@codemirror/view";
import { createApp, h, nextTick, reactive, type App } from "vue";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import WorkbenchEditorStack from "../components/workbench/WorkbenchEditorStack.vue";
import WorkspaceFilePreview from "../components/workbench/WorkspaceFilePreview.vue";
import { WorkspaceFilePreviewCache } from "../components/workbench/workspaceFilePreviewCache";
import type { ProjectExplorerFilePreview, ProjectExplorerFileRevision, WorkbenchEditorGroup, WorkbenchEditorInput } from "../types/workbench";

const files = vi.hoisted(() => ({ read: vi.fn(), write: vi.fn(), revision: vi.fn() }));
vi.mock("../services/workspaceExplorer", () => ({
  projectExplorerPreviewFile: files.read,
  workspaceFilePreview: files.read,
  projectExplorerWriteFile: files.write,
  workspaceFileWrite: files.write,
  projectExplorerFileRevision: files.revision,
  workspaceFileRevision: files.revision,
  subscribeWorkspaceFileChanges: vi.fn(async () => () => {}),
}));

let app: App<Element> | null = null;
function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (error: Error) => void;
  const promise = new Promise<T>((yes, no) => { resolve = yes; reject = no; });
  return { promise, resolve, reject };
}
function preview(path: string, text = `# ${path}\n\nContent`, version = "v1"): ProjectExplorerFilePreview {
  return {
    path, name: path, extension: "md", kind: "text", mimeType: "text/markdown",
    text, size: text.length, contentHash: version, editable: true, truncated: false,
    revision: { exists: true, size: text.length, modifiedAtNanos: version, key: version },
  };
}
function editor(id: string, path = `${id}.md`, kind: "workspaceFile" | "localFile" | "asset" = "workspaceFile", generation = 1): WorkbenchEditorInput {
  return {
    editorId: id, sourcePath: path,
    resource: kind === "localFile" ? { kind, projectId: "project", nodeId: path } : { kind, projectId: "project", path },
    title: path, preview: true, pinned: false, dirty: false, availability: "available",
    capabilities: { split: true, detach: true, duplicate: true },
    checkoutBinding: { checkoutId: "checkout", expectedGeneration: generation, expectedMaterializationEpoch: 1 },
  };
}
async function flush() {
  for (let i = 0; i < 8; i++) { await Promise.resolve(); await nextTick(); }
}
function mountStack(initial = editor("a")) {
  const group = reactive<WorkbenchEditorGroup>({ paneId: "pane", tabs: [initial], activeEditorId: initial.editorId });
  const host = document.createElement("div");
  document.body.appendChild(host);
  app = createApp({ render: () => h(WorkbenchEditorStack, { group }, {
    default: ({ editor: item, contentActive, ready }: { editor: WorkbenchEditorInput; contentActive: boolean; ready: () => void }) => h(WorkspaceFilePreview, {
      path: item.sourcePath!, projectId: item.resource.projectId,
      workspaceRef: item.resource.kind === "localFile" ? null : item.checkoutBinding,
      active: contentActive, onReady: ready,
    }),
  }) });
  app.mount(host);
  const surface = (id: string) => host.querySelector<HTMLElement>(`[data-editor-id="${id}"]`);
  const visible = () => Array.from(host.querySelectorAll<HTMLElement>(".workbench-editor-instance"))
    .filter((element) => element.style.display !== "none" && !element.classList.contains("is-preparing"));
  const replace = async (item: WorkbenchEditorInput) => {
    group.tabs = [item]; group.activeEditorId = item.editorId;
    await flush();
  };
  return { host, surface, visible, replace };
}

beforeEach(() => {
  files.read.mockReset().mockImplementation(async (path: string) => preview(path));
  files.write.mockReset();
  files.revision.mockReset().mockImplementation(async () => preview("file.md").revision);
  Object.defineProperty(Range.prototype, "getClientRects", { configurable: true, value: () => [] });
  Object.defineProperty(Range.prototype, "getBoundingClientRect", { configurable: true,
    value: () => ({ left: 0, right: 0, top: 0, bottom: 0, width: 0, height: 0 }) });
});
afterEach(() => { app?.unmount(); app = null; document.body.replaceChildren(); });

describe("file preview presentation", () => {
  it.each(["workspaceFile", "localFile", "asset"] as const)("keeps the outgoing Markdown DOM until %s is rendered", async (kind) => {
    if (kind === "localFile") files.read.mockImplementation(async (_project: string, path: string) => preview(path));
    const stack = mountStack(editor("a", "a.md", kind));
    await flush();
    const previous = stack.surface("a")!;
    const dom = previous.querySelector<HTMLElement>(".cm-editor")!;
    expect(EditorView.findFromDOM(dom)?.state.doc.toString()).toContain("a.md");
    const pending = deferred<ProjectExplorerFilePreview>();
    files.read.mockReturnValueOnce(pending.promise);
    await stack.replace(editor("b", "b.md", kind));
    expect(stack.visible()).toEqual([previous]);
    expect(previous.querySelector(".cm-editor")).toBe(dom);
    expect(previous.hasAttribute("inert")).toBe(true);
    expect(previous.querySelector(".workspace-file-preview-state")).toBeNull();
    pending.resolve(preview("b.md"));
    await flush();
    expect(stack.visible()).toEqual([stack.surface("b")]);
    expect(stack.surface("b")?.querySelector(".cm-editor")).not.toBeNull();
    expect(stack.surface("a")).toBeNull();
  });

  it("shows a cached document before disk validation and avoids a second content read", async () => {
    const stack = mountStack();
    await flush();
    await stack.replace(editor("b"));
    const revision = deferred<ProjectExplorerFileRevision>();
    files.revision.mockReturnValueOnce(revision.promise);
    await stack.replace(editor("a-again", "a.md"));
    expect(stack.visible()).toEqual([stack.surface("a-again")]);
    expect(stack.surface("a-again")?.textContent).toContain("a.md");
    expect(files.read).toHaveBeenCalledTimes(2);
    expect(files.revision).toHaveBeenCalledOnce();
    revision.resolve(preview("a.md").revision);
    await flush();
    expect(files.read).toHaveBeenCalledTimes(2);
  });

  it("refreshes cached content in place when the disk revision changes", async () => {
    const stack = mountStack();
    await flush();
    await stack.replace(editor("b"));
    const updated = preview("a.md", "# Changed externally", "v2");
    const revision = deferred<ProjectExplorerFileRevision>();
    const read = deferred<ProjectExplorerFilePreview>();
    files.revision.mockReturnValueOnce(revision.promise);
    files.read.mockReturnValueOnce(read.promise);
    await stack.replace(editor("a-again", "a.md"));
    const dom = stack.surface("a-again")?.querySelector(".cm-editor");
    revision.resolve(updated.revision);
    await flush();
    expect(stack.surface("a-again")?.querySelector(".cm-editor")).toBe(dom);
    expect(stack.surface("a-again")?.querySelector(".workspace-file-preview-state")).toBeNull();
    read.resolve(updated);
    await flush();
    expect(stack.surface("a-again")?.textContent).toContain("Changed externally");
    expect(stack.surface("a-again")?.querySelector(".cm-editor")).toBe(dom);
  });

  it("isolates cached previews by checkout lifetime", async () => {
    const stack = mountStack();
    await flush();
    await stack.replace(editor("b"));
    const pending = deferred<ProjectExplorerFilePreview>();
    files.read.mockReturnValueOnce(pending.promise);
    await stack.replace(editor("a-new-generation", "a.md", "workspaceFile", 2));
    expect(files.read).toHaveBeenCalledTimes(3);
    expect(stack.visible()).toEqual([stack.surface("b")]);
    pending.resolve(preview("a.md", "# New checkout"));
    await flush();
    expect(stack.visible()[0]?.textContent).toContain("New checkout");
  });

  it("keeps edits made while cached content is being refreshed", async () => {
    const stack = mountStack();
    await flush();
    await stack.replace(editor("b"));
    const revision = deferred<ProjectExplorerFileRevision>();
    const read = deferred<ProjectExplorerFilePreview>();
    files.revision.mockReturnValueOnce(revision.promise);
    files.read.mockReturnValueOnce(read.promise);
    await stack.replace(editor("a-again", "a.md"));
    const updated = preview("a.md", "# Remote content", "v2");
    revision.resolve(updated.revision);
    await flush();
    const dom = stack.surface("a-again")!.querySelector<HTMLElement>(".cm-editor")!;
    const view = EditorView.findFromDOM(dom)!;
    view.dispatch({ changes: { from: 0, to: view.state.doc.length, insert: "# Local edit" } });
    const state = view.state;
    read.resolve(updated);
    await flush();
    expect(view.state).toBe(state);
    expect(view.state.doc.toString()).toBe("# Local edit");
    expect(stack.surface("a-again")?.querySelector(".workspace-file-preview-conflict")).not.toBeNull();
    expect(files.write).not.toHaveBeenCalled();
  });

  it("caches the saved baseline instead of resurfacing an older version", async () => {
    const stack = mountStack();
    await flush();
    const dom = stack.surface("a")!.querySelector<HTMLElement>(".cm-editor")!;
    const view = EditorView.findFromDOM(dom)!;
    view.dispatch({ changes: { from: 0, to: view.state.doc.length, insert: "# Saved edit" } });
    const saved = preview("a.md", "# Saved edit", "v2");
    files.write.mockResolvedValueOnce(saved);
    view.contentDOM.dispatchEvent(new KeyboardEvent("keydown", {
      key: "s", code: "KeyS", ctrlKey: true, bubbles: true, cancelable: true,
    }));
    await flush();
    expect(files.write).toHaveBeenCalledOnce();
    await stack.replace(editor("b"));
    files.revision.mockResolvedValueOnce(saved.revision);
    await stack.replace(editor("a-again", "a.md"));
    expect(stack.visible()[0]?.textContent).toContain("Saved edit");
    expect(files.read).toHaveBeenCalledTimes(2);
  });

  it("ignores superseded loads and presents failures without getting stuck", async () => {
    const stack = mountStack();
    await flush();
    const b = deferred<ProjectExplorerFilePreview>();
    const c = deferred<ProjectExplorerFilePreview>();
    files.read.mockReturnValueOnce(b.promise).mockReturnValueOnce(c.promise);
    await stack.replace(editor("b"));
    await stack.replace(editor("c"));
    b.resolve(preview("b.md"));
    await flush();
    expect(stack.visible()).toEqual([stack.surface("a")]);
    c.reject(new Error("read failed"));
    await flush();
    expect(stack.visible()).toEqual([stack.surface("c")]);
    expect(stack.surface("c")?.textContent).toContain("read failed");
    await stack.replace(editor("d"));
    expect(stack.visible()[0]?.textContent).toContain("d.md");
  });
});

describe("file preview cache bounds", () => {
  it("evicts least recently used text and excludes large or binary payloads", () => {
    const cache = new WorkspaceFilePreviewCache();
    for (let i = 0; i < 12; i++) cache.set(String(i), preview(`${i}.md`));
    expect(cache.get("0")).toBeDefined();
    cache.set("12", preview("12.md"));
    expect(cache.get("0")).toBeDefined();
    expect(cache.get("1")).toBeUndefined();
    cache.set("large", preview("large.md", "x".repeat(512 * 1024 + 1)));
    cache.set("image", { ...preview("image.png"), kind: "image", blobUrl: "http://locus-binary.localhost/blob/123" });
    expect(cache.get("large")).toBeUndefined();
    expect(cache.get("image")).toBeUndefined();
  });
});
