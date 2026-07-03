import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const cwd = process.cwd();

function read(relPath: string) {
  return readFileSync(resolve(cwd, relPath), "utf8");
}

describe("chat changes file context menu", () => {
  it("opens a context menu on file rows in both tree and list views", () => {
    const panel = read("src/components/ChatChangesPanel.vue");

    expect(panel).toContain('import BaseContextMenu from "./ui/BaseContextMenu.vue";');
    expect(panel).toContain(
      "const fileCtxMenu = ref<{ x: number; y: number; item: DisplayItem } | null>(null);",
    );
    expect(panel).toContain(
      '@contextmenu.prevent="onFileRowContextMenu($event, row.file.displayItem)"',
    );
    expect(panel).toContain('@contextmenu.prevent="onFileRowContextMenu($event, item)"');
    expect(panel).toContain('class="changes-ctx-menu"');
  });

  it("routes menu actions through the existing handlers", () => {
    const panel = read("src/components/ChatChangesPanel.vue");

    // Right-click revert reuses the same confirm flow as the inline button.
    expect(panel).toContain("function ctxRevertFile(ev: MouseEvent) {");
    expect(panel).toContain("if (item) onRevertFileClick(ev, item);");
    expect(panel).toContain("if (item) void onItemClick(item);");
    expect(panel).toContain("if (item) onSelectInUnity(ev, item.fileChange.path);");
    expect(panel).toContain("if (item) onOpenInEditor(ev, item.fileChange.path);");
    // Opening the menu dismisses the hover diff popover so they never overlap.
    expect(panel).toMatch(/function onFileRowContextMenu\(ev: MouseEvent, item: DisplayItem\) \{\n  clearHover\(\);/);
  });

  it("marks revert as destructive and hides it while streaming", () => {
    const panel = read("src/components/ChatChangesPanel.vue");

    expect(panel).toContain('class="changes-ctx-item danger"');
    expect(panel).toContain(':disabled="isRevertingFile"');
    const menuBlock = panel.slice(panel.indexOf('class="changes-ctx-menu"'));
    expect(menuBlock).toContain('<template v-if="!chatStore.isStreaming">');
  });

  it("keeps the menu labels translated in both locales", () => {
    const zh = read("src/language/zh.json");
    const en = read("src/language/en.json");

    expect(en).toContain('"chat.changes.viewDiff": "View diff"');
    expect(zh).toContain('"chat.changes.viewDiff": "查看差异"');
    expect(en).toContain('"chat.changes.fileMenu": "File actions"');
    expect(zh).toContain('"chat.changes.fileMenu": "文件操作"');
  });
});
