// @vitest-environment jsdom
import { createApp, h, nextTick, shallowReactive, type App } from "vue";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { DiffHunk, ToolCallDisplay } from "../types";

const mocks = vi.hoisted(() => ({
  diffStrings: vi.fn<(oldText: string, newText: string, contextLines?: number) => Promise<DiffHunk[]>>(),
  viewerMounts: 0,
}));

vi.mock("../services/diff", () => ({ diffStrings: mocks.diffStrings }));
vi.mock("../stores/project", () => ({
  useProjectStore: () => ({ workingDir: "F:/Game", extraWorkdirs: {} }),
}));
vi.mock("../stores/workspaceContext", () => ({
  useWorkspaceContextStore: () => ({ focusedWorkspaceRef: null }),
}));
vi.mock("../stores/notification", () => ({
  useNotificationStore: () => ({ addNotice: vi.fn() }),
}));
vi.mock("../components/MarkdownRenderer.vue", () => ({ default: { render: () => null } }));
vi.mock("../components/tool-block-overrides/toolBlockOverrides", () => ({
  resolveToolBlockOverride: () => null,
}));
vi.mock("../components/diff/FileDiffViewer.vue", async () => {
  const { defineComponent, h: render, onMounted } = await import("vue");
  return {
    default: defineComponent({
      props: { payload: { type: Object, required: true } },
      setup(props) {
        onMounted(() => { mocks.viewerMounts += 1; });
        return () => render("div", {
          class: "diff-viewer-stub",
          "data-summary": (props.payload as { previewSummary: string[] }).previewSummary[0],
        });
      },
    }),
  };
});

import ToolCallBlock from "../components/ToolCallBlock.vue";
import {
  loadEditDiffPayload,
  peekEditDiffPayload,
  resetEditDiffPayloadCacheForTest,
  sameEditDiffSources,
} from "../components/toolEditDiffPayloads";

function hunksFor(additions: number): DiffHunk[] {
  return [{
    header: "@@ -1 +1 @@",
    oldStart: 1,
    oldLines: 1,
    newStart: 1,
    newLines: additions,
    lines: [
      { kind: "delete", content: "old", oldLine: 1 },
      ...Array.from({ length: additions }, (_, index) => ({ kind: "add", content: `new ${index}`, newLine: index + 1 })),
    ],
  }] as unknown as DiffHunk[];
}

function editToolCall(newString = "new", id = "edit-call"): ToolCallDisplay {
  return {
    id,
    name: "edit",
    arguments: JSON.stringify({ filePath: "Assets/Player.cs", oldString: "old", newString }),
    output: "Edited Assets/Player.cs [lines:12]",
    status: "done",
  };
}

const apps: App[] = [];

function mountBlock(toolCall: ToolCallDisplay) {
  const state = shallowReactive({ toolCall });
  const host = document.createElement("div");
  const app = createApp({ render: () => h(ToolCallBlock, { toolCall: state.toolCall, initialExpanded: true }) });
  app.mount(host);
  apps.push(app);
  return { host, state };
}

async function flushDiffs() {
  for (let index = 0; index < 4; index += 1) {
    await Promise.resolve();
    await nextTick();
  }
}

beforeEach(() => {
  resetEditDiffPayloadCacheForTest();
  mocks.diffStrings.mockReset();
  mocks.viewerMounts = 0;
});

afterEach(() => {
  while (apps.length) apps.pop()!.unmount();
});

describe("edit diff payload cache", () => {
  it("shares one backend diff per edit content and exposes the finished payload", async () => {
    mocks.diffStrings.mockResolvedValue(hunksFor(1));
    const source = { filePath: "a.ts", oldStr: "old", newStr: "new", index: 0 };

    const first = loadEditDiffPayload(source);
    const second = loadEditDiffPayload({ ...source });
    expect(peekEditDiffPayload(source)).toBeNull();
    const [a, b] = await Promise.all([first, second]);

    expect(mocks.diffStrings).toHaveBeenCalledTimes(1);
    expect(a).toBe(b);
    expect(peekEditDiffPayload({ ...source })).toBe(a);
    expect(a).toMatchObject({ key: "edit-0", filePath: "a.ts", stats: { additions: 1, deletions: 1, changedHunks: 1 } });
  });

  it("does not cache failed diffs", async () => {
    mocks.diffStrings.mockRejectedValueOnce(new Error("ipc down")).mockResolvedValueOnce(hunksFor(1));
    const source = { filePath: "a.ts", oldStr: "old", newStr: "new", index: 0 };

    await expect(loadEditDiffPayload(source)).resolves.toBeNull();
    await expect(loadEditDiffPayload(source)).resolves.not.toBeNull();
    expect(mocks.diffStrings).toHaveBeenCalledTimes(2);
  });

  it("compares edit sources by content", () => {
    const a = [{ filePath: "a.ts", oldStr: "x", newStr: "y", index: 0 }];
    expect(sameEditDiffSources(a, [{ ...a[0]! }])).toBe(true);
    expect(sameEditDiffSources(a, [{ ...a[0]!, newStr: "z" }])).toBe(false);
    expect(sameEditDiffSources(a, [])).toBe(false);
  });
});

describe("ToolCallBlock edit diff stability", () => {
  it("keeps the mounted diff when an equal tool call object is passed again", async () => {
    mocks.diffStrings.mockResolvedValue(hunksFor(1));
    const { host, state } = mountBlock(editToolCall());
    await flushDiffs();
    const viewer = host.querySelector(".diff-viewer-stub");
    expect(viewer).not.toBeNull();

    state.toolCall = editToolCall();
    await nextTick();

    expect(host.querySelector(".edit-diff-container")).toBeNull();
    expect(host.querySelector(".diff-viewer-stub")).toBe(viewer);
    await flushDiffs();
    expect(host.querySelector(".diff-viewer-stub")).toBe(viewer);
    expect(mocks.viewerMounts).toBe(1);
    expect(mocks.diffStrings).toHaveBeenCalledTimes(1);
  });

  it("renders a remounted block's diff in its first render", async () => {
    mocks.diffStrings.mockResolvedValue(hunksFor(1));
    mountBlock(editToolCall());
    await flushDiffs();

    const { host } = mountBlock(editToolCall());

    expect(host.querySelector(".diff-viewer-stub")).not.toBeNull();
    expect(host.querySelector(".edit-diff-container")).toBeNull();
    expect(mocks.diffStrings).toHaveBeenCalledTimes(1);
  });

  it("keeps the previous diff until a changed edit is ready", async () => {
    mocks.diffStrings.mockResolvedValueOnce(hunksFor(1));
    const { host, state } = mountBlock(editToolCall("new"));
    await flushDiffs();
    expect(host.querySelector(".diff-viewer-stub")?.getAttribute("data-summary")).toBe("+1 -1");

    let finish!: (hunks: DiffHunk[]) => void;
    mocks.diffStrings.mockReturnValueOnce(new Promise((resolve) => { finish = resolve; }));
    state.toolCall = editToolCall("newer");
    await flushDiffs();

    expect(host.querySelector(".edit-diff-container")).toBeNull();
    expect(host.querySelector(".diff-viewer-stub")?.getAttribute("data-summary")).toBe("+1 -1");

    finish(hunksFor(2));
    await flushDiffs();
    expect(host.querySelector(".diff-viewer-stub")?.getAttribute("data-summary")).toBe("+2 -1");
  });

  it("ignores a superseded diff that finishes last", async () => {
    let finishFirst!: (hunks: DiffHunk[]) => void;
    mocks.diffStrings
      .mockReturnValueOnce(new Promise((resolve) => { finishFirst = resolve; }))
      .mockResolvedValueOnce(hunksFor(3));
    const { host, state } = mountBlock(editToolCall("first"));
    state.toolCall = editToolCall("second");
    await flushDiffs();
    expect(host.querySelector(".diff-viewer-stub")?.getAttribute("data-summary")).toBe("+3 -1");

    finishFirst(hunksFor(1));
    await flushDiffs();
    expect(host.querySelector(".diff-viewer-stub")?.getAttribute("data-summary")).toBe("+3 -1");
  });

  it("falls back to the plain edit view when the diff fails", async () => {
    mocks.diffStrings.mockRejectedValue(new Error("ipc down"));
    const { host } = mountBlock(editToolCall());
    await flushDiffs();

    expect(host.querySelector(".diff-viewer-stub")).toBeNull();
    expect(host.querySelector(".edit-diff-container")).not.toBeNull();
  });
});
