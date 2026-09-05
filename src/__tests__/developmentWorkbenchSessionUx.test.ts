import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const root = process.cwd();

function read(path: string): string {
  return readFileSync(resolve(root, path), "utf8");
}

describe("development workbench session experience", () => {
  it("keeps the project session catalog synchronized after chat mutations", () => {
    const workbench = read("src/components/workbench/DevelopmentWorkbench.vue");
    const explorerStore = read("src/stores/workspaceExplorer.ts");

    expect(workbench).toContain("explorerStore.refreshProjectSessions(projectId)");
    expect(explorerStore).toContain("async function refreshProjectSessions(projectId: string)");
    expect(explorerStore).toContain("await placeMissingResources(projectId, layoutEpoch)");
  });

  it("projects live runtime and pending-selection state into the workspace tree", () => {
    const workbench = read("src/components/workbench/DevelopmentWorkbench.vue");

    expect(workbench).toContain("sessionTreeStatusForSession");
    expect(workbench).toContain("maxSessionTreeStatus");
    expect(workbench).toContain('"is-session-pending": chatStore.pendingSelectionSessionId === session.id');
    expect(workbench).toContain("development-session-title-scan");
    expect(workbench).toContain("development-session-spinner");
    expect(workbench).toContain("!isAnimatedSessionStatus(itemRuntimeStatus(item as DevelopmentTreeItem))");
    expect(workbench).not.toContain("development-session-pulse");
  });

  it("keeps session selection exclusive with knowledge and collaboration", () => {
    const workbench = read("src/components/workbench/DevelopmentWorkbench.vue");

    expect(workbench).toContain("function isWorkspaceSessionSelected(projectId: string, sessionId: string)");
    expect(workbench).toContain("if (pendingSessionId) return pendingSessionId === sessionId;");
    expect(workbench.indexOf("if (pendingSessionId) return pendingSessionId === sessionId;")).toBeLessThan(
      workbench.indexOf("const resource = activeResource.value;"),
    );
    expect(workbench).toContain('resource.kind === "session"');
    expect(workbench).toContain("return chatStore.activeSessionId === sessionId;");
    expect(workbench).toContain("const selected = isWorkspaceSessionSelected(project.projectId, session.id);");
    expect(workbench).not.toContain("const selected = chatStore.activeSessionId === session.id");
    expect(workbench).not.toContain(".workspace-tree-row-shell.is-session-pending .workspace-tree-name");
  });

  it("wires ctrl and shift session multi-selection into batch context actions", () => {
    const workbench = read("src/components/workbench/DevelopmentWorkbench.vue");

    expect(workbench).toContain("resolveWorkspaceSessionSelection({");
    expect(workbench).toContain("visibleWorkspaceSessionTargets()");
    expect(workbench).toContain("selected: selected || multiSelected || contextSelected");
    expect(workbench).toContain("resolveWorkspaceSessionContextIds({");
    expect(workbench).toContain('t("chat.session.archiveMany", contextMenu.sessionTargets?.length ?? 0)');
    expect(workbench).toContain('t("chat.session.deleteMany", contextMenu.sessionTargets?.length ?? 0)');
    expect(workbench).toContain("for (const target of targets) await archiveSessionEntry(target);");
    expect(workbench).toContain("for (const target of dialog.targets)");
  });

  it("creates sibling folders from session menus and groups the actions", () => {
    const workbench = read("src/components/workbench/DevelopmentWorkbench.vue");
    const menuStart = workbench.indexOf(
      '<template v-else-if="contextMenu.item.meta.kind === \'session\'">',
    );
    const menuEnd = workbench.indexOf("\n      <template v-else>", menuStart);
    const sessionMenu = workbench.slice(menuStart, menuEnd);

    expect(menuStart).toBeGreaterThan(0);
    expect(menuEnd).toBeGreaterThan(menuStart);
    expect(sessionMenu).toContain('@click="beginCreateFolder"');
    expect(sessionMenu).toContain('t("development.newFolder")');
    expect(sessionMenu).toMatch(
      /contextOpenSessionInUnity[\s\S]*?base-context-menu-separator[\s\S]*?beginCreateFolder[\s\S]*?base-context-menu-separator[\s\S]*?exportContextSession[\s\S]*?reviewContextSession[\s\S]*?base-context-menu-separator[\s\S]*?archiveContextSession/,
    );
    expect(workbench).toContain('const parentNodeId = item.meta.kind === "folder"');
    expect(workbench).toContain(': item.meta.explorerNode?.parentNodeId ?? null;');
  });

  it("renames sessions in place inside the workspace tree", () => {
    const workbench = read("src/components/workbench/DevelopmentWorkbench.vue");

    expect(workbench).toContain("editing: sessionInlineRename.value?.sessionId === session.id");
    expect(workbench).toContain('class="development-session-rename-input"');
    expect(workbench).toContain('@keydown.enter.prevent="submitSessionRename"');
    expect(workbench).toContain('@keydown.esc.prevent.stop="cancelSessionRename"');
    expect(workbench).toContain('@blur="submitSessionRename"');
    expect(workbench).not.toContain("sessionDialog.mode === 'rename'");
  });

  it("places drops over empty folder rows and child rows inside the folder", () => {
    const workbench = read("src/components/workbench/DevelopmentWorkbench.vue");

    expect(workbench).toContain("dropParentNodeId?: string | null;");
    expect(workbench).toContain("dropParentNodeId: node.nodeId");
    expect(workbench).toContain('target.meta.kind === "empty" && target.meta.dropParentNodeId');
    expect(workbench).toContain("parentNodeId: target.meta.dropParentNodeId");
    expect(workbench).toContain("const parentNodeId = targetNode.parentNodeId ?? null;");
  });

  it("renders session children while keeping manual drops folder-based", () => {
    const workbench = read("src/components/workbench/DevelopmentWorkbench.vue");

    expect(workbench).toContain('candidate.parentNodeId === node.nodeId');
    expect(workbench).toContain('candidate.resourceKind === "session"');
    expect(workbench).toContain('target.meta.kind === "folder" && ratio >= 0.25 && ratio <= 0.75');
    expect(workbench).toContain('if (parentNode.nodeKind !== "folder") return false;');
    expect(workbench).not.toContain('allowSessionParent');
    expect(workbench).toContain('if (item.treeRow?.expandable) toggleItem(item);');
  });

  it("reveals an accessible archive action when a session row is hovered", () => {
    const workbench = read("src/components/workbench/DevelopmentWorkbench.vue");

    expect(workbench).toContain('"is-session-row": kind === "session"');
    expect(workbench).toContain('class="development-session-archive-button"');
    expect(workbench).toContain(':aria-label="t(\'chat.session.archive\')"');
    expect(workbench).toContain('@click.stop="archiveSessionItem(item as DevelopmentTreeItem)"');
    expect(workbench).toContain(".workspace-tree-row-shell.is-session-row:hover .development-session-archive-button");
    expect(workbench).toMatch(/\.development-session-archive-button\s*\{[\s\S]*?right:\s*14px;/);
    expect(workbench).toContain("await archiveSessionEntry({");
    expect(workbench).toContain(".workspace-tree-row-shell.is-session-row:hover .development-branch-label");
    expect(workbench).toContain(":has(.development-session-archive-button:focus-visible) .development-branch-label");
    expect(workbench).not.toContain(".workspace-tree-row-shell.is-session-row:focus-within .development-branch-label");
  });

  it("keeps the default cursor across workspace tree rows", () => {
    const workbench = read("src/components/workbench/DevelopmentWorkbench.vue");

    expect(workbench).toMatch(
      /\.development-tree :deep\(\.workspace-tree-row\)\s*\{[^}]*cursor:\s*default;/s,
    );
  });
});
