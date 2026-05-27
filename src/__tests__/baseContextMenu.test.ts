import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const cwd = process.cwd();

function read(relPath: string) {
  return readFileSync(resolve(cwd, relPath), "utf8");
}

describe("BaseContextMenu", () => {
  it("centralizes positioning, fade-in, and menu item styling for right-click menus", () => {
    const component = read("src/components/ui/BaseContextMenu.vue");
    const app = read("src/App.vue");
    const chat = read("src/components/ChatView.vue");
    const collab = read("src/components/CollabView.vue");
    const sessionPanel = read("src/components/chat/SessionPanel.vue");
    const viewPackage = read("src/components/ViewPackageView.vue");
    const knowledgeExplorer = read("src/components/knowledge/KnowledgeExplorer.vue");
    const agent = read("src/components/AgentView.vue");

    expect(component).toContain("clampFloatingPosition");
    expect(component).toContain('name="base-context-menu-fade" appear');
    expect(component).toContain("@contextmenu.prevent=\"close\"");
    expect(component).toContain(".base-context-menu :deep(button)");
    expect(component).toContain(".base-context-menu-fade-enter-from .base-context-menu");

    for (const source of [app, chat, collab, sessionPanel, viewPackage, knowledgeExplorer, agent]) {
      expect(source).toContain("BaseContextMenu");
      expect(source).toContain("<BaseContextMenu");
    }
  });
});
