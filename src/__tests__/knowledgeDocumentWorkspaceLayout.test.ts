import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const read = (path: string) => readFileSync(resolve(process.cwd(), path), "utf8");

describe("workspace knowledge document layout", () => {
  it("uses the workspace tree as the only document list for a selected document", () => {
    const workbench = read("src/components/workbench/DevelopmentWorkbench.vue");
    const knowledgeView = read("src/components/KnowledgeView.vue");

    expect(workbench).toContain(':embedded="editor.resource.kind === \'knowledge\'"');
    expect(workbench).toContain(':selected-document-id="editorKnowledgeDocument(editor)?.id ?? null"');
    expect(workbench).toContain(':selected-document-target="editorKnowledgeDocument(editor)"');
    expect(workbench).toContain(':active="group.activeEditorId === editor.editorId"');
    expect(knowledgeView).toMatch(/v-if="!props\.embedded"\s+class="kx-side"/);
    expect(knowledgeView).toMatch(/v-if="!props\.embedded"\s+class="resize-handle"/);
    expect(knowledgeView).toContain(':embedded="props.embedded"');
    expect(knowledgeView).toContain(':active="props.active"');
  });

  it("hides the redundant document header inside a workspace tab", () => {
    const preview = read("src/components/knowledge/KnowledgePreview.vue");

    expect(preview).toContain("embedded?: boolean;");
    expect(preview).toContain('<div v-if="!props.embedded" class="preview-header">');
  });

  it("lets the continuous document scroller receive wheels from auto-grow editors", () => {
    const preview = read("src/components/knowledge/KnowledgePreview.vue");

    expect(preview).toMatch(/\.preview-main\s*\{[\s\S]*overflow:\s*auto;/);
    expect(preview).toMatch(/\.document-body :deep\(\.base-markdown-editor \.cm-scroller\)\s*\{[\s\S]*overflow:\s*visible;[\s\S]*overscroll-behavior:\s*auto;/);
  });

  it("keeps rendered tables inside the centered document page", () => {
    const preview = read("src/components/knowledge/KnowledgePreview.vue");
    const livePreview = read("src/components/ui/markdown-editor/markdownLivePreview.ts");

    expect(preview).toMatch(/\.document-page\s*\{[\s\S]*width:\s*min\(100%, 980px\);[\s\S]*margin:\s*0 auto;/);
    expect(livePreview).toMatch(/"\.cm-live-table-row":\s*\{[\s\S]*width:\s*"100%",/);
  });

  it("uses a physical hit area for the knowledge directory resize handle", () => {
    const knowledgeView = read("src/components/KnowledgeView.vue");

    expect(knowledgeView).toContain('role="separator"');
    expect(knowledgeView).toContain(':class="{ active: resizingSidebar }"');
    expect(knowledgeView).toMatch(/\.resize-handle\s*\{[\s\S]*width:\s*6px;[\s\S]*margin:\s*0 -3px;/);
    expect(knowledgeView).toMatch(/function onResizeStart\(event: MouseEvent\)\s*\{[\s\S]*event\.preventDefault\(\);/);
  });
});
