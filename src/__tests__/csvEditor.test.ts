// @vitest-environment jsdom
import { createApp, h, nextTick, ref, type App } from "vue";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { ProjectExplorerFilePreview } from "../types/workbench";
import WorkspaceCsvEditor from "../components/csv/WorkspaceCsvEditor.vue";
import { csvEditorSessions } from "../components/csv/csvEditorSessions";
import { parseCsvView, serializeCsvView } from "../document/csv/csvView";

const mocks = vi.hoisted(() => ({ read: vi.fn(), write: vi.fn(), revision: vi.fn(), readView: vi.fn(), writeView: vi.fn(),
  fileListeners: new Set<(event: never) => void>() }));
vi.mock("../services/workspaceExplorer", () => ({
  projectExplorerPreviewFile: mocks.read, workspaceFilePreview: mocks.read,
  projectExplorerWriteFile: mocks.write, workspaceFileWrite: mocks.write,
  projectExplorerFileRevision: mocks.revision, workspaceFileRevision: mocks.revision,
  subscribeWorkspaceFileChanges: vi.fn(async (listener: (event: never) => void) => {
    mocks.fileListeners.add(listener); return () => mocks.fileListeners.delete(listener);
  }),
}));
vi.mock("../services/csvDocument", () => ({ readCsvViewFile: mocks.readView, writeCsvViewFile: mocks.writeView }));
vi.mock("../components/csv/CsvGrid.vue", async () => {
  const { defineComponent, h } = await import("vue");
  return { default: defineComponent({
    props: ["document", "view", "readOnly"], emits: ["edit", "viewChange"],
    setup(props, { emit, expose }) {
      expose({ getSnapshot: () => ({ row: 1, column: 0, scrollTop: 0, scrollLeft: 0 }),
        applySnapshot: async () => {}, selectedRows: () => [1], focus: () => {} });
      return () => h("div", { class: "test-grid", "data-text": props.document.records[1]?.fields[1]?.value }, [
        h("button", { class: "edit-cell", onClick: () => emit("edit", [{ row: 1, column: 1, value: "changed" }]) }, "edit"),
        h("button", { class: "extend-cell", onClick: () => emit("edit", [{ row: 8, column: 6, value: "extended" }]) }, "extend"),
        h("button", { class: "resize-column", onClick: () => {
          const id = props.view.columnOrder[0];
          emit("viewChange", { ...props.view, columns: { ...props.view.columns, [id]: { ...props.view.columns[id], width: 220 } } });
        } }, "resize"),
      ]);
    },
  }) };
});
vi.mock("../components/ui/BaseMarkdownEditor.vue", async () => {
  const { defineComponent, h } = await import("vue");
  return { default: defineComponent({ props: ["modelValue", "disabled"], setup(props) {
    return () => h("textarea", { class: "test-source", value: props.modelValue, disabled: props.disabled });
  } }) };
});

function preview(text: string, hash = "v1"): ProjectExplorerFilePreview {
  return { path: "Assets/items.csv", name: "items.csv", extension: "csv", size: text.length, kind: "text", text,
    contentHash: hash, editable: true, truncated: false, mimeType: "text/csv",
    revision: { key: hash, exists: true, size: text.length, modifiedAtNanos: hash } };
}
const apps: App[] = [];
function emitFile(path = "Assets/items.csv") {
  for (const listener of mocks.fileListeners) listener({ checkoutId: "csv-test", workspaceGeneration: 1, payload: { path } } as never);
}
async function flush() { for (let index = 0; index < 12; index++) { await Promise.resolve(); await nextTick(); } }
async function mount(onReady?: () => void) {
  const host = document.createElement("div"); document.body.appendChild(host);
  const editor = ref<InstanceType<typeof WorkspaceCsvEditor> | null>(null);
  const changes: boolean[] = [];
  const app = createApp({ setup: () => () => h(WorkspaceCsvEditor, {
    ref: editor, path: "Assets/items.csv", workspaceRef: { checkoutId: "csv-test", expectedGeneration: 1 },
    onDirtyChange: (dirty: boolean) => changes.push(dirty),
    onReady,
  }) });
  apps.push(app); app.mount(host); await flush();
  return { host, editor, changes, app };
}
beforeEach(() => {
  mocks.read.mockResolvedValue(preview('\ufeffid,name,\r\n001,"original",\r\n\r\n'));
  mocks.write.mockImplementation(async (_path: string, text: string) => preview(text, "v2"));
  mocks.readView.mockResolvedValue({ text: null, contentHash: null });
  mocks.writeView.mockImplementation(async (_scope: unknown, text: string) => ({ text, contentHash: "view-v2" }));
  mocks.revision.mockResolvedValue(preview("").revision);
});
afterEach(() => {
  for (const app of apps.splice(0)) app.unmount();
  csvEditorSessions.clear(); document.body.innerHTML = ""; vi.useRealTimers(); vi.resetAllMocks();
  mocks.fileListeners.clear();
});

describe("CSV file editor", () => {
  it("waits for both file content and the saved view before reporting ready", async () => {
    let finishView!: (value: { text: null; contentHash: null }) => void;
    mocks.readView.mockReturnValueOnce(new Promise((resolve) => { finishView = resolve; }));
    const ready = vi.fn();
    const { host } = await mount(ready);
    expect(ready).not.toHaveBeenCalled();
    finishView({ text: null, contentHash: null });
    await flush();
    expect(ready).toHaveBeenCalledOnce();
    expect(host.querySelector(".test-grid")).not.toBeNull();
  });

  it("reports a settled failed read so the workbench can display the error", async () => {
    mocks.read.mockRejectedValueOnce(new Error("read failed"));
    const ready = vi.fn();
    const { host } = await mount(ready);
    expect(ready).toHaveBeenCalledOnce();
    expect(host.querySelector('[role="alert"]')?.textContent).toContain("read failed");
  });

  it("commits focused inline cell text before saving the file", async () => {
    const { host, editor } = await mount();
    const input = document.createElement("span");
    input.className = "csv-cell-input"; input.setAttribute("contenteditable", "plaintext-only"); input.tabIndex = 0;
    input.addEventListener("blur", () => host.querySelector<HTMLButtonElement>(".edit-cell")!.click());
    host.querySelector(".test-grid")!.appendChild(input); input.focus();
    await expect(editor.value!.saveFile()).resolves.toBe(true);
    expect(mocks.write).toHaveBeenCalledTimes(1);
    expect(mocks.write.mock.calls[0]![1]).toContain('"changed"');
  });
  it("rechecks an event received before the initial file load finishes", async () => {
    vi.useFakeTimers();
    let finish!: (next: ProjectExplorerFilePreview) => void;
    mocks.read.mockReturnValueOnce(new Promise((resolve) => { finish = resolve; }));
    const { editor } = await mount();
    const remote = preview("id,name\n001,remote\n", "remote");
    mocks.revision.mockResolvedValue(remote.revision); mocks.read.mockResolvedValue(remote);
    emitFile(); await vi.advanceTimersByTimeAsync(80);
    finish(preview("id,name\n001,original\n")); await flush();
    await vi.advanceTimersByTimeAsync(80); await flush();
    expect(mocks.read).toHaveBeenCalledTimes(2);
    expect(editor.value!.exportTransferSnapshot()).toMatchObject({ text: remote.text });
  });

  it("does not turn a save notification into a conflict before its acknowledgement", async () => {
    vi.useFakeTimers();
    const { editor, host } = await mount();
    host.querySelector<HTMLButtonElement>(".edit-cell")!.click(); await flush();
    let finish!: (next: ProjectExplorerFilePreview) => void;
    mocks.write.mockReturnValueOnce(new Promise((resolve) => { finish = resolve; }));
    const saving = editor.value!.saveFile(); await flush();
    const saved = preview((editor.value!.exportTransferSnapshot() as { text: string }).text, "saved");
    mocks.revision.mockResolvedValue(saved.revision);
    emitFile(); await vi.advanceTimersByTimeAsync(80);
    expect(host.querySelector(".csv-message")).toBeNull();
    finish(saved); await saving; await vi.advanceTimersByTimeAsync(80); await flush();
    expect(mocks.read).toHaveBeenCalledTimes(1);
    expect(host.querySelector(".csv-message")).toBeNull();
    expect(mocks.write).toHaveBeenCalledTimes(1);
  });

  it("coalesces CSV and sidecar notifications into one refresh with no writeback", async () => {
    vi.useFakeTimers();
    const { editor, host } = await mount();
    const before = editor.value!.exportTransferSnapshot();
    if (before.kind !== "workspaceFile" || !before.csv) throw new Error("Expected CSV snapshot");
    const view = parseCsvView(before.csv.view);
    view.schema = "locus.csv-view.v2";
    view.styles = [{ id: "header", rows: [0, 0], style: { bold: true } }];
    const remote = preview("id,name,\n001,remote,\n\n", "remote");
    mocks.read.mockResolvedValue(remote); mocks.revision.mockResolvedValue(remote.revision);
    mocks.readView.mockResolvedValue({ text: serializeCsvView(view), contentHash: "remote-view" });
    for (let i = 0; i < 20; i++) { emitFile(); emitFile("Assets/items.csv.view"); }
    await vi.advanceTimersByTimeAsync(80); await flush();
    expect(mocks.read).toHaveBeenCalledTimes(2);
    expect(mocks.readView).toHaveBeenCalledTimes(2);
    expect(host.querySelector<HTMLElement>(".test-grid")?.dataset.text).toBe("remote");
    expect(editor.value!.exportTransferSnapshot()).toMatchObject({ text: remote.text, csv: { viewHash: "remote-view" } });
    await vi.advanceTimersByTimeAsync(1000);
    expect(mocks.write).not.toHaveBeenCalled(); expect(mocks.writeView).not.toHaveBeenCalled();
  });

  it("keeps default column identities on external reload and ignores metadata-only echoes", async () => {
    const { editor } = await mount();
    const before = editor.value!.exportTransferSnapshot();
    if (before.kind !== "workspaceFile" || !before.csv) throw new Error("Expected CSV snapshot");
    const remote = preview("id,name,\n001,remote,\n\n", "remote");
    mocks.read.mockResolvedValue(remote); mocks.revision.mockResolvedValue(remote.revision);
    await editor.value!.refreshIfChanged();
    const after = editor.value!.exportTransferSnapshot();
    if (after.kind !== "workspaceFile" || !after.csv) throw new Error("Expected CSV snapshot");
    expect(parseCsvView(after.csv.view).columnOrder).toEqual(parseCsvView(before.csv.view).columnOrder);
    const echo = { ...remote, revision: { ...remote.revision, key: "touched" } };
    mocks.read.mockResolvedValue(echo); mocks.revision.mockResolvedValue(echo.revision);
    await editor.value!.refreshIfChanged();
    expect(editor.value!.exportTransferSnapshot()).toMatchObject(after);
    expect(mocks.writeView).not.toHaveBeenCalled();
  });

  it("manual refresh checks formatting without reading CSV data again", async () => {
    const { editor } = await mount();
    const before = editor.value!.exportTransferSnapshot();
    if (before.kind !== "workspaceFile" || !before.csv) throw new Error("Expected CSV snapshot");
    const view = parseCsvView(before.csv.view); view.wrapText = true;
    mocks.readView.mockResolvedValue({ text: serializeCsvView(view), contentHash: "view-remote" });
    await editor.value!.refreshIfChanged();
    expect(mocks.read).toHaveBeenCalledTimes(1); expect(mocks.readView).toHaveBeenCalledTimes(2);
    expect(editor.value!.exportTransferSnapshot()).toMatchObject({ csv: { viewHash: "view-remote" } });
  });

  it("defers a remote update while a cell has uncommitted input", async () => {
    vi.useFakeTimers();
    const { editor, host } = await mount();
    const input = document.createElement("textarea"); input.className = "csv-cell-input";
    host.querySelector(".test-grid")!.append(input); input.focus(); input.value = "uncommitted";
    const remote = preview("id,name,\n001,remote,\n\n", "remote");
    mocks.read.mockResolvedValue(remote); mocks.revision.mockResolvedValue(remote.revision);
    await editor.value!.refreshIfChanged();
    expect(mocks.read).toHaveBeenCalledTimes(1);
    expect(input.value).toBe("uncommitted");
    expect(host.querySelector(".csv-message")).toBeNull();
    input.blur(); input.remove(); await flush(); await vi.advanceTimersByTimeAsync(80); await flush();
    expect(editor.value!.exportTransferSnapshot()).toMatchObject({ text: remote.text });
  });

  it("retains CSV and view drafts if a column is resized during a remote read", async () => {
    const { editor, host } = await mount();
    const before = editor.value!.exportTransferSnapshot();
    const remote = preview("id,name,\n001,remote,\n\n", "remote");
    let finish!: (next: ProjectExplorerFilePreview) => void;
    mocks.read.mockReturnValueOnce(new Promise((resolve) => { finish = resolve; }));
    mocks.revision.mockResolvedValue(remote.revision);
    const refresh = editor.value!.refreshIfChanged(); await flush();
    host.querySelector<HTMLButtonElement>(".resize-column")!.click(); await flush();
    finish(remote); await refresh;
    expect(editor.value!.exportTransferSnapshot()).toMatchObject({ text: (before as { text: string }).text, contentHash: "v1" });
    expect(host.querySelector(".csv-message")).not.toBeNull();
    expect(mocks.writeView).not.toHaveBeenCalled();
  });
  it("opens CSV without creating a view file or changing data", async () => {
    vi.useFakeTimers();
    const { host } = await mount();
    expect(host.querySelector(".test-grid")).not.toBeNull();
    await vi.advanceTimersByTimeAsync(1000);
    expect(mocks.write).not.toHaveBeenCalled(); expect(mocks.writeView).not.toHaveBeenCalled();
  });
  it("persists only YAML when a column width changes", async () => {
    vi.useFakeTimers();
    const { host } = await mount();
    host.querySelector<HTMLButtonElement>(".resize-column")!.click(); await flush();
    await vi.advanceTimersByTimeAsync(700); await flush();
    expect(mocks.write).not.toHaveBeenCalled();
    expect(mocks.writeView).toHaveBeenCalledTimes(1);
    const call = mocks.writeView.mock.calls[0]!;
    expect(call[2]).toBeNull(); expect(call[3]).toBe("v1");
    expect(Object.values(parseCsvView(call[1]).columns).some((column) => column.width === 220)).toBe(true);
  });
  it("saves edited data with the loaded revision and preserves lexical whitespace", async () => {
    const { host, editor, changes } = await mount();
    host.querySelector<HTMLButtonElement>(".edit-cell")!.click(); await flush();
    expect(changes[changes.length - 1]).toBe(true);
    await expect(editor.value!.saveFile()).resolves.toBe(true);
    expect(mocks.write).toHaveBeenCalledWith("Assets/items.csv", '\ufeffid,name,\r\n001,"changed",\r\n\r\n', "v1",
      { checkoutId: "csv-test", expectedGeneration: 1 });
    expect(mocks.writeView).not.toHaveBeenCalled();
  });
  it("does not create a view file merely because typing extends CSV columns", async () => {
    const { host, editor } = await mount();
    host.querySelector<HTMLButtonElement>(".extend-cell")!.click(); await flush();
    await expect(editor.value!.saveFile()).resolves.toBe(true);
    expect(mocks.write.mock.calls[0]![1]).toContain(",,,,,,extended");
    expect(mocks.writeView).not.toHaveBeenCalled();
  });
  it("keeps data saved and view dirty when the sidecar write fails", async () => {
    const { host, editor, changes } = await mount();
    host.querySelector<HTMLButtonElement>(".edit-cell")!.click();
    host.querySelector<HTMLButtonElement>(".resize-column")!.click(); await flush();
    mocks.writeView.mockRejectedValueOnce(new Error("Sidecar write denied"));
    await expect(editor.value!.saveFile()).resolves.toBe(false);
    expect(mocks.write).toHaveBeenCalledTimes(1);
    expect(changes[changes.length - 1]).toBe(true);
    expect(host.textContent).toContain("Sidecar write denied");
    await expect(editor.value!.saveFile()).resolves.toBe(true);
    expect(mocks.write).toHaveBeenCalledTimes(1);
    expect(mocks.writeView).toHaveBeenCalledTimes(2);
  });
  it("retains a malformed view and allows CSV edits without overwriting the view", async () => {
    vi.useFakeTimers(); mocks.readView.mockResolvedValue({ text: "schema: other\n", contentHash: "bad" });
    const { host, editor } = await mount();
    expect(host.querySelector(".csv-message")).not.toBeNull();
    host.querySelector<HTMLButtonElement>(".resize-column")!.click();
    host.querySelector<HTMLButtonElement>(".edit-cell")!.click(); await flush();
    await editor.value!.saveFile(); await vi.advanceTimersByTimeAsync(1000);
    expect(mocks.write).toHaveBeenCalledTimes(1); expect(mocks.writeView).not.toHaveBeenCalled();
  });
  it("blocks overwriting externally changed data until the conflict is resolved", async () => {
    const { host, editor } = await mount();
    host.querySelector<HTMLButtonElement>(".edit-cell")!.click(); await flush();
    mocks.revision.mockResolvedValue(preview("different", "remote").revision);
    await editor.value!.refreshIfChanged(); await flush();
    await expect(editor.value!.saveFile()).resolves.toBe(false);
    expect(mocks.write).not.toHaveBeenCalled();
    expect(host.querySelector(".csv-message")).not.toBeNull();
  });
  it("reloads SDK formatting in a clean editor without writing back either file", async () => {
    vi.useFakeTimers();
    const { editor } = await mount();
    const before = editor.value!.exportTransferSnapshot();
    if (before.kind !== "workspaceFile" || !before.csv) throw new Error("Expected CSV snapshot");
    const view = parseCsvView(before.csv.view);
    view.schema = "locus.csv-view.v2";
    view.styles = [{ id: "header", rows: [0, 0], style: { bold: true, background: "subtle" } }];
    mocks.readView.mockResolvedValue({ text: serializeCsvView(view), contentHash: "sdk-v2" });
    window.dispatchEvent(new Event("focus"));
    await vi.advanceTimersByTimeAsync(200); await flush();
    const after = editor.value!.exportTransferSnapshot();
    if (after.kind !== "workspaceFile" || !after.csv) throw new Error("Expected CSV snapshot");
    expect(after.csv.viewHash).toBe("sdk-v2");
    expect(parseCsvView(after.csv.view).styles).toEqual(view.styles);
    expect(after.text).toBe(before.text);
    await vi.advanceTimersByTimeAsync(1000);
    expect(mocks.write).not.toHaveBeenCalled(); expect(mocks.writeView).not.toHaveBeenCalled();
  });
  it("preserves a dirty CSV draft when the SDK writes a new view", async () => {
    vi.useFakeTimers();
    const { editor, host } = await mount();
    const before = editor.value!.exportTransferSnapshot();
    if (before.kind !== "workspaceFile" || !before.csv) throw new Error("Expected CSV snapshot");
    const view = parseCsvView(before.csv.view);
    view.schema = "locus.csv-view.v2";
    view.styles = [{ id: "all", style: { size: 16 } }];
    host.querySelector<HTMLButtonElement>(".edit-cell")!.click(); await flush();
    mocks.readView.mockResolvedValue({ text: serializeCsvView(view), contentHash: "sdk-v2" });
    window.dispatchEvent(new Event("focus"));
    await vi.advanceTimersByTimeAsync(1000); await flush();
    const after = editor.value!.exportTransferSnapshot();
    if (after.kind !== "workspaceFile" || !after.csv) throw new Error("Expected CSV snapshot");
    expect(after.text).toContain("changed");
    expect(after.csv.view).toBe(before.csv.view);
    expect(host.querySelector(".csv-message")).not.toBeNull();
    expect(mocks.write).not.toHaveBeenCalled(); expect(mocks.writeView).not.toHaveBeenCalled();
  });
  it("restores dirty drafts on remount and removes them when the user discards", async () => {
    const first = await mount();
    first.host.querySelector<HTMLButtonElement>(".edit-cell")!.click(); await flush();
    first.app.unmount(); apps.splice(apps.indexOf(first.app), 1);
    const second = await mount();
    expect(second.editor.value!.exportTransferSnapshot().kind).toBe("workspaceFile");
    expect((second.editor.value!.exportTransferSnapshot() as { text: string }).text).toContain('"changed"');
    second.editor.value!.discardChanges(); second.app.unmount(); apps.splice(apps.indexOf(second.app), 1);
    const third = await mount();
    expect((third.editor.value!.exportTransferSnapshot() as { text: string }).text).toContain('"original"');
  });
  it("transfers unsaved CSV and YAML together and rejects a changed disk baseline", async () => {
    const first = await mount();
    first.host.querySelector<HTMLButtonElement>(".edit-cell")!.click();
    first.host.querySelector<HTMLButtonElement>(".resize-column")!.click(); await flush();
    const snapshot = first.editor.value!.exportTransferSnapshot();
    first.editor.value!.discardChanges(); first.app.unmount(); apps.splice(apps.indexOf(first.app), 1);
    const second = await mount();
    await expect(second.editor.value!.applyTransferSnapshot(snapshot)).resolves.toBe(true);
    const restored = second.editor.value!.exportTransferSnapshot();
    expect(restored).toMatchObject(snapshot);
    if (snapshot.kind !== "workspaceFile") throw new Error("Expected file snapshot");
    await expect(second.editor.value!.applyTransferSnapshot({ ...snapshot, contentHash: "other-revision" })).resolves.toBe(false);
    await expect(second.editor.value!.saveFile()).resolves.toBe(false);
  });
});
