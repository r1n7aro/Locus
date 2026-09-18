import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const read = (path: string) => readFileSync(resolve(process.cwd(), path), "utf8");

describe("workspace knowledge document layout", () => {
  it("uses the workspace tree as the only document list for a selected document", () => {
    const workbench = read("src/components/workbench/DevelopmentWorkbench.vue");
    const knowledgeView = read("src/components/KnowledgeView.vue");

    expect(workbench).toContain(':embedded="editor.resource.kind === \'knowledge\' || (editor.resource.kind === \'section\' && !!editor.resource.knowledgePage)"');
    expect(workbench).toContain(':selected-document-id="editorKnowledgeDocument(editor)?.id ?? null"');
    expect(workbench).toContain(':selected-document-target="editorKnowledgeDocument(editor)"');
    expect(workbench).toContain(':active="contentActive"');
    expect(knowledgeView).toMatch(/v-if="!props\.embedded"\s+class="kx-side"/);
    expect(knowledgeView).toMatch(/v-if="!props\.embedded && !props\.listOnly"\s+class="resize-handle"/);
    expect(knowledgeView).toContain(':embedded="props.embedded"');
    expect(knowledgeView).toContain(':active="props.active"');
  });

  it("removes the redundant document header in every host", () => {
    const preview = read("src/components/knowledge/KnowledgePreview.vue");

    expect(preview).toContain("embedded?: boolean;");
    expect(preview).not.toContain('class="preview-header"');
  });

  it("lets the continuous document scroller receive wheels from auto-grow editors", () => {
    const styles = read("src/components/ui/markdown-document.css");

    expect(styles).toMatch(/\.document-scroller\s*\{[\s\S]*overflow:\s*auto;/);
    expect(styles).toMatch(/\.document-body :deep\(\.base-markdown-editor \.cm-scroller\)\s*\{[\s\S]*overflow:\s*visible;[\s\S]*overscroll-behavior:\s*auto;/);
  });

  it("keeps rendered tables inside the centered document page", () => {
    const styles = read("src/components/ui/markdown-document.css");
    const livePreview = read("src/components/ui/markdown-editor/markdownLivePreview.ts");

    expect(styles).toMatch(/\.document-page\s*\{[\s\S]*width:\s*min\(100%, 980px\);[\s\S]*margin:\s*0 auto;/);
    expect(livePreview).toMatch(/"\.cm-live-table-row":\s*\{[\s\S]*width:\s*"100%",/);
  });

  it("keeps the outline visible before there is room for a centered document", () => {
    const styles = read("src/components/ui/markdown-document.css");

    expect(styles).toMatch(/@container knowledge-document \(min-width: 1120px\)\s*\{[\s\S]*grid-template-columns:\s*210px minmax\(0, 920px\);[\s\S]*justify-content:\s*center;/);
    expect(styles).toContain("@container knowledge-document (min-width: 1488px)");
    expect(styles).toMatch(/@container knowledge-document \(min-width: 1488px\)\s*\{[\s\S]*grid-template-columns:\s*minmax\(210px, 1fr\) minmax\(0, 980px\) minmax\(210px, 1fr\);/);
    expect(styles).toMatch(/\.document-workspace\.has-outline \.document-outline\s*\{[\s\S]*grid-column:\s*1;[\s\S]*justify-self:\s*end;/);
    expect(styles).toMatch(/\.document-workspace\.has-outline \.document-page\s*\{[\s\S]*grid-column:\s*2;/);
  });

  it("uses a physical hit area for the knowledge directory resize handle", () => {
    const knowledgeView = read("src/components/KnowledgeView.vue");

    expect(knowledgeView).toContain('role="separator"');
    expect(knowledgeView).toContain(':class="{ active: resizingSidebar }"');
    expect(knowledgeView).toMatch(/\.resize-handle\s*\{[\s\S]*width:\s*6px;[\s\S]*margin:\s*0 -3px;/);
    expect(knowledgeView).toMatch(/function onResizeStart\(event: MouseEvent\)\s*\{[\s\S]*event\.preventDefault\(\);/);
  });
});
