// @vitest-environment jsdom
import { createApp, h, nextTick, type App } from "vue";
import { afterEach, describe, expect, it, vi } from "vitest";
import FileHistoryPreviewComponent from "../components/explorer/FileHistoryPreview.vue";
import type { FileHistoryPreview } from "../services/fileHistory";

vi.mock("../components/ui/BaseMarkdownEditor.vue", () => ({ __esModule: true, default: { props: { modelValue: String, disabled: Boolean }, setup: (p: { modelValue: string; disabled: boolean }) => () => h("article", { "data-readonly": p.disabled }, p.modelValue) } }));
vi.mock("../components/csv/CsvGrid.vue", () => ({ __esModule: true, default: { props: { document: Object, readOnly: Boolean }, setup: (p: { readOnly: boolean }) => () => h("table", { "data-readonly": p.readOnly }) } }));
vi.mock("../components/asset/AssetTextViewer.vue", () => ({ default: { props: ["snippet"], setup: (p: { snippet: string }) => () => h("pre", p.snippet) } }));
vi.mock("../components/asset/AssetPreviewHost.vue", () => ({ default: { props: ["payload"], setup: (p: { payload: { kind: string } }) => () => h("div", { "data-asset": p.payload.kind }) } }));
let app: App | null = null;
afterEach(() => { app?.unmount(); document.body.innerHTML = ""; });
async function mount(extension: string, kind: FileHistoryPreview["file"]["kind"] = "text", asset: FileHistoryPreview["asset"] = null) {
  const preview: FileHistoryPreview = { contentHash: "snapshot", csvView: null, asset, file: {
    path: `note.${extension}`, name: `note.${extension}`, extension, kind, text: extension === "csv" ? "a,b\n1,2\n" : "# Snapshot", size: 10,
    mimeType: "text/plain", blobUrl: "http://locus-binary.localhost/blob/snapshot", truncated: false, editable: false,
    revision: { exists: true, size: 10, modifiedAtNanos: "1", key: "1" },
  } };
  const host = document.createElement("div"); document.body.appendChild(host);
  app = createApp(FileHistoryPreviewComponent, { preview, workspaceRef: null }); app.mount(host);
  await vi.waitFor(() => expect(host.querySelector("article,table,pre,iframe,audio,video,[data-asset]")).not.toBeNull());
  await nextTick(); return host;
}
describe("history snapshot preview selection", () => {
  it("renders Markdown as a read-only document", async () => {
    const article = (await mount("md")).querySelector("article");
    expect(article?.textContent).toBe("# Snapshot");
    expect(article?.getAttribute("data-readonly")).toBe("true");
  });
  it("renders CSV with the read-only spreadsheet grid", async () => { expect((await mount("csv")).querySelector("table")?.getAttribute("data-readonly")).toBe("true"); });
  it("renders source code with the existing text viewer", async () => { expect((await mount("ts")).querySelector("pre")?.textContent).toBe("# Snapshot"); });
  it.each([["pdf", "iframe"], ["audio", "audio"], ["video", "video"]] as const)("renders %s with its media viewer", async (kind, selector) => { expect((await mount(kind, kind)).querySelector(selector)).not.toBeNull(); });
  it("uses the asset viewer for image snapshots", async () => {
    const host = await mount("png", "image", { kind: "binaryPreview", preview: { kind: "image", before: { url: "snapshot.png", byteSize: 12 } }, meta: { path: "note.png", name: "note.png", size: 12, ext: "png" } });
    expect(host.querySelector('[data-asset="binaryPreview"]')).not.toBeNull();
  });
  it("uses the structured asset viewer for Unity snapshots", async () => {
    const host = await mount("prefab", "text", { kind: "structured", previewKey: "snapshot", layout: "sceneHierarchyInspector", tree: [], targets: [] });
    expect(host.querySelector('[data-asset="structured"]')).not.toBeNull();
  });
});
