import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const editorSource = readFileSync(
  resolve(process.cwd(), "src/components/ui/BaseMarkdownEditor.vue"),
  "utf8",
);
const livePreviewSource = readFileSync(
  resolve(process.cwd(), "src/components/ui/markdown-editor/markdownLivePreview.ts"),
  "utf8",
);

describe("Markdown editor selection layout", () => {
  it("keeps horizontal document spacing on lines so multi-line selection excludes the outer gutter", () => {
    expect(editorSource).toMatch(
      /\.base-markdown-editor :deep\(\.cm-content\)\s*\{[\s\S]*?padding:\s*14px 0 16px;/,
    );
    expect(editorSource).toMatch(
      /\.base-markdown-editor :deep\(\.cm-line\)\s*\{[\s\S]*?padding:\s*0 var\(--markdown-document-padding-right\) 0 var\(--markdown-document-padding-left\);/,
    );
  });

  it("preserves inset block surfaces after moving the document gutter to lines", () => {
    expect(editorSource).toMatch(
      /\.base-markdown-editor :deep\(\.cm-live-blockquote\)\s*\{[\s\S]*?margin-right:\s*var\(--markdown-document-padding-right\);[\s\S]*?margin-left:\s*var\(--markdown-document-padding-left\);/,
    );
    expect(editorSource).toMatch(
      /\.base-markdown-editor :deep\(\.cm-live-fenced-code\)\s*\{[\s\S]*?margin-right:\s*var\(--markdown-document-padding-right\);[\s\S]*?margin-left:\s*var\(--markdown-document-padding-left\);/,
    );
    expect(livePreviewSource).toMatch(
      /"\.cm-live-table-line":\s*\{[\s\S]*?marginRight:\s*"var\(--markdown-document-padding-right\)"[\s\S]*?marginLeft:\s*"var\(--markdown-document-padding-left\)"/,
    );
  });
});
