// @vitest-environment jsdom
import { createPinia } from "pinia";
import { createApp, h, nextTick, reactive, type App } from "vue";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { AssistantRenderPart, ChatMessage, DiffHunk, ToolCallDisplay, ToolCallInfo } from "../types";

const mocks = vi.hoisted(() => ({
  viewerMounts: 0,
  collectionUpdates: new Map<string, number>(),
  diffStrings: vi.fn(async () => [{
    header: "@@ -1 +1 @@", oldStart: 1, oldLines: 1, newStart: 1, newLines: 1,
    lines: [{ kind: "delete", content: "a", oldLine: 1 }, { kind: "add", content: "b", newLine: 1 }],
  }] as unknown as DiffHunk[]),
}));

vi.mock("../services/diff", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../services/diff")>()),
  diffStrings: mocks.diffStrings,
}));
vi.mock("../components/diff/FileDiffViewer.vue", async () => {
  const { defineComponent, h: render, onMounted } = await import("vue");
  return {
    default: defineComponent({
      props: { payload: { type: Object, required: true } },
      setup() {
        onMounted(() => { mocks.viewerMounts += 1; });
        return () => render("div", { class: "diff-viewer-stub" });
      },
    }),
  };
});
// Markdown segments expose the workspace they were rendered with.
vi.mock("../components/MarkdownRenderer.vue", async () => {
  const { defineComponent, h: render } = await import("vue");
  return {
    default: defineComponent({
      props: { content: { type: String, default: "" }, workspaceRef: { type: Object, default: null } },
      setup(props) {
        return () => render("div", {
          class: "markdown-stub",
          "data-checkout": (props.workspaceRef as { checkoutId?: string } | null)?.checkoutId ?? "",
        }, props.content);
      },
    }),
  };
});
vi.mock("../components/chat/StreamingMarkdownRenderer.vue", async () => {
  const { defineComponent, h: render } = await import("vue");
  return {
    default: defineComponent({
      props: { content: { type: String, default: "" }, workspaceRef: { type: Object, default: null } },
      setup(props) {
        return () => render("div", {
          class: "markdown-stub",
          "data-checkout": (props.workspaceRef as { checkoutId?: string } | null)?.checkoutId ?? "",
        }, props.content);
      },
    }),
  };
});
// Renders its tool blocks like the real collection and counts re-renders. The
// collection receives a scoped slot, so it re-renders whenever its group does.
vi.mock("../components/ToolCallCollection.vue", async () => {
  const { defineComponent, h: render, onUpdated } = await import("vue");
  return {
    default: defineComponent({
      props: { toolCalls: { type: Array, required: true } },
      setup(props, { slots }) {
        const ids = () => (props.toolCalls as ToolCallDisplay[]).map((toolCall) => toolCall.id).join(",");
        onUpdated(() => { mocks.collectionUpdates.set(ids(), (mocks.collectionUpdates.get(ids()) ?? 0) + 1); });
        return () => render(
          "div",
          { class: "collection-stub", "data-ids": ids() },
          (props.toolCalls as ToolCallDisplay[]).map((toolCall) => slots.default?.({ toolCall })),
        );
      },
    }),
  };
});

import ChatTranscript from "../components/chat/ChatTranscript.vue";
import { resetEditDiffPayloadCacheForTest } from "../components/toolEditDiffPayloads";

function toolMessage(id: string, toolCall: ToolCallInfo, text?: string): ChatMessage {
  return {
    id,
    role: "assistant",
    content: text ?? "",
    createdAt: toolCall.order!,
    toolCalls: [toolCall],
    renderParts: [
      { kind: "toolCall", id: toolCall.id, order: { runId: id, seq: 1 }, toolCall },
      ...(text ? [{ kind: "text" as const, id: `${id}-text`, order: { runId: id, seq: 2 }, content: text }] : []),
    ],
  };
}

const oldEdit: ToolCallInfo = {
  id: "old-edit",
  name: "edit",
  arguments: JSON.stringify({ filePath: "a.ts", oldString: "a", newString: "b" }),
  order: 1,
  outcome: "done",
  recordedOutput: "Edited a.ts",
};
const currentRead: ToolCallInfo = {
  id: "current-read",
  name: "read",
  arguments: JSON.stringify({ filePath: "b.ts" }),
  order: 3,
  outcome: "done",
  recordedOutput: "content",
};

function baseMessages(): ChatMessage[] {
  return [
    { id: "u1", role: "user", content: "First", createdAt: 0 },
    toolMessage("a1", oldEdit, "Edited"),
    { id: "u2", role: "user", content: "Second", createdAt: 2 },
    toolMessage("a2", currentRead),
  ];
}

const apps: App[] = [];

afterEach(() => {
  while (apps.length) apps.pop()!.unmount();
  resetEditDiffPayloadCacheForTest();
  mocks.viewerMounts = 0;
  mocks.collectionUpdates.clear();
  mocks.diffStrings.mockClear();
  document.body.innerHTML = "";
});

function mountTranscript(messages = baseMessages(), extra: Record<string, unknown> = {}) {
  const props = reactive({
    messages,
    streamingText: "",
    streamingTextOrder: 0,
    isStreaming: true,
    isThinking: false,
    activeToolCalls: [] as ToolCallDisplay[],
    liveRenderParts: [] as AssistantRenderPart[],
    variant: "session" as const,
    sessionKey: "history-memo",
    selectedMessageId: null as string | null,
    assistantLabel: "Locus",
    workspaceRef: { checkoutId: "checkout", expectedGeneration: 1 } as { checkoutId: string; expectedGeneration: number },
    ...extra,
  });
  const host = document.createElement("div");
  document.body.append(host);
  const app = createApp({ render: () => h(ChatTranscript, props) });
  app.use(createPinia());
  app.mount(host);
  apps.push(app);
  return { props, host };
}

async function flush() {
  for (let index = 0; index < 6; index += 1) {
    await Promise.resolve();
    await nextTick();
  }
}

const updatesOf = (ids: string) => mocks.collectionUpdates.get(ids) ?? 0;

describe("ChatTranscript history memoization", () => {
  it("does not re-render earlier groups for live tool calls, streaming text, or status changes", async () => {
    const { props } = mountTranscript();
    await flush();
    mocks.collectionUpdates.clear();

    for (const activeToolCalls of [
      [{ id: "live-bash", name: "bash", arguments: "{}", status: "running" }],
      [{ id: "live-bash", name: "bash", arguments: "{}", status: "done", output: "ok" }],
      [],
    ] as ToolCallDisplay[][]) {
      props.activeToolCalls = activeToolCalls;
      await flush();
    }
    props.streamingText = "Streaming answer";
    await flush();
    props.isThinking = true;
    await flush();

    expect(updatesOf("old-edit")).toBe(0);
  });

  it("re-renders an earlier group for selection, label, and workspace changes", async () => {
    const { props, host } = mountTranscript();
    await flush();
    const firstAssistant = () => host.querySelector<HTMLElement>('.chat-transcript-message[data-chat-message-id="a1"]')!;

    props.selectedMessageId = "a1";
    await flush();
    expect(firstAssistant().querySelector(".is-context-selected")).not.toBeNull();

    props.assistantLabel = "Agent";
    await flush();
    expect(firstAssistant().querySelector(".chat-transcript-message-role")?.textContent?.trim()).toBe("Agent");

    const firstAnswer = () => firstAssistant().querySelector<HTMLElement>(".markdown-stub")!;
    expect(firstAnswer().dataset.checkout).toBe("checkout");
    props.workspaceRef = { checkoutId: "other-checkout", expectedGeneration: 1 };
    await flush();
    expect(firstAnswer().dataset.checkout).toBe("other-checkout");
  });

  it("re-renders an earlier group when its tool output arrives", async () => {
    const pending: ToolCallInfo = { id: "pending-bash", name: "bash", arguments: JSON.stringify({ command: "ls" }), order: 1 };
    const messages = baseMessages();
    messages[1] = toolMessage("a1", pending, "Ran it");
    const { props, host } = mountTranscript(messages);
    await flush();
    const block = () => host.querySelector<HTMLElement>('[data-render-part-key="history:tools:pending-bash"] .tool-call-block')!;
    block().querySelector<HTMLElement>(".tool-call-header")!.click();
    await flush();
    expect(block().textContent).not.toContain("listing-output");

    props.messages = [
      ...props.messages,
      { id: "tool-result", role: "tool", content: "listing-output", createdAt: 5, toolCallId: "pending-bash" },
    ];
    await flush();
    expect(block().textContent).toContain("listing-output");
  });

  it("updates the trailing group's continuation layout", async () => {
    const { props, host } = mountTranscript(baseMessages(), { isStreaming: false });
    await flush();
    const last = () => host.querySelector<HTMLElement>('.chat-transcript-message[data-chat-message-id="a2"]')!;
    expect(last().classList.contains("before-continuation")).toBe(false);

    props.isStreaming = true;
    props.streamingText = "Continuing";
    await flush();
    expect(last().classList.contains("before-continuation")).toBe(true);
  });

  it("keeps an expanded edit diff in an earlier group mounted while live tool calls run", async () => {
    const { props, host } = mountTranscript();
    await flush();

    const oldGroup = host.querySelector('[data-render-part-key="history:tools:old-edit"]')!;
    oldGroup.querySelector<HTMLElement>(".tool-call-header")!.click();
    await flush();
    expect(oldGroup.querySelector(".diff-viewer-stub")).not.toBeNull();
    expect(mocks.viewerMounts).toBe(1);

    for (const activeToolCalls of [
      [{ id: "live-bash", name: "bash", arguments: "{}", status: "running" }],
      [{ id: "live-bash", name: "bash", arguments: "{}", status: "done", output: "ok" }],
      [],
    ] as ToolCallDisplay[][]) {
      props.activeToolCalls = activeToolCalls;
      await nextTick();
      expect(oldGroup.querySelector(".edit-diff-container")).toBeNull();
      await flush();
    }

    expect(oldGroup.querySelector(".diff-viewer-stub")).not.toBeNull();
    expect(mocks.viewerMounts).toBe(1);
    expect(mocks.diffStrings).toHaveBeenCalledTimes(1);
  });
});
