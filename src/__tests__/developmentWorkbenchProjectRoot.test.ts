import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";
import {
  UNITY_PROJECT_ICON,
  projectIconForServices,
} from "../components/icons/projectIcons";

const workbench = readFileSync(
  resolve(process.cwd(), "src/components/workbench/DevelopmentWorkbench.vue"),
  "utf8",
);
const workspaceTree = readFileSync(
  resolve(process.cwd(), "src/components/explorer/WorkspaceTree.vue"),
  "utf8",
);

describe("Development workbench project roots", () => {
  it("only toggles the project tree branch when a project root is activated", () => {
    const projectBranch = workbench.match(
      /if \(item\.meta\.kind === "project"\) \{\s*toggleItem\(item\);\s*return;\s*\}/,
    )?.[0] ?? "";

    expect(projectBranch).toContain("toggleItem(item);");
    expect(projectBranch).toContain("return;");
    expect(projectBranch).not.toContain("activeResource.value");
    expect(projectBranch).not.toContain("selectedNodeKey");
  });

  it("offers a persistent delete action from project root context menus", () => {
    expect(workbench).toContain('@click="removeContextWorkspace"');
    expect(workbench).toContain('t("development.deleteWorkspace")');
    expect(workbench).toContain('t("development.deleteWorkspaceConfirm", projectLabel(project))');
    expect(workbench).toContain("workspaceContextBaseStore.removeProject(project.projectId)");
    expect(workbench).toContain("projectStore.loadRecentDirs()");
  });

  it("uses the Unity mark for projects with the Unity service detected", () => {
    expect(workbench).toContain("projectIconForServices(project?.detectedServices ?? [])");
    expect(workbench).toContain("if (projectIcon) return projectIcon;");
    expect(projectIconForServices(["unity"])).toBe(UNITY_PROJECT_ICON);
    expect(UNITY_PROJECT_ICON).toHaveLength(1);
    expect(UNITY_PROJECT_ICON[0]?.[0]).toBe("path");
  });

  it("projects embedded knowledge as one sortable system entry", () => {
    expect(workbench).toContain('const NEW_SESSION_SYSTEM_RESOURCE_ID = "newSession";');
    expect(workbench).toContain('const KNOWLEDGE_SYSTEM_RESOURCE_ID = "knowledge";');
    expect(workbench).toContain('node.resourceKind === SYSTEM_RESOURCE_KIND');
    expect(workbench).toContain('node.resourceId === KNOWLEDGE_SYSTEM_RESOURCE_ID');
    expect(workbench).toContain('const key = `knowledge-root:${project.projectId}`;');
    expect(workbench).toContain("dragEnabled: true");
    expect(workbench).toContain("explorerNode: node");
    expect(workbench).toContain('kind: "knowledgeRoot"');
    expect(workbench).toContain('case "knowledgeRoot": return BookOpen;');
    expect(workbench).toContain("editor.resource.kind === 'knowledgeRoot'");
    expect(workbench).toContain("<KnowledgeView");
  });

  it("places the new-session action inside the sortable workspace model", () => {
    expect(workbench).toContain('node.resourceId === NEW_SESSION_SYSTEM_RESOURCE_ID');
    expect(workbench).toContain('const key = `new-session:${project.projectId}`;');
    expect(workbench).toContain('kind: "newSession"');
    expect(workbench).not.toContain("const newSessionKey");
  });

  it("does not render legacy embedded knowledge folders or document placements", () => {
    expect(workbench).toContain('node.resourceKind === "knowledge"');
    expect(workbench).toContain('node.nodeId.startsWith("knowledge-type:")');
    expect(workbench).toContain('node.nodeId.startsWith("knowledge-path:")');
    expect(workbench).not.toContain("initializedKnowledgeExpansionProjects");
  });

  it("inherits the legacy KnowledgeExplorer tree density with restrained UI typography", () => {
    expect(workbench).toContain(':base-indent="12"');
    expect(workbench).toContain(':indent-size="14"');
    expect(workbench).toContain(':stroke-width="2"');
    expect(workbench).toMatch(/\.development-tree :deep\(\.workspace-tree-row\)\s*\{\s*gap:\s*6px;/);
    expect(workbench).toContain('"kx-folder": !isPackage');
    expect(workbench).toContain('"kx-leaf": true');
    expect(workbench).toContain('"is-open": selected');
    expect(workbench).toContain("background: color-mix(in srgb, var(--panel-bg) 88%, var(--bg-color) 12%);");
    expect(workbench).not.toMatch(/\.development-tree :deep\(\.workspace-tree-name\)[\s\S]*font-family/);
    expect(workspaceTree).toContain("font-family: var(--font-ui);");
    expect(workspaceTree).toContain("color: color-mix(in srgb, var(--text-color) 78%, var(--text-secondary) 22%);");
  });

  it("keeps empty folders expandable and renders a disabled child row", () => {
    expect(workbench).toContain("expandable: true");
    expect(workbench).toContain('t("development.emptyFolder")');
    expect(workbench).toContain('classes: { "is-empty-folder-row": true }');
    expect(workbench).toContain("disabled: true");
    expect(workbench).toContain('kind: "empty"');
    expect(workbench).toContain("dropParentNodeId: node.nodeId");
  });
});
