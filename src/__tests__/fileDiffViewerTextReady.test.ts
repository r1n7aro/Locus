// @vitest-environment jsdom
import { createApp, h, nextTick, reactive, ref, type App } from "vue";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { DiffHunk, FileDiffPayload, TextDiff } from "../types";

vi.mock("../services/diff", () => ({
  diffSemanticTarget: vi.fn(),
  diffTextForLarge: vi.fn(),
  invalidateDiffCache: vi.fn(),
  parseDiffRequestKey: () => null,
}));
vi.mock("../services/git", () => ({ gitExecute: vi.fn() }));
vi.mock("../services/workspaceExplorer", () => ({
  subscribeWorkspaceFileChanges: vi.fn(async () => () => undefined),
}));
vi.mock("../components/diff/UnityHierarchyPane.vue", () => ({ default: { render: () => null } }));
vi.mock("../components/diff/UnityInspectorPane.vue", () => ({ default: { render: () => null } }));
vi.mock("../components/diff/BinaryPreviewHost.vue", () => ({ default: { render: () => null } }));

import FileDiffViewer from "../components/diff/FileDiffViewer.vue";
import { SYNC_TEXT_RENDER_LINE_LIMIT } from "../components/diff/fileDiffTextReady";

const frames: FrameRequestCallback[] = [];
const apps: App[] = [];

function textWithLines(count: number): TextDiff {
  const hunk = {
    header: "@@ -1 +1 @@",
    oldStart: 1,
    oldLines: 0,
    newStart: 1,
    newLines: count,
    lines: Array.from({ length: count }, (_, index) => ({ kind: "add", content: `line ${index}`, newLine: index + 1 })),
  } as unknown as DiffHunk;
  return { hunks: [hunk] };
}

function payloadFor(text: TextDiff): FileDiffPayload {
  return {
    key: "edit-0",
    filePath: "Assets/Player.cs",
    status: "M",
    isBinary: false,
    isLarge: false,
    contentState: { type: "normal" },
    stats: { additions: 0, deletions: 0, changedHunks: 1 },
    previewSummary: [],
    text,
  } as FileDiffPayload;
}

function mountViewer(props: Record<string, unknown>) {
  const host = document.createElement("div");
  const view = ref<{ toggleTextDisplayMode(): void } | null>(null);
  const app = createApp({ render: () => h(FileDiffViewer, { ...props, ref: view } as never) });
  app.mount(host);
  apps.push(app);
  return { host, view };
}

async function flushFrames() {
  await nextTick();
  while (frames.length) frames.shift()!(16);
  await nextTick();
}

beforeEach(() => {
  frames.length = 0;
  vi.stubGlobal("requestAnimationFrame", (callback: FrameRequestCallback) => {
    frames.push(callback);
    return frames.length;
  });
});

afterEach(() => {
  while (apps.length) apps.pop()!.unmount();
  vi.unstubAllGlobals();
});

describe("FileDiffViewer text readiness", () => {
  it("renders a small diff in its mounting render", () => {
    const { host } = mountViewer({ payload: payloadFor(textWithLines(3)), compact: true });

    expect(host.querySelector(".diff-loading")).toBeNull();
    expect(host.querySelectorAll(".diff-line")).toHaveLength(3);
  });

  it("gates a large diff once and renders it immediately when remounted", async () => {
    const text = textWithLines(SYNC_TEXT_RENDER_LINE_LIMIT + 50);
    const first = mountViewer({ payload: payloadFor(text), compact: true });
    expect(first.host.querySelector(".diff-loading")).not.toBeNull();

    await flushFrames();
    expect(first.host.querySelector(".diff-loading")).toBeNull();

    const second = mountViewer({ payload: payloadFor(text), compact: true });
    expect(second.host.querySelector(".diff-loading")).toBeNull();
    expect(second.host.querySelectorAll(".diff-line").length).toBeGreaterThan(0);
  });

  it("does not let a superseded loading gate reveal a newer large diff early", async () => {
    const props = reactive({ payload: payloadFor(textWithLines(SYNC_TEXT_RENDER_LINE_LIMIT + 10)), compact: true });
    const { host } = mountViewer(props);
    await nextTick();
    const staleFrames = frames.splice(0);

    props.payload = payloadFor(textWithLines(SYNC_TEXT_RENDER_LINE_LIMIT + 20));
    await nextTick();
    staleFrames.forEach((frame) => frame(16));
    await nextTick();
    expect(host.querySelector(".diff-loading")).not.toBeNull();

    await flushFrames();
    expect(host.querySelector(".diff-loading")).toBeNull();
  });

  it("keeps its view state when the parent passes an equal workspace reference", async () => {
    const props = reactive({
      payload: payloadFor(textWithLines(3)),
      workspaceRef: { checkoutId: "checkout", expectedGeneration: 2 },
    });
    const { host, view } = mountViewer(props);
    view.value!.toggleTextDisplayMode();
    await nextTick();
    expect(host.querySelector(".diff-text")?.classList.contains("side-by-side")).toBe(true);

    props.workspaceRef = { checkoutId: "checkout", expectedGeneration: 2 };
    await nextTick();
    expect(host.querySelector(".diff-text")?.classList.contains("side-by-side")).toBe(true);

    props.workspaceRef = { checkoutId: "checkout", expectedGeneration: 3 };
    await nextTick();
    expect(host.querySelector(".diff-text")?.classList.contains("unified")).toBe(true);
  });
});
