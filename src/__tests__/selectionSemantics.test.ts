import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const cwd = process.cwd();

function read(relPath: string) {
  return readFileSync(resolve(cwd, relPath), "utf8");
}

describe("selection semantics", () => {
  it("defines shared selection utilities in App.vue", () => {
    const app = read("src/App.vue");

    expect(app).toMatch(/body\s*\{[\s\S]*?user-select:\s*none;/);
    expect(app).toContain("body.is-dragging-select-lock");
    expect(app).toContain(".ui-select-none");
    expect(app).toContain(".ui-select-text");
    expect(app).toContain(".selectable-text");
    expect(app).toContain(":where(pre, code)");
  });

  it("marks critical content leaves explicitly", () => {
    const chatView = read("src/components/ChatView.vue");
    const chatTranscript = read("src/components/chat/ChatTranscript.vue");
    const chatComposer = read("src/components/chat/ChatComposer.vue");
    const terminal = read("src/components/GitTerminal.vue");
    const commitDetail = read("src/components/collab/CommitDetail.vue");
    const gitSidebar = read("src/components/collab/GitSidebar.vue");
    const markdownRenderer = read("src/components/MarkdownRenderer.vue");
    const assetChip = read("src/components/AssetChip.vue");

    expect(chatView).toContain('class="chat-view-layout"');
    expect(chatTranscript).toMatch(/class="[^"]*\bchat-transcript-plain-text\b[^"]*\bui-select-text\b/);
    expect(chatComposer).toMatch(/class="[^"]*\bchat-composer-action\b[^"]*\bui-select-none\b/);

    expect(terminal).toMatch(/class="[^"]*\bterm-cmd-text\b[^"]*\bui-select-text\b/);
    expect(terminal).toMatch(/class="[^"]*\btool-sum\b[^"]*\bui-select-text\b/);
    expect(terminal).toMatch(/class="[^"]*\bterm-prompt\b[^"]*\bui-select-none\b/);
    expect(terminal).toMatch(/class="[^"]*\bterm-cancel-inline\b[^"]*\bui-select-none\b/);

    expect(commitDetail).toMatch(/class="[^"]*\bcommit-detail-subject\b[^"]*\bui-select-text\b/);
    expect(commitDetail).toMatch(/class="[^"]*\bcommit-detail-hash-text\b[^"]*\bui-select-text\b/);
    expect(commitDetail).toMatch(/class="[^"]*\bcommit-detail-body\b[^"]*\bui-select-text\b/);
    expect(commitDetail).toMatch(/class="[^"]*\bfile-name\b[^"]*\bui-select-text\b/);
    expect(commitDetail).toMatch(/class="[^"]*\bfile-dir\b[^"]*\bui-select-text\b/);

    expect(gitSidebar).toMatch(/class="[^"]*\bsidebar-item\b[^"]*\bui-select-none\b"[\s\S]*'stash-item': true/);
    expect(markdownRenderer).toContain('class="markdown-body ui-select-text"');
    expect(markdownRenderer).toMatch(/\.md-asset-chip\s*\{[\s\S]*?user-select:\s*none;/);
    expect(markdownRenderer).toMatch(/\.md-file-ref\s*\{[\s\S]*?user-select:\s*none;/);
    expect(markdownRenderer).toMatch(/\.md-workspace-ref\s*\{[\s\S]*?user-select:\s*none;/);
    expect(markdownRenderer).toContain(".markdown-body :is(.md-asset-chip, .md-file-ref, .md-workspace-ref)");
    expect(assetChip).toMatch(/\.asset-chip\s*\{[\s\S]*?user-select:\s*none;/);
  });
});
