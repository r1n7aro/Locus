// @vitest-environment jsdom
import { createApp, h, nextTick, ref } from "vue";
import { afterEach, beforeEach, describe, expect, it, vi, type Mock } from "vitest";
import ModelViewer from "../components/model-preview/ModelViewer.vue";
import type { ModelPreviewEngine } from "../components/model-preview/modelPreviewEngine";
import { t } from "../i18n";

const engineMock = vi.hoisted(() => ({ create: vi.fn() }));
vi.mock("../components/model-preview/modelPreviewEngine", () => {
  class ModelPreviewError extends Error {
    constructor(readonly code: string, message: string) {
      super(message);
    }
  }
  return { createModelPreviewEngine: engineMock.create, ModelPreviewError };
});

const GLB = "data:model/gltf-binary;base64," + btoa("glTF\u0002\u0000\u0000\u0000");
const stats = { meshes: 2, vertices: 100, triangles: 1510, materials: 9, textures: 0, bones: 0, size: [5.92, 5.66, 5.03] as [number, number, number] };

type EngineMock = { [K in keyof ModelPreviewEngine]: Mock<ModelPreviewEngine[K]> };

function createEngine(overrides: Partial<EngineMock> = {}): EngineMock {
  return {
    load: vi.fn<ModelPreviewEngine["load"]>(async () => ({ stats, clips: [] })),
    setMaterialMode: vi.fn<ModelPreviewEngine["setMaterialMode"]>(),
    setWireframe: vi.fn<ModelPreviewEngine["setWireframe"]>(),
    setGridVisible: vi.fn<ModelPreviewEngine["setGridVisible"]>(),
    setPlaying: vi.fn<ModelPreviewEngine["setPlaying"]>(),
    selectClip: vi.fn<ModelPreviewEngine["selectClip"]>(),
    resetView: vi.fn<ModelPreviewEngine["resetView"]>(),
    setActive: vi.fn<ModelPreviewEngine["setActive"]>(),
    resize: vi.fn<ModelPreviewEngine["resize"]>(),
    dispose: vi.fn<ModelPreviewEngine["dispose"]>(),
    ...overrides,
  };
}

const flush = async () => {
  for (let index = 0; index < 6; index += 1) {
    await Promise.resolve();
    await nextTick();
  }
};

const cleanups: Array<() => void> = [];

function mountViewer(props: Record<string, unknown> = {}, handlers: Record<string, unknown> = {}) {
  const host = document.createElement("div");
  document.body.appendChild(host);
  const app = createApp({ render: () => h(ModelViewer, { url: GLB, ...props, ...handlers }) });
  app.mount(host);
  cleanups.push(() => { app.unmount(); host.remove(); });
  return { host, app };
}

beforeEach(() => {
  engineMock.create.mockReset();
  // jsdom has no layout; BaseDropdown scrolls the active option into view.
  Object.defineProperty(Element.prototype, "scrollIntoView", { configurable: true, value: vi.fn() });
});

afterEach(() => {
  cleanups.splice(0).forEach((cleanup) => cleanup());
  Reflect.deleteProperty(Element.prototype, "scrollIntoView");
});

describe("ModelViewer", () => {
  it("parses the decoded bytes with the sniffed format and shows model statistics", async () => {
    const engine = createEngine();
    engineMock.create.mockResolvedValue(engine);
    const onLoaded = vi.fn();
    const { host } = mountViewer({ format: "fbx" }, { onLoaded });
    await flush();

    expect(onLoaded).toHaveBeenCalledTimes(1);
    expect(engine.load).toHaveBeenCalledTimes(1);
    const [buffer, format] = engine.load.mock.calls[0];
    expect(buffer.byteLength).toBe(8);
    // The glTF signature wins over the extension hint.
    expect(format).toBe("glb");
    const text = host.querySelector(".model-viewer-stats")?.textContent ?? "";
    expect(text).toContain("2");
    expect(text).toContain("1,510");
    expect(text).toContain("5.92");
    expect(host.querySelector(".model-viewer-overlay")).toBeNull();
  });

  it("rejects content that is not a supported model", async () => {
    const engine = createEngine();
    engineMock.create.mockResolvedValue(engine);
    const { host } = mountViewer({ url: "data:application/octet-stream;base64," + btoa("not a model") });
    await flush();

    expect(engine.load).not.toHaveBeenCalled();
    expect(host.querySelector(".model-viewer-overlay.error")).not.toBeNull();
  });

  it("reports unreachable bytes so the owner can refresh the source", async () => {
    vi.stubGlobal("fetch", vi.fn(async () => ({ ok: false, status: 404 })));
    const onFetchError = vi.fn();
    const onLoaded = vi.fn();
    const { host } = mountViewer({ url: "http://locus-binary.localhost/blob/missing" }, { onFetchError, onLoaded });
    await flush();
    vi.unstubAllGlobals();

    expect(onFetchError).toHaveBeenCalledTimes(1);
    expect(onLoaded).not.toHaveBeenCalled();
    expect(engineMock.create).not.toHaveBeenCalled();
    expect(host.querySelector(".model-viewer-overlay.error")).not.toBeNull();
  });

  it("explains unsupported compression instead of a generic parse failure", async () => {
    const { ModelPreviewError } = await import("../components/model-preview/modelPreviewEngine");
    const engine = createEngine({
      load: vi.fn<ModelPreviewEngine["load"]>(async () => { throw new ModelPreviewError("unsupportedCompression", "DRACOLoader"); }),
    });
    engineMock.create.mockResolvedValue(engine);
    const { host } = mountViewer();
    await flush();

    const message = host.querySelector(".model-viewer-overlay.error")?.textContent ?? "";
    expect(message).toMatch(/Draco|压缩/);
    expect(message).not.toContain("DRACOLoader");
  });

  it("reports a WebGL failure when the engine cannot start", async () => {
    engineMock.create.mockRejectedValue(new Error("WebGL unavailable"));
    const { host } = mountViewer();
    await flush();

    expect(host.querySelector(".model-viewer-overlay.error")).not.toBeNull();
    expect(host.querySelector(".model-viewer-stats")).toBeNull();
  });

  it("drives the engine from the toolbar and releases it on unmount", async () => {
    const engine = createEngine({
      load: vi.fn<ModelPreviewEngine["load"]>(async () => ({
        stats,
        clips: [{ name: "Idle", duration: 1 }, { name: "", duration: 2 }],
      })),
    });
    engineMock.create.mockResolvedValue(engine);
    const { host, app } = mountViewer();
    await flush();

    const click = async (selector: string) => {
      host.querySelector<HTMLElement>(selector)!.click();
      await nextTick();
    };
    const toolButtons = () => Array.from(host.querySelectorAll<HTMLButtonElement>(".model-viewer-tool"));
    toolButtons()[1].click(); // wireframe
    toolButtons()[2].click(); // clay
    await nextTick();
    expect(engine.setWireframe).toHaveBeenLastCalledWith(true);
    expect(engine.setMaterialMode).toHaveBeenLastCalledWith("clay");

    await click(".model-viewer-tool.icon:not(.trailing)"); // pause
    expect(engine.setPlaying).toHaveBeenLastCalledWith(false);
    await click(".model-viewer-clip .base-dropdown-trigger");
    const clipOptions = Array.from(document.querySelectorAll<HTMLElement>('[role="option"]'));
    expect(clipOptions.map((option) => option.textContent?.trim())).toEqual(["Idle", t("model.preview.clipUnnamed", 2)]);
    clipOptions[1].click();
    await nextTick();
    expect(engine.selectClip).toHaveBeenCalledWith(1);
    await click(".model-viewer-tool.trailing");
    expect(engine.resetView).toHaveBeenCalled();

    app.unmount();
    expect(engine.dispose).toHaveBeenCalledTimes(1);
  });

  it("disposes an engine that finishes starting after the viewer is gone", async () => {
    const engine = createEngine();
    let resolveEngine!: (value: ModelPreviewEngine) => void;
    engineMock.create.mockReturnValue(new Promise<ModelPreviewEngine>((resolve) => { resolveEngine = resolve; }));
    const { app } = mountViewer();
    await flush();
    app.unmount();
    resolveEngine(engine);
    await flush();

    expect(engine.dispose).toHaveBeenCalledTimes(1);
    expect(engine.load).not.toHaveBeenCalled();
  });

  it("uses a single engine when the source changes while it is starting", async () => {
    const engine = createEngine();
    let resolveEngine!: (value: ModelPreviewEngine) => void;
    engineMock.create.mockReturnValue(new Promise<ModelPreviewEngine>((resolve) => { resolveEngine = resolve; }));
    const source = ref(GLB);
    const host = document.createElement("div");
    document.body.appendChild(host);
    const app = createApp({ render: () => h(ModelViewer, { url: source.value }) });
    app.mount(host);
    cleanups.push(() => { app.unmount(); host.remove(); });
    await flush();

    source.value = "data:model/gltf-binary;base64," + btoa("glTF\u0002\u0000\u0000\u0000\u0000");
    await flush();
    resolveEngine(engine);
    await flush();

    expect(engineMock.create).toHaveBeenCalledTimes(1);
    // Only the latest source is parsed; the superseded one is dropped.
    expect(engine.load).toHaveBeenCalledTimes(1);
    expect(engine.load.mock.calls[0][0].byteLength).toBe(9);
  });
});
