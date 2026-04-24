import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const cwd = process.cwd();

function read(relPath: string) {
  return readFileSync(resolve(cwd, relPath), "utf8");
}

describe("chat status indicators", () => {
  it("moves Unity and asset database status into the composer footer", () => {
    const chatView = read("src/components/ChatView.vue");
    const sessionPanel = read("src/components/chat/SessionPanel.vue");
    const indicators = read("src/components/chat/ChatStatusIndicators.vue");

    expect(chatView).toContain('import ChatStatusIndicators from "./chat/ChatStatusIndicators.vue"');
    expect(chatView).toMatch(/<template #footer>[\s\S]*<ChatStatusIndicators[\s\S]*<div class="footer-spacer"><\/div>[\s\S]*<TokenUsageBar/);
    expect(chatView).toContain('@start-scan="emit(\'startScan\')"');
    expect(sessionPanel).not.toContain("sp-unity-status");
    expect(sessionPanel).not.toContain("sp-scan-status");
    expect(indicators).toContain('id: "assetDb"');
    expect(indicators).toContain('id: "unity"');
  });

  it("uses icon-only triggers with hover titles and click popovers", () => {
    const indicators = read("src/components/chat/ChatStatusIndicators.vue");

    expect(indicators).toContain('icon: "database"');
    expect(indicators).toContain('icon: "unity"');
    expect(indicators).toContain('class="chat-status-icon-btn ui-select-none"');
    expect(indicators).toContain(':title="item.summary"');
    expect(indicators).toContain('class="chat-status-popover"');
    expect(indicators).toContain('role="dialog"');
    expect(indicators).toContain("tone-danger");
    expect(indicators).toContain("var(--status-danger-fg)");
  });
});
