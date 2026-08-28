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

  it("restores the primary session actions from the legacy session tree", () => {
    const workbench = read("src/components/workbench/DevelopmentWorkbench.vue");
    const chatWorkspace = read("src/components/ChatWorkspaceView.vue");

    expect(workbench).toContain("contextOpenSessionWindow");
    expect(workbench).toContain("contextOpenSessionInUnity");
    expect(workbench).toContain("exportContextSession");
    expect(workbench).toContain("reviewContextSession");
    expect(workbench).toContain("archiveContextSession");
    expect(workbench).toContain("beginDeleteSession");
    expect(chatWorkspace).toContain("defineExpose({");
    expect(chatWorkspace).toContain("reviewSessionContext,");
  });

  it("places drops over empty folder rows and child rows inside the folder", () => {
    const workbench = read("src/components/workbench/DevelopmentWorkbench.vue");

    expect(workbench).toContain("dropParentNodeId?: string | null;");
    expect(workbench).toContain("dropParentNodeId: node.nodeId");
    expect(workbench).toContain('target.meta.kind === "empty" && target.meta.dropParentNodeId');
    expect(workbench).toContain("parentNodeId: target.meta.dropParentNodeId");
    expect(workbench).toContain("const parentNodeId = targetNode.parentNodeId ?? null;");
  });

  it("reveals an accessible archive action when a session row is hovered", () => {
    const workbench = read("src/components/workbench/DevelopmentWorkbench.vue");

    expect(workbench).toContain('"is-session-row": kind === "session"');
    expect(workbench).toContain('class="development-session-archive-button"');
    expect(workbench).toContain(':aria-label="t(\'chat.session.archive\')"');
    expect(workbench).toContain('@click.stop="archiveSessionItem(item as DevelopmentTreeItem)"');
    expect(workbench).toContain(".workspace-tree-row-shell.is-session-row:hover .development-session-archive-button");
    expect(workbench).toMatch(/\.development-session-archive-button\s*\{[\s\S]*?right:\s*14px;/);
    expect(workbench).toContain("await archiveSessionEntry(item.meta.projectId, item.meta.session);");
  });

  it("keeps the standard pointer cursor on draggable session rows", () => {
    const workbench = read("src/components/workbench/DevelopmentWorkbench.vue");

    expect(workbench).toMatch(
      /\.workspace-tree-row-shell\.is-session-row \.workspace-tree-row\.drag-enabled\)\s*\{\s*cursor:\s*pointer;/,
    );
  });
});
