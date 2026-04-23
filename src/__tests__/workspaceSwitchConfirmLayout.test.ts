import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const cwd = process.cwd();

function read(relPath: string) {
  return readFileSync(resolve(cwd, relPath), "utf8");
}

describe("workspace switch confirm flow", () => {
  it("confirms before switching away from a running workspace session and shows a cancel banner after the switch", () => {
    const app = read("src/App.vue");
    const chatStore = read("src/stores/chat.ts");
    const zh = read("src/language/zh.json");
    const en = read("src/language/en.json");

    expect(app).toContain("const pendingWorkspaceSwitchPath = ref<string | null>(null);");
    expect(app).toContain("const runningSessionCount = computed(() => chatStore.streamingSessionIds.size);");
    expect(app).toContain("await chatStore.cancelSessions(sessionIds);");
    expect(app).toContain("notifyCancelledWorkspaceSessions(cancelledSessionCount);");
    expect(app).toContain("operation: \"workspaceSwitchCancelled\"");
    expect(app).toContain("class=\"workspace-switch-overlay\"");
    expect(app).toContain("class=\"workspace-switch-dialog\"");
    expect(app).toContain('t("app.dir.runningConfirmAction")');
    expect(chatStore).toContain("async function cancelSessions(sessionIds: string[]) {");
    expect(chatStore).toContain("await Promise.all(targets.map((sessionId) => cancelSession(sessionId)));");
    expect(zh).toContain('"app.dir.runningConfirmTitle": "切换工作区"');
    expect(zh).toContain('"app.dir.runningCancelledNotice": "已取消 {0} 个进行中的会话"');
    expect(en).toContain('"app.dir.runningConfirmTitle": "Switch workspace"');
    expect(en).toContain('"app.dir.runningCancelledNotice": "Cancelled {0} running sessions"');
  });
});
