// @vitest-environment jsdom
import { createApp, defineComponent, h, nextTick, shallowRef, type App } from "vue";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { StreamingTextChunks } from "../composables/streamingTextChunks";

const prefetchMocks = vi.hoisted(() => ({
  prefetchMarkdownPathCandidates: vi.fn<(candidates: Iterable<string>) => Promise<void> | null>(),
}));

vi.mock("../composables/markdownPathStatusPrefetch", () => prefetchMocks);
vi.mock("../components/MarkdownRenderer.vue", () => ({
  default: defineComponent({
    props: ["content"],
    setup: (props) => () => h("div", { class: "md-stub" }, props.content),
  }),
}));

import StreamingMarkdownRenderer from "../components/chat/StreamingMarkdownRenderer.vue";

let app: App | null = null;
let host: HTMLElement | null = null;

function shownText(): string {
  return [...host!.querySelectorAll(".md-stub")].map((node) => node.textContent ?? "").join("|");
}

async function flush() {
  await nextTick();
  await Promise.resolve();
  await nextTick();
}

function mountRenderer(props: Record<string, unknown>) {
  const state = shallowRef(props);
  host = document.createElement("div");
  app = createApp(defineComponent({
    setup: () => () => h(StreamingMarkdownRenderer, state.value),
  }));
  app.mount(host);
  return {
    async update(next: Record<string, unknown>) {
      state.value = { ...state.value, ...next };
      await flush();
    },
  };
}

describe("streaming markdown path references", () => {
  beforeEach(() => {
    vi.useFakeTimers();
    prefetchMocks.prefetchMarkdownPathCandidates.mockReset();
    prefetchMocks.prefetchMarkdownPathCandidates.mockReturnValue(null);
  });

  afterEach(() => {
    app?.unmount();
    app = null;
    host = null;
    vi.useRealTimers();
  });

  it("holds a growing trailing path run until a delimiter arrives or the text idles", async () => {
    const renderer = mountRenderer({ content: "Edit", enableFileRefs: true, cursor: true });
    await flush();
    expect(shownText()).toBe("Edit");

    await renderer.update({ content: "Edit src/compo" });
    expect(shownText()).toBe("Edit ");

    await renderer.update({ content: "Edit src/components/Panel.vue now" });
    expect(shownText()).toBe("Edit src/components/Panel.vue ");

    vi.advanceTimersByTime(250);
    await flush();
    expect(shownText()).toBe("Edit src/components/Panel.vue now");
  });

  it("does not hold text of finalized or file-reference-free renderers", async () => {
    const finalized = mountRenderer({ content: "Edit", enableFileRefs: true, finalized: true });
    await finalized.update({ content: "Edit src/compo" });
    expect(shownText()).toBe("Edit src/compo");
    app?.unmount();

    const plain = mountRenderer({ content: "Edit" });
    await plain.update({ content: "Edit src/compo" });
    expect(shownText()).toBe("Edit src/compo");
  });

  it("waits for unknown path statuses before showing new text, within a bounded budget", async () => {
    const renderer = mountRenderer({ content: "Edit", enableFileRefs: true, cursor: true });
    await flush();

    let resolveStatuses!: () => void;
    prefetchMocks.prefetchMarkdownPathCandidates.mockImplementation((candidates) => (
      [...candidates].includes("src/a.ts")
        ? new Promise<void>((resolve) => { resolveStatuses = resolve; })
        : null
    ));
    await renderer.update({ content: "Edit src/a.ts and " });
    expect(shownText()).toBe("Edit");

    prefetchMocks.prefetchMarkdownPathCandidates.mockReturnValue(null);
    resolveStatuses();
    await flush();
    expect(shownText()).toBe("Edit src/a.ts and ");

    prefetchMocks.prefetchMarkdownPathCandidates.mockReturnValue(new Promise<void>(() => {}));
    await renderer.update({ content: "Edit src/a.ts and src/b.ts then " });
    expect(shownText()).toBe("Edit src/a.ts and ");
    vi.advanceTimersByTime(80);
    await flush();
    expect(shownText()).toBe("Edit src/a.ts and src/b.ts then ");
  });

  it("prefetches statuses as stream deltas arrive, before the next render", async () => {
    const stream = new StreamingTextChunks();
    mountRenderer({ stream, enableFileRefs: true, cursor: true });
    await flush();

    stream.append("See src/core/Engine.ts and src/ui/Pa");
    await flush();
    const requested = prefetchMocks.prefetchMarkdownPathCandidates.mock.calls.map(([candidates]) => [...candidates]);
    expect(requested).toContainEqual(["src/core/Engine.ts"]);
    expect(requested.flat()).not.toContain("src/ui/Pa");
    expect(shownText()).toBe("");

    vi.advanceTimersByTime(80);
    await flush();
    expect(shownText()).toBe("See src/core/Engine.ts and ");
  });
});
