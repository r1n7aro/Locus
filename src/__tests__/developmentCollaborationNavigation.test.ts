import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const cwd = process.cwd();

function read(relPath: string) {
  return readFileSync(resolve(cwd, relPath), "utf8");
}

describe("Development collaboration navigation", () => {
  it("derives Collaboration expansion from the active tree resource", () => {
    const workbench = read("src/components/workbench/DevelopmentWorkbench.vue");

    expect(workbench).toContain("function isCollaborationExpanded(projectId: string): boolean");
    expect(workbench).toContain('resource.kind === "checkout"');
    expect(workbench).toContain('resource.kind === "section" && resource.section === "collab"');
    expect(workbench).toContain("const collaborationExpanded = isCollaborationExpanded(project.projectId);");
    expect(workbench).toContain("expanded: collaborationExpanded");
    expect(workbench).toContain("if (collaborationExpanded)");
    expect(workbench).not.toContain("next.add(`collaboration:${project.projectId}`)");
  });

  it("requests Git Graph focus after a worktree is activated", () => {
    const workbench = read("src/components/workbench/DevelopmentWorkbench.vue");
    const collabView = read("src/components/CollabView.vue");

    expect(workbench).toContain("collabHeadFocusRequest.value = {");
    expect(workbench).toContain("checkoutId: item.meta.checkoutId");
    expect(workbench).toContain(':head-focus-request="collabHeadFocusRequest"');
    expect(collabView).toContain("props.headFocusRequest?.id");
    expect(collabView).toContain("requestedCheckoutId !== currentCheckoutId");
    expect(collabView).toContain("loadedCommits.some((commit) => commit.hash === headHash)");
    expect(collabView).toContain('{ kind: "commit", hash: headHash }');
    expect(collabView).toContain('{ scroll: true, behavior: "smooth" }');
  });
});
