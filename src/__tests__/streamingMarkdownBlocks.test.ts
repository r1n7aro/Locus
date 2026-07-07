import { describe, expect, it } from "vitest";
import { Marked } from "marked";
import {
  StreamingMarkdownSplitter,
  type StreamingMarkdownSplit,
} from "../composables/streamingMarkdownBlocks";

function feedInSteps(text: string, step: number): StreamingMarkdownSplit {
  const splitter = new StreamingMarkdownSplitter();
  let split: StreamingMarkdownSplit = { blocks: [], tail: "" };
  for (let cut = step; cut < text.length + step; cut += step) {
    split = splitter.update(text.slice(0, Math.min(cut, text.length)));
  }
  return split;
}

function joined(split: StreamingMarkdownSplit): string {
  return split.blocks.map((block) => block.text).join("") + split.tail;
}

describe("StreamingMarkdownSplitter", () => {
  it("freezes paragraph blocks and keeps the trailing completed block in the tail", () => {
    // Trailing newline completes the last line: only complete lines register
    // cut points, so an in-flight line can never advance the boundary.
    const text = "first para\n\nsecond para\n\nthird para\n\nfourth grows\n";
    const split = feedInSteps(text, 7);

    // Cuts exist before "second", "third", "fourth"; lag-one keeps the block
    // before the newest cut ("third para") in the tail.
    expect(split.blocks.map((b) => b.text)).toEqual([
      "first para\n\n",
      "second para\n\n",
    ]);
    expect(split.tail).toBe("third para\n\nfourth grows\n");
    expect(joined(split)).toBe(text);
  });

  it("reassembles the source exactly for any step size", () => {
    const text = [
      "# Title",
      "",
      "Paragraph one with `inline code`.",
      "",
      "```csharp",
      "int a = 1;",
      "",
      "int b = 2;",
      "```",
      "",
      "- item one",
      "",
      "- item two",
      "",
      "Done.",
    ].join("\n");
    for (const step of [1, 3, 8, 64, text.length]) {
      const split = feedInSteps(text, step);
      expect(joined(split)).toBe(text);
    }
  });

  it("produces the same split incrementally as in one shot", () => {
    const text = [
      "Intro paragraph.",
      "",
      "> quote line one",
      "",
      "> quote line two",
      "",
      "New paragraph.",
      "",
      "1. first",
      "2. second",
      "",
      "   still second (loose continuation)",
      "",
      "Ending paragraph after the list.",
      "",
      "Final tail",
    ].join("\n");

    const oneShot = new StreamingMarkdownSplitter().update(text);
    for (const step of [1, 2, 5, 17]) {
      const incremental = feedInSteps(text, step);
      expect(incremental.blocks.map((b) => b.text)).toEqual(
        oneShot.blocks.map((b) => b.text),
      );
      expect(incremental.tail).toBe(oneShot.tail);
    }
  });

  it("append(delta) produces the same split as update(full) for any step size", () => {
    const text = [
      "# Title",
      "",
      "Intro paragraph with `code`.",
      "",
      "```csharp",
      "int a = 1;",
      "",
      "int b = 2;",
      "```",
      "",
      "- item one",
      "",
      "- item two",
      "",
      "> quote one",
      "",
      "> quote two",
      "",
      "Ending paragraph.",
      "",
      "Tail line",
    ].join("\n");

    const oneShot = new StreamingMarkdownSplitter().update(text);
    for (const step of [1, 2, 3, 7, 31, 128, text.length]) {
      const splitter = new StreamingMarkdownSplitter();
      let split: StreamingMarkdownSplit = { blocks: [], tail: "" };
      for (let cut = 0; cut < text.length; cut += step) {
        split = splitter.append(text.slice(cut, cut + step));
      }
      expect(split.blocks.map((b) => b.text)).toEqual(oneShot.blocks.map((b) => b.text));
      expect(split.tail).toBe(oneShot.tail);
      expect(joined(split)).toBe(text);
    }
  });

  it("append after update continues the same document", () => {
    const splitter = new StreamingMarkdownSplitter();
    splitter.update("first para\n\nsecond");
    const split = splitter.append(" para\n\nthird para\n\nfourth\n");
    const oneShot = new StreamingMarkdownSplitter().update(
      "first para\n\nsecond para\n\nthird para\n\nfourth\n",
    );
    expect(split.blocks.map((b) => b.text)).toEqual(oneShot.blocks.map((b) => b.text));
    expect(split.tail).toBe(oneShot.tail);
  });

  it("append with an empty delta returns the current snapshot unchanged", () => {
    const splitter = new StreamingMarkdownSplitter();
    const before = splitter.update("alpha\n\nbeta\n\ngamma\n\ndelta\n");
    const after = splitter.append("");
    expect(after.blocks.map((b) => b.text)).toEqual(before.blocks.map((b) => b.text));
    expect(after.tail).toBe(before.tail);
  });

  it("never cuts inside a fenced code block even across blank lines", () => {
    const text = [
      "before",
      "",
      "```python",
      "block_one()",
      "",
      "",
      "block_two()",
      "```",
      "",
      "after one",
      "",
      "after two",
      "",
      "after three",
    ].join("\n");
    const split = feedInSteps(text, 5);
    const fenceBlock = [...split.blocks.map((b) => b.text), split.tail].find(
      (chunk) => chunk.includes("```python"),
    );
    expect(fenceBlock).toBeDefined();
    expect(fenceBlock).toContain("block_two()\n```");
    expect(joined(split)).toBe(text);
  });

  it("treats an unclosed fence as one growing block", () => {
    const text = [
      "intro",
      "",
      "```js",
      "line one",
      "",
      "line two never closes",
      "",
      "more code",
    ].join("\n");
    const split = feedInSteps(text, 4);
    expect(split.tail).toContain("```js");
    expect(split.tail).toContain("more code");
    // Only the cut before the fence opener may freeze anything.
    expect(split.blocks.map((b) => b.text).join("")).toBe(
      split.blocks.length ? "intro\n\n" : "",
    );
  });

  it("keeps loose list items in one block", () => {
    const text = [
      "- alpha",
      "",
      "- beta",
      "",
      "- gamma",
      "",
      "Paragraph after the list.",
      "",
      "Trailing paragraph one.",
      "",
      "Trailing paragraph two.",
    ].join("\n");
    const split = feedInSteps(text, 6);
    const listChunk = [...split.blocks.map((b) => b.text), split.tail].find(
      (chunk) => chunk.includes("- alpha"),
    );
    expect(listChunk).toContain("- gamma");
    expect(joined(split)).toBe(text);
  });

  it("keeps ordered list numbering together across blank-separated items", () => {
    const text = [
      "1. one",
      "",
      "2. two",
      "",
      "3. three",
      "",
      "After list para.",
      "",
      "Another para.",
      "",
      "Third para.",
    ].join("\n");
    const split = feedInSteps(text, 9);
    const listChunk = [...split.blocks.map((b) => b.text), split.tail].find(
      (chunk) => chunk.includes("1. one"),
    );
    expect(listChunk).toContain("3. three");
  });

  it("keeps blockquote runs separated by blank lines together", () => {
    const text = [
      "> first quote",
      "",
      "> second quote",
      "",
      "plain paragraph",
      "",
      "another paragraph",
      "",
      "third paragraph",
    ].join("\n");
    const split = feedInSteps(text, 5);
    const quoteChunk = [...split.blocks.map((b) => b.text), split.tail].find(
      (chunk) => chunk.includes("> first quote"),
    );
    expect(quoteChunk).toContain("> second quote");
    expect(quoteChunk).not.toContain("plain paragraph");
  });

  it("keeps indented code runs separated by blank lines together", () => {
    const text = [
      "intro paragraph",
      "",
      "    code line one",
      "",
      "    code line two",
      "",
      "outro paragraph",
      "",
      "second outro",
      "",
      "third outro",
    ].join("\n");
    const split = feedInSteps(text, 5);
    const codeChunk = [...split.blocks.map((b) => b.text), split.tail].find(
      (chunk) => chunk.includes("code line one"),
    );
    expect(codeChunk).toContain("code line two");
    expect(codeChunk).not.toContain("outro paragraph");
  });

  it("resets when the text shrinks or is replaced", () => {
    const splitter = new StreamingMarkdownSplitter();
    splitter.update("alpha\n\nbeta\n\ngamma\n\ndelta\n\n");
    const before = splitter.update("alpha\n\nbeta\n\ngamma\n\ndelta\n\nepsilon");
    expect(before.blocks.length).toBeGreaterThan(0);
    const beforeIds = before.blocks.map((b) => b.id);

    const after = splitter.update("totally different\n\ncontent here\n\nthird\n\nfourth");
    expect(joined(after)).toBe("totally different\n\ncontent here\n\nthird\n\nfourth");
    for (const block of after.blocks) {
      expect(beforeIds).not.toContain(block.id);
    }
  });

  it("keeps frozen block object identity stable across updates", () => {
    const splitter = new StreamingMarkdownSplitter();
    const first = splitter.update("one\n\ntwo\n\nthree\n\nfour\n\n");
    expect(first.blocks.length).toBeGreaterThan(0);
    const firstBlock = first.blocks[0];
    const second = splitter.update("one\n\ntwo\n\nthree\n\nfour\n\nfive grows here");
    expect(second.blocks[0]).toBe(firstBlock);
  });

  it("handles CRLF line endings", () => {
    const text = "para one\r\n\r\npara two\r\n\r\npara three\r\n\r\npara four";
    const split = feedInSteps(text, 6);
    expect(split.blocks.length).toBeGreaterThan(0);
    expect(joined(split)).toBe(text);
  });

  it("does not cut before the first non-blank content", () => {
    const splitter = new StreamingMarkdownSplitter();
    const split = splitter.update("\n\n\n\nfirst actual content");
    expect(split.blocks).toEqual([]);
    expect(split.tail).toBe("\n\n\n\nfirst actual content");
  });

  it("renders split blocks identically to a full parse (marked equivalence)", () => {
    const md = new Marked({ breaks: true, gfm: true });
    const corpus = [
      "# Heading",
      "",
      "First paragraph with **bold**, `code`, and 中文文本。",
      "",
      "```ts",
      "const value = compute();",
      "",
      "export default value;",
      "```",
      "",
      "- item one",
      "- item two",
      "",
      "- loose item three",
      "",
      "1. ordered one",
      "",
      "2. ordered two",
      "",
      "> a quote",
      "",
      "> more of the same quote",
      "",
      "| a | b |",
      "| - | - |",
      "| 1 | 2 |",
      "",
      "Paragraph before setext-ish line",
      "",
      "===",
      "",
      "    indented code",
      "",
      "    still indented code",
      "",
      "Closing paragraph.",
    ].join("\n");

    for (const step of [3, 11, corpus.length]) {
      const split = feedInSteps(corpus, step);
      expect(split.blocks.length).toBeGreaterThan(2);
      const splitHtml = [...split.blocks.map((b) => b.text), split.tail]
        .map((chunk) => md.parse(chunk) as string)
        .join("");
      const fullHtml = md.parse(corpus) as string;
      expect(splitHtml).toBe(fullHtml);
    }
  });

  it("random fuzz: any feeding schedule reassembles and matches one-shot cuts", () => {
    const corpus = [
      "Analysis paragraph about `PlayerController.cs` behavior.",
      "",
      "```csharp",
      "void Update() {",
      "",
      "    Move();",
      "}",
      "```",
      "",
      "- consider gravity",
      "",
      "- consider drag",
      "",
      "> note: physics steps",
      "",
      "> run at fixed rate",
      "",
      "Conclusion paragraph.",
      "",
      "Final paragraph.",
    ].join("\n");

    const oneShot = new StreamingMarkdownSplitter().update(corpus);
    let seed = 42;
    const nextRandom = () => {
      seed = (seed * 1103515245 + 12345) % 2147483648;
      return seed / 2147483648;
    };
    for (let round = 0; round < 20; round++) {
      const splitter = new StreamingMarkdownSplitter();
      let cursor = 0;
      let split: StreamingMarkdownSplit = { blocks: [], tail: "" };
      while (cursor < corpus.length) {
        cursor = Math.min(corpus.length, cursor + 1 + Math.floor(nextRandom() * 24));
        split = splitter.update(corpus.slice(0, cursor));
      }
      expect(joined(split)).toBe(corpus);
      expect(split.blocks.map((b) => b.text)).toEqual(oneShot.blocks.map((b) => b.text));
      expect(split.tail).toBe(oneShot.tail);
    }
  });
});
