// @vitest-environment jsdom
import { createApp, defineComponent, h, nextTick, type App } from "vue";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import WorkspaceFilePreview from "../components/workbench/WorkspaceFilePreview.vue";
import type { ProjectExplorerFilePreview } from "../types/workbench";

const files = vi.hoisted(() => ({
  read: vi.fn(),
  revision: vi.fn(),
  release: vi.fn(),
  viewers: [] as Array<{ emit: (event: "loaded" | "fetchError") => void }>,
}));
vi.mock("../services/workspaceExplorer", () => ({
  projectExplorerPreviewFile: files.read,
  workspaceFilePreview: files.read,
  projectExplorerWriteFile: vi.fn(),
  workspaceFileWrite: vi.fn(),
  projectExplorerFileRevision: files.revision,
  workspaceFileRevision: files.revision,
  subscribeWorkspaceFileChanges: vi.fn(async () => () => {}),
  releasePreviewBlob: files.release,
}));
// WebGL is unavailable in jsdom; the viewer itself is covered by modelViewer.test.ts.
vi.mock("../components/model-preview/ModelViewer.vue", () => ({
  default: defineComponent({
    props: { url: String, format: String, active: Boolean },
    emits: ["loaded", "fetchError"],
    setup(props, { emit }) {
      files.viewers.push({ emit: (event) => emit(event) });
      return () => h("div", {
        "data-model-stub": "",
        "data-url": props.url,
        "data-format": props.format,
        "data-active": String(props.active),
      });
    },
  }),
}));

const BLOB = "http://locus-binary.localhost/blob/";
let app: App<Element> | null = null;

function modelPreview(overrides: Partial<ProjectExplorerFilePreview> = {}): ProjectExplorerFilePreview {
  return {
    path: "F:/Project/Models/House.glb",
    name: "House.glb",
    extension: "glb",
    size: 112004,
    kind: "model",
    mimeType: "model/gltf-binary",
    blobUrl: `${BLOB}a`,
    truncated: false,
    editable: false,
    revision: { exists: true, size: 112004, modifiedAtNanos: "1", key: "v1" },
    ...overrides,
  };
}

async function flush() {
  for (let index = 0; index < 8; index += 1) {
    await Promise.resolve();
    await nextTick();
  }
}

async function mountPreview(path: string, active = true) {
  const host = document.createElement("div");
  document.body.appendChild(host);
  app = createApp({ render: () => h(WorkspaceFilePreview, { projectId: "project", path, active }) });
  app.mount(host);
  await flush();
  return host;
}

const viewerUrl = (host: HTMLElement) => host.querySelector<HTMLElement>("[data-model-stub]")?.dataset.url;

beforeEach(() => {
  files.read.mockReset();
  files.release.mockReset().mockResolvedValue(true);
  files.viewers.length = 0;
  files.revision.mockReset().mockResolvedValue(modelPreview().revision);
});

afterEach(() => {
  app?.unmount();
  app = null;
  document.body.replaceChildren();
});

describe("workspace model file preview", () => {
  it("fetches the model by blob URL, with the format implied by the file name", async () => {
    files.read.mockResolvedValue(modelPreview());
    const host = await mountPreview("Models/House.glb");

    const viewer = host.querySelector<HTMLElement>("[data-model-stub]");
    expect(viewer?.dataset.url).toBe(`${BLOB}a`);
    expect(viewer?.dataset.format).toBe("glb");
    expect(viewer?.dataset.active).toBe("true");
    expect(host.querySelector(".workspace-binary-preview")).toBeNull();
  });

  it("recognises FBX and passes the inactive state through", async () => {
    files.read.mockResolvedValue(modelPreview({
      path: "F:/Project/Models/Hero.fbx", name: "Hero.fbx", extension: "fbx",
      mimeType: "application/octet-stream",
    }));
    const host = await mountPreview("Models/Hero.fbx", false);

    const viewer = host.querySelector<HTMLElement>("[data-model-stub]");
    expect(viewer?.dataset.format).toBe("fbx");
    expect(viewer?.dataset.active).toBe("false");
  });

  it("keeps the information card for models the backend did not hand out", async () => {
    files.read.mockResolvedValue(modelPreview({ kind: "binary", blobUrl: undefined }));
    const host = await mountPreview("Models/Huge.glb");

    expect(host.querySelector("[data-model-stub]")).toBeNull();
    expect(host.querySelector(".workspace-binary-preview")?.textContent).toContain("House.glb");
    expect(files.release).not.toHaveBeenCalled();
  });

  it("releases the blob when the preview closes", async () => {
    files.read.mockResolvedValue(modelPreview());
    await mountPreview("Models/House.glb");
    expect(files.release).not.toHaveBeenCalled();

    app!.unmount();
    app = null;

    expect(files.release).toHaveBeenCalledTimes(1);
    expect(files.release).toHaveBeenCalledWith(`${BLOB}a`);
  });

  it("asks for a fresh blob when the viewer finds its bytes gone, and frees the old one", async () => {
    files.read
      .mockResolvedValueOnce(modelPreview({ blobUrl: `${BLOB}a` }))
      .mockResolvedValueOnce(modelPreview({ blobUrl: `${BLOB}b` }));
    const host = await mountPreview("Models/House.glb");

    files.viewers[0].emit("fetchError");
    await flush();

    expect(files.read).toHaveBeenCalledTimes(2);
    expect(viewerUrl(host)).toBe(`${BLOB}b`);
    expect(files.release).toHaveBeenCalledTimes(1);
    expect(files.release).toHaveBeenCalledWith(`${BLOB}a`);
  });

  it("retries only once per failure streak so a broken source cannot loop", async () => {
    files.read.mockImplementation(async () => modelPreview({ blobUrl: `${BLOB}${files.read.mock.calls.length}` }));
    await mountPreview("Models/House.glb");

    files.viewers[0].emit("fetchError");
    await flush();
    files.viewers[0].emit("fetchError");
    await flush();
    expect(files.read).toHaveBeenCalledTimes(2);

    // A successful load ends the streak; the next failure may retry again.
    files.viewers[0].emit("loaded");
    files.viewers[0].emit("fetchError");
    await flush();
    expect(files.read).toHaveBeenCalledTimes(3);
  });

  it("hands the viewer a binary cache URL, never inline data", async () => {
    files.read.mockResolvedValue(modelPreview());
    await mountPreview("Models/House.glb");

    const reply = await files.read.mock.results[0].value as ProjectExplorerFilePreview;
    expect(reply.blobUrl).toMatch(/^http:\/\/locus-binary\.localhost\/blob\//);
    expect(JSON.stringify(reply)).not.toContain("base64");
  });
});

describe("workspace media file preview", () => {
  const mediaPreview = (kind: "image" | "pdf" | "audio" | "video", name: string, blob = "a") => modelPreview({
    kind, name, path: `F:/Project/${name}`, extension: name.split(".").pop()!, blobUrl: `${BLOB}${blob}`,
  });

  it.each([
    ["image", "photo.png", "img"],
    ["pdf", "paper.pdf", "iframe"],
    ["audio", "clip.mp3", "audio"],
    ["video", "clip.mp4", "video"],
  ] as const)("plays %s straight from its blob URL", async (kind, name, selector) => {
    files.read.mockResolvedValue(mediaPreview(kind, name));
    const host = await mountPreview(name);

    expect(host.querySelector(selector)?.getAttribute("src")).toBe(`${BLOB}a`);
    expect(host.querySelector("[data-model-stub]")).toBeNull();
    expect(host.querySelector(".workspace-binary-preview")).toBeNull();
  });

  it("shows no element for a media file whose bytes were not handed out", async () => {
    files.read.mockResolvedValue(modelPreview({ kind: "binary", name: "huge.mp4", blobUrl: undefined }));
    const host = await mountPreview("huge.mp4");

    expect(host.querySelector("video,audio,img,iframe")).toBeNull();
    expect(host.querySelector(".workspace-binary-preview")?.textContent).toContain("huge.mp4");
  });

  it("releases media bytes when the preview closes", async () => {
    files.read.mockResolvedValue(mediaPreview("video", "clip.mp4"));
    await mountPreview("clip.mp4");

    app!.unmount();
    app = null;

    expect(files.release).toHaveBeenCalledWith(`${BLOB}a`);
  });

  it("reads the file again once when an element cannot get its bytes, then frees the old blob", async () => {
    files.read
      .mockResolvedValueOnce(mediaPreview("image", "photo.png", "a"))
      .mockResolvedValueOnce(mediaPreview("image", "photo.png", "b"));
    const host = await mountPreview("photo.png");

    host.querySelector("img")!.dispatchEvent(new Event("error"));
    await flush();

    expect(files.read).toHaveBeenCalledTimes(2);
    expect(host.querySelector("img")?.getAttribute("src")).toBe(`${BLOB}b`);
    expect(files.release).toHaveBeenCalledWith(`${BLOB}a`);

    // A second failure in the same streak is not retried again.
    host.querySelector("img")!.dispatchEvent(new Event("error"));
    await flush();
    expect(files.read).toHaveBeenCalledTimes(2);

    // Once an element loaded, the next failure may retry.
    host.querySelector("img")!.dispatchEvent(new Event("load"));
    host.querySelector("img")!.dispatchEvent(new Event("error"));
    await flush();
    expect(files.read).toHaveBeenCalledTimes(3);
  });

  it("retries media elements the same way for audio and video", async () => {
    for (const [kind, name, tag, event] of [["audio", "a.mp3", "audio", "error"], ["video", "v.mp4", "video", "error"]] as const) {
      files.read.mockReset().mockResolvedValue(mediaPreview(kind, name));
      app?.unmount();
      app = null;
      document.body.replaceChildren();
      const host = await mountPreview(name);
      host.querySelector(tag)!.dispatchEvent(new Event(event));
      await flush();
      expect(files.read, name).toHaveBeenCalledTimes(2);
    }
  });
});
