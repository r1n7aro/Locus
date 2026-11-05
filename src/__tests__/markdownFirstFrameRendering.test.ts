// @vitest-environment jsdom
import { describe, expect, it } from "vitest";
import { markdownEngine } from "../composables/markdownEngine";
import {
  collectFileRefPathCandidates,
  collectInlineCodePathCandidates,
  collectMarkdownInlineCodePathCandidates,
  collectMarkdownPathCandidates,
  injectAssetRefs,
} from "../composables/markdownInject";
import { normalizeMarkdownForRender } from "../composables/markdownRender";
import { sanitizeRenderedMarkdownHtml } from "../composables/markdownSanitize";

const transcriptMarkdown = [
  "## Changes",
  "Updated `src/core/Engine.ts` and `src/ui/Panel.vue`; the folder `docs/` is unchanged.",
  "Run `bun run test` afterwards, then open `README.md`.",
  "",
  "| File | State |",
  "| --- | --- |",
  "| `scripts/build.mjs` | edited |",
  "| ``config/app.json`` | new |",
  "",
  "- check `tests/engine.test.ts:12`",
  "",
  "```ts",
  "import { marker } from \"src/fenced/Ignored.ts\";",
  "const path = `src/fenced/Template.ts`;",
  "```",
  "",
  "~~~",
  "`src/fenced/Tilde.ts`",
  "~~~",
  "Final note on `src/net/Session.ts`.",
].join("\n");

function rendererCandidates(markdown: string): string[] {
  return collectInlineCodePathCandidates(markdownEngine.parse(normalizeMarkdownForRender(markdown)) as string);
}

describe("first-frame markdown rendering", () => {
  it("collects every path the renderer resolves directly from markdown source", () => {
    const fromSource = collectMarkdownInlineCodePathCandidates(transcriptMarkdown);
    const fromRenderer = rendererCandidates(transcriptMarkdown);

    expect(fromRenderer.length).toBeGreaterThan(0);
    expect(fromSource).toEqual(expect.arrayContaining(fromRenderer));
    expect(fromSource.some((path) => path.includes("fenced"))).toBe(false);
  });

  it("collects every bare-text path the renderer resolves directly from markdown source", () => {
    const markdown = [
      "Edited src/core/Engine.ts:12 and utils/helpers.ts; see Assets/Prefabs/Hero.prefab.",
      "Timings 6.9/7.0/7.1 ms, docs at https://example.com/a/b.html, and C:/Work/notes.txt.",
      "",
      "```ts",
      "import { marker } from \"src/fenced/Ignored.ts\";",
      "```",
      "- `src/inline/Code.ts` next to src/list/Item.vue",
    ].join("\n");
    const html = markdownEngine.parse(normalizeMarkdownForRender(markdown)) as string;
    const fromRenderer = collectFileRefPathCandidates(html);
    const fromSource = collectMarkdownPathCandidates(markdown, { fileRefs: true });

    expect(fromRenderer).toEqual(expect.arrayContaining([
      "src/core/Engine.ts",
      "utils/helpers.ts",
      "Assets/Prefabs/Hero.prefab",
      "C:/Work/notes.txt",
      "src/list/Item.vue",
    ]));
    expect(fromSource).toEqual(expect.arrayContaining(fromRenderer));
    expect(fromSource).toContain("src/inline/Code.ts");
    expect(fromSource.some((path) => path.includes("fenced") || path.includes("7.1") || path.includes("example.com"))).toBe(false);
  });

  it("returns no candidates for markdown without inline code", () => {
    expect(collectMarkdownInlineCodePathCandidates("Plain text with a path src/a.ts")).toEqual([]);
  });

  it("sanitizes file references to the same markup with or without their discarded icons", () => {
    const html = markdownEngine.parse(normalizeMarkdownForRender(transcriptMarkdown)) as string;
    const withReferences = injectAssetRefs(html, {
      inlinePathStatus: (path) => ({ path, exists: true, entryKind: path.endsWith("/") ? "folder" : "file" }),
      requireInlinePathStatus: true,
    });
    expect(withReferences).toContain("md-ref-icon-lucide");

    const sanitized = sanitizeRenderedMarkdownHtml(withReferences);
    expect(sanitized).toContain("md-workspace-ref");
    expect(sanitized).not.toContain("<svg");
    // The html profile drops SVG, so removing generated icons first is output-neutral.
    const withoutIcons = withReferences.replace(/<svg class="md-ref-icon md-ref-icon-lucide[^"]*"[^>]*>[\s\S]*?<\/svg>/g, "");
    expect(sanitizeRenderedMarkdownHtml(`${withoutIcons} `)).toBe(`${sanitized} `);
    expect(sanitizeRenderedMarkdownHtml(withReferences)).toBe(sanitized);
  });

  it("still removes unsafe markup from cached and uncached input", () => {
    const unsafe = '<p onclick="alert(1)">x</p><script>alert(2)</script><img src="x" onerror="alert(3)">';
    for (let attempt = 0; attempt < 2; attempt += 1) {
      const sanitized = sanitizeRenderedMarkdownHtml(unsafe);
      expect(sanitized).not.toMatch(/onclick|onerror|<script/i);
    }
  });
});
