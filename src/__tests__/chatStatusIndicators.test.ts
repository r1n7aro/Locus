import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const cwd = process.cwd();

function read(relPath: string) {
  return readFileSync(resolve(cwd, relPath), "utf8");
}

describe("chat status indicators", () => {
  it("moves Unity and asset database status onto the input backdrop", () => {
    const chatView = read("src/components/ChatView.vue");
    const sessionPanel = read("src/components/chat/SessionPanel.vue");
    const indicators = read("src/components/chat/ChatStatusIndicators.vue");

    expect(chatView).toContain('import ChatStatusIndicators from "./chat/ChatStatusIndicators.vue"');
    expect(chatView).toMatch(/<div v-if="!inputControlsCollapsed" class="input-backdrop-status">[\s\S]*<ChatStatusIndicators/);
    expect(chatView).toMatch(/<template v-if="!inputControlsCollapsed" #footer-start>[\s\S]*<ModelEffortSelector[\s\S]*\/>\s*<TokenUsageBar/);
    expect(chatView).toContain('@start-scan="emit(\'startScan\')"');
    expect(sessionPanel).not.toContain("sp-unity-status");
    expect(sessionPanel).not.toContain("sp-scan-status");
    expect(indicators).toContain('id: "assetDb"');
    expect(indicators).toContain('id: "unity"');
  });

  it("uses fixed icon triggers with top hover labels and click popovers", () => {
    const indicators = read("src/components/chat/ChatStatusIndicators.vue");

    expect(indicators).toContain('icon: "database"');
    expect(indicators).toContain('icon: "unity"');
    expect(indicators).toContain('class="chat-status-icon-btn ui-select-none"');
    expect(indicators).toContain('class="chat-status-icon-label"');
    expect(indicators).toContain("{{ item.inlineLabel }}");
    expect(indicators).toContain('bottom: calc(100% + 6px);');
    expect(indicators).toContain('left: 50%;');
    expect(indicators).toContain('transform: translate(-50%, 3px);');
    expect(indicators).toContain('color: currentColor;');
    expect(indicators).toContain('width: 24px;');
    expect(indicators).toContain(':aria-label="`${item.title}: ${item.summary}`"');
    expect(indicators).toContain('class="chat-status-popover"');
    expect(indicators).toContain('role="dialog"');
    expect(indicators).toContain("tone-danger");
    expect(indicators).toContain("var(--status-danger-fg)");
  });
});
