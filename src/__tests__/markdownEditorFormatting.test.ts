import { describe, expect, it } from "vitest";
import {
  canSyncMarkdownEditorWhileFocused,
  countHiddenInlineMarkers,
  hasRenderableInlineCompletion,
  normalizeInlineMarkdown,
  normalizeMarkdownEditorLineEndings,
  shouldPreferMarkdownPlainTextPaste,
} from "../components/ui/markdownEditorFormatting";

describe("markdownEditorFormatting", () => {
  it("counts hidden markers for completed inline markdown", () => {
    expect(countHiddenInlineMarkers("*asd*")).toBe(2);
    expect(countHiddenInlineMarkers("**速度**")).toBe(4);
    expect(countHiddenInlineMarkers("***very*** ~~fast~~")).toBe(10);
  });

  it("ignores escaped markers when checking renderable completion", () => {
    expect(hasRenderableInlineCompletion("\\*literal\\*")).toBe(false);
    expect(hasRenderableInlineCompletion("*italic*")).toBe(true);
    expect(hasRenderableInlineCompletion("`code`")).toBe(true);
  });

  it("normalizes inline markdown escapes only for formatting tokens", () => {
    expect(normalizeInlineMarkdown("\\*a\\* and \\_b\\_")).toBe("*a* and _b_");
    expect(normalizeInlineMarkdown("\\[link\\]")).toBe("\\[link\\]");
  });

  it("allows focused editor resync when markdown content is unchanged", () => {
    expect(canSyncMarkdownEditorWhileFocused("**速度**\r\n", "**速度**\n")).toBe(true);
    expect(normalizeMarkdownEditorLineEndings("a\r\nb")).toBe("a\nb");
    expect(canSyncMarkdownEditorWhileFocused("**速度**", "*速度*")).toBe(false);
  });

  it("prefers plain-text markdown paste for prose copied from preformatted html", () => {
    const html = '<pre style="font-family: monospace;">- 用户希望汇报方式更简洁\n- 用户希望保留基本实现逻辑</pre>';
    const text = "- 用户希望汇报方式更简洁\n- 用户希望保留基本实现逻辑";

    expect(shouldPreferMarkdownPlainTextPaste(html, text)).toBe(true);
  });

  it("keeps real code pastes on the code-path heuristic", () => {
    const html = '<pre style="font-family: monospace;">const answer = compute();\nreturn answer;</pre>';
    const text = "const answer = compute();\nreturn answer;";

    expect(shouldPreferMarkdownPlainTextPaste(html, text)).toBe(false);
  });
});
