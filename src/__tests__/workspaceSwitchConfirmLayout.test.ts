import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const cwd = process.cwd();

function read(relPath: string) {
  return readFileSync(resolve(cwd, relPath), "utf8");
}

describe("workspace switch flow", () => {
  it("focuses checkouts through the Development tree and keeps runs scoped", () => {
    const app = read("src/App.vue");
    const workbench = read("src/components/workbench/DevelopmentWorkbench.vue");
    const chatStore = read("src/stores/chat.ts");

    expect(app).toContain("const runningSessionCount = computed(() => chatStore.streamingSessionIds.size);");
    expect(app).not.toContain("switchingWorkspacePath");
    expect(app).not.toContain("workspace-switch-spinner");
    expect(app).not.toContain("performWorkingDirChange");
    expect(workbench).toContain("await workspaceContextStore.focusCheckout(item.meta.checkoutId);");
    expect(workbench).toContain("await workspaceContextStore.openAndFocus(selected);");
    expect(chatStore).toContain("const requestWorkspaceRef = captureFocusedWorkspaceRef();");
    expect(app).not.toContain("await chatStore.cancelSessions(sessionIds);");
    expect(chatStore).toContain("async function cancelSessions(sessionIds: string[]) {");
    expect(chatStore).toContain("await Promise.all(targets.map((sessionId) => cancelSession(sessionId)));");
  });
});
