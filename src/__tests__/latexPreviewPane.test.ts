// @vitest-environment jsdom
import { createApp, h, nextTick, reactive, type App } from "vue";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { LatexPreviewSnapshot } from "../services/latexPreview";

const mocks = vi.hoisted(() => ({
  open: vi.fn(),
  setVisible: vi.fn(),
  close: vi.fn(),
  rebuild: vi.fn(),
  subscribe: vi.fn(),
  engine: vi.fn(),
  unavailable: vi.fn(),
  listener: null as null | ((snapshot: LatexPreviewSnapshot) => void),
}));
vi.mock("../i18n", () => ({ t: (key: string, ...args: unknown[]) => [key, ...args].join("|") }));
vi.mock("../services/latexPreview", () => ({
  openLatexPreview: mocks.open,
  setLatexPreviewVisible: mocks.setVisible,
  closeLatexPreview: mocks.close,
  rebuildLatexPreview: mocks.rebuild,
  subscribeLatexPreviewUpdated: mocks.subscribe,
  latexEngineAvailable: mocks.engine,
}));

import LatexPreviewPane from "../components/workbench/LatexPreviewPane.vue";

const workspaceRef = { checkoutId: "c1", expectedGeneration: 1, expectedMaterializationEpoch: 0 };

function snapshot(overrides: Partial<LatexPreviewSnapshot> = {}): LatexPreviewSnapshot {
  return {
    docId: "doc1",
    checkoutId: "c1",
    path: "F:/paper/main.tex",
    rootPath: "F:/paper/main.tex",
    status: "ready",
    mode: "svg",
    revision: 1,
    pages: [{ url: "http://locus-binary.localhost/latex/doc1/1/p1.svg", width: 612, height: 792 }],
    pdfUrl: "http://locus-binary.localhost/latex/doc1/1/document.pdf",
    pdfRevision: 1,
    fallbackReason: null,
    fallbackDetail: null,
    engine: "xelatex",
    errors: [],
    warnings: [],
    svgMs: 900,
    pdfMs: 1200,
    building: false,
    updatedAt: 0,
    ...overrides,
  };
}

// jsdom never loads images; resolve preloads on the next tick.
class LoadingImage {
  onload: (() => void) | null = null;
  onerror: (() => void) | null = null;
  set src(_value: string) {
    setTimeout(() => this.onload?.(), 0);
  }
}

let app: App | null = null;
let host: HTMLElement;
const props = reactive({
  workspaceRef: workspaceRef as typeof workspaceRef | null,
  path: "F:/paper/main.tex",
  active: true,
  autoFallback: false,
});

async function flush() {
  for (let i = 0; i < 4; i += 1) {
    await new Promise((resolve) => setTimeout(resolve, 0));
    await nextTick();
  }
}

function mount() {
  host = document.createElement("div");
  document.body.append(host);
  app = createApp({ render: () => h(LatexPreviewPane, { ...props, onUnavailable: mocks.unavailable }) });
  app.mount(host);
}

beforeEach(() => {
  vi.resetAllMocks();
  vi.stubGlobal("Image", LoadingImage);
  props.workspaceRef = workspaceRef;
  props.active = true;
  props.autoFallback = false;
  mocks.engine.mockResolvedValue(true);
  mocks.open.mockResolvedValue({ subscriptionId: "s1", snapshot: snapshot() });
  mocks.setVisible.mockImplementation(async () => snapshot());
  mocks.close.mockResolvedValue(undefined);
  mocks.subscribe.mockImplementation(async (listener: (value: LatexPreviewSnapshot) => void) => {
    mocks.listener = listener;
    return () => { mocks.listener = null; };
  });
});

afterEach(() => {
  app?.unmount();
  app = null;
  host?.remove();
  vi.unstubAllGlobals();
});

describe("LaTeX preview pane", () => {
  it("opens visible and shows the SVG pages, then follows updates", async () => {
    mount();
    await flush();
    expect(mocks.open).toHaveBeenCalledWith(workspaceRef, "F:/paper/main.tex", true);
    const pages = host.querySelectorAll<HTMLImageElement>("img.latex-preview-page");
    expect(pages).toHaveLength(1);
    expect(pages[0].getAttribute("src")).toContain("/doc1/1/p1.svg");
    expect(host.textContent).toContain("SVG");

    mocks.listener?.(snapshot({
      revision: 2,
      pages: [
        { url: "http://locus-binary.localhost/latex/doc1/2/p1.svg" },
        { url: "http://locus-binary.localhost/latex/doc1/2/p2.svg" },
      ],
    }));
    await flush();
    const updated = host.querySelectorAll<HTMLImageElement>("img.latex-preview-page");
    expect(updated).toHaveLength(2);
    expect(updated[1].getAttribute("src")).toContain("/doc1/2/p2.svg");
  });

  it("shows the PDF with the fallback reason when SVG is unsupported", async () => {
    mocks.open.mockResolvedValue({
      subscriptionId: "s1",
      snapshot: snapshot({ mode: "pdf", pages: [], fallbackReason: "package:pdfpages" }),
    });
    mount();
    await flush();
    const frame = host.querySelector<HTMLIFrameElement>("iframe.latex-preview-pdf");
    expect(frame?.getAttribute("src")).toContain("document.pdf");
    expect(host.querySelector(".latex-preview-fallback")?.textContent).toContain("latexPreview.fallback.package|pdfpages");
    expect(host.querySelector("img.latex-preview-page")).toBeNull();
  });

  it("lists build errors with their location next to the last output", async () => {
    mocks.open.mockResolvedValue({
      subscriptionId: "s1",
      snapshot: snapshot({
        status: "error",
        errors: [{ severity: "error", file: "F:/paper/main.tex", line: 12, message: "Undefined control sequence.", context: "\\foo" }],
      }),
    });
    mount();
    await flush();
    const diagnostics = host.querySelector(".latex-preview-diagnostics.error")?.textContent ?? "";
    expect(diagnostics).toContain("main.tex:12");
    expect(diagnostics).toContain("Undefined control sequence.");
    expect(host.querySelectorAll("img.latex-preview-page")).toHaveLength(1);
  });

  it("reports visibility changes and closes on unmount", async () => {
    mount();
    await flush();
    props.active = false;
    await flush();
    expect(mocks.setVisible).toHaveBeenLastCalledWith("s1", false);
    props.active = true;
    await flush();
    expect(mocks.setVisible).toHaveBeenLastCalledWith("s1", true);
    app!.unmount();
    app = null;
    await flush();
    expect(mocks.close).toHaveBeenCalledWith("s1");
  });

  it("opens a hidden pane without requesting a build", async () => {
    props.active = false;
    mount();
    await flush();
    expect(mocks.open).toHaveBeenCalledWith(workspaceRef, "F:/paper/main.tex", false);
  });

  it("falls back to source without opening when no TeX engine is installed", async () => {
    props.autoFallback = true;
    mocks.engine.mockResolvedValue(false);
    mount();
    await flush();
    expect(mocks.unavailable).toHaveBeenCalledTimes(1);
    expect(mocks.open).not.toHaveBeenCalled();

    // With an engine the same pane opens normally.
    app!.unmount();
    app = null;
    mocks.engine.mockResolvedValue(true);
    mount();
    await flush();
    expect(mocks.open).toHaveBeenCalledTimes(1);
  });

  it("opens on demand without any switch, also outside a workspace", async () => {
    props.workspaceRef = null;
    mount();
    await flush();
    expect(mocks.open).toHaveBeenCalledWith(null, "F:/paper/main.tex", true);
    expect(host.querySelectorAll("img.latex-preview-page")).toHaveLength(1);
  });
});
