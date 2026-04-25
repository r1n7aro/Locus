import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const cwd = process.cwd();

function read(relPath: string) {
  return readFileSync(resolve(cwd, relPath), "utf8");
}

describe("tool confirm layout", () => {
  it("renders single-card confirmation for one tool and batch card for multiple tools", () => {
    const chatView = read("src/components/ChatView.vue");
    const embeddedPane = read("src/components/chat/EmbeddedChatPane.vue");

    expect(chatView).toContain('v-if="showBatchToolConfirmCard"');
    expect(chatView).toContain('v-else-if="showSingleToolConfirmCard"');
    expect(chatView).toContain("<ToolConfirmCard");

    expect(embeddedPane).toContain('v-if="showBatchToolConfirmCard"');
    expect(embeddedPane).toContain('v-else-if="showSingleToolConfirmCard"');
    expect(embeddedPane).toContain("<ToolConfirmCard");
  });

  it("uses the neutral Unity status confirmation treatment", () => {
    const card = read("src/components/chat/ToolConfirmCard.vue");
    const labels = read("src/components/chat/toolConfirmLabels.ts");
    const zh = read("src/language/zh.json");

    expect(card).toContain("is-unity-status-change");
    expect(card).toContain("unity-status-change-details");
    expect(card).toContain("titleForUnityEditorStatusChange");
    expect(labels).toContain("titleForUnityEditorStatusChange");
    expect(zh).toContain('"chat.toolConfirm.unityStatus.title.playing": "请求进入运行状态"');
  });
});
