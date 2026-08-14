import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const cwd = process.cwd();

function read(relPath: string) {
  return readFileSync(resolve(cwd, relPath), "utf8");
}

describe("streaming markdown renderer wiring", () => {
  it("renders the transient content segment through the block splitter", () => {
    const transcript = read("src/components/chat/ChatTranscript.vue");

    expect(transcript).toContain(
      'import StreamingMarkdownRenderer from "./StreamingMarkdownRenderer.vue";',
    );
    // The transient (streaming) segment uses the O(n) block renderer...
    const transientBlock = transcript.slice(
      transcript.indexOf('data-render-part-scope="transient"'),
    );
    expect(transientBlock).toContain("<StreamingMarkdownRenderer");
    // History keeps the same split renderer for messages that have appeared
    // as the live tail, avoiding a transient-to-history height handoff. Older
    // one-shot history still uses MarkdownRenderer.
    const historyBlock = transcript.slice(
      transcript.indexOf('data-render-part-scope="history"'),
      transcript.indexOf('data-render-part-scope="transient"'),
    );
    expect(historyBlock).toContain("<MarkdownRenderer");
    expect(historyBlock).toContain("<StreamingMarkdownRenderer");
    expect(historyBlock).toContain("shouldUseStableHistoryMarkdown(segment.itemId)");
  });

  it("freezes prefix blocks behind stable keys and re-renders only the tail", () => {
    const renderer = read("src/components/chat/StreamingMarkdownRenderer.vue");

    expect(renderer).toContain("new StreamingMarkdownSplitter()");
    expect(renderer).toContain('v-for="block in split.blocks"');
    expect(renderer).toContain(':key="block.id"');
    // Only the tail renderer carries the streaming cursor.
    expect(renderer).toContain(':cursor="cursor"');
    // Oversized tails (single uncuttable block, e.g. a giant unclosed fence)
    // degrade to plain text so per-frame cost stays bounded.
    expect(renderer).toContain("TAIL_MARKDOWN_LIMIT");
    expect(renderer).toContain("split.tail.length > TAIL_MARKDOWN_LIMIT");
  });

  it("shares one Marked instance across markdown surfaces", () => {
    const renderer = read("src/components/MarkdownRenderer.vue");
    const engine = read("src/composables/markdownEngine.ts");

    expect(engine).toContain("export const markdownEngine = new Marked(");
    expect(renderer).toContain(
      'import { escapeMarkdownHtml, markdownEngine } from "../composables/markdownEngine";',
    );
    expect(renderer).not.toContain("new Marked(");
  });

  it("reconciles the viewport after the streaming Markdown DOM patch", () => {
    const renderer = read("src/components/chat/StreamingMarkdownRenderer.vue");
    const transcript = read("src/components/chat/ChatTranscript.vue");
    const chatView = read("src/components/ChatView.vue");

    expect(renderer).toContain('(e: "layoutChange"): void;');
    expect(renderer).toContain('watch(split, () => emit("layoutChange"), { flush: "post" });');
    expect(transcript).toContain('@layout-change="emit(\'streamLayoutChange\')"');
    expect(chatView).toContain('@stream-layout-change="reconcileStreamingLayoutNow"');
    expect(chatView).toContain("function reconcileStreamingLayoutNow()");
  });

  it("keeps the streaming cursor out of inline layout width", () => {
    const renderer = read("src/components/MarkdownRenderer.vue");
    const cursorRule = renderer.match(/\.streaming-cursor\s*\{([^}]+)\}/)?.[1] ?? "";

    expect(cursorRule).toContain("display: inline-block;");
    expect(cursorRule).toContain("width: 0;");
    expect(cursorRule).toContain("margin-left: 0;");
    expect(cursorRule).toContain("overflow: visible;");
  });

  it("does not render an empty transient wrapper during the stream-end handoff", () => {
    const transcript = read("src/components/chat/ChatTranscript.vue");

    expect(transcript).toContain("const shouldRenderTransientAssistantMessage = computed(() =>");
    expect(transcript).toContain("hasTransientAssistantMessage.value && transientRenderSegments.value.length > 0");
    expect(transcript).toContain("const isRenderedStreamingContinuation = computed(() =>");
    expect(transcript).toContain('v-if="shouldRenderTransientAssistantMessage"');
    expect(transcript).toContain("'before-continuation': isRenderedStreamingContinuation");
  });
});
