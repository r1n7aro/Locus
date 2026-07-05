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
    // ...while history keeps the one-shot full render that corrects any
    // block-boundary divergence accepted during streaming.
    const historyBlock = transcript.slice(
      transcript.indexOf('data-render-part-scope="history"'),
      transcript.indexOf('data-render-part-scope="transient"'),
    );
    expect(historyBlock).toContain("<MarkdownRenderer");
    expect(historyBlock).not.toContain("<StreamingMarkdownRenderer");
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
});
