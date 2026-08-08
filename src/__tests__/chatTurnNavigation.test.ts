import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";
import type { ChatMessage } from "../types";
import {
  buildChatTurnNavigationItems,
  findActiveChatTurnIds,
} from "../components/chat/chatTurnNavigation";

function message(
  id: string,
  role: ChatMessage["role"],
  content: string,
): ChatMessage {
  return { id, role, content, createdAt: 0 };
}

describe("chat turn navigation", () => {
  it("keeps every user turn and pairs it with the first following response", () => {
    const items = buildChatTurnNavigationItems([
      message("u1", "user", "First prompt"),
      message("t1", "tool", "tool output"),
      message("a1", "assistant", "First response"),
      message("a2", "assistant", "Later response"),
      message("u2", "user", "Second prompt"),
      message("a3", "assistant", "Second response"),
      message("u3", "user", "Third prompt"),
    ]);

    expect(items).toEqual([
      { id: "u1", prompt: "First prompt", response: "First response" },
      { id: "u2", prompt: "Second prompt", response: "Second response" },
      { id: "u3", prompt: "Third prompt", response: "" },
    ]);
  });

  it("marks every turn range intersecting the viewport", () => {
    const anchors = [
      { id: "u1", start: 0 },
      { id: "u2", start: 100 },
      { id: "u3", start: 200 },
    ];

    expect(findActiveChatTurnIds(anchors, 100, 200, 300)).toEqual(["u2"]);
    expect(findActiveChatTurnIds(anchors, 80, 220, 300)).toEqual(["u1", "u2", "u3"]);
    expect(findActiveChatTurnIds(anchors, 340, 400, 400)).toEqual(["u3"]);
  });

  it("integrates a default-on setting and preserves the Codex rail interactions", () => {
    const rail = readFileSync(
      resolve(process.cwd(), "src/components/chat/ChatTurnNavigationRail.vue"),
      "utf8",
    );
    const chatView = readFileSync(resolve(process.cwd(), "src/components/ChatView.vue"), "utf8");
    const display = readFileSync(
      resolve(process.cwd(), "src/composables/useDisplaySettings.ts"),
      "utf8",
    );
    const displayView = readFileSync(
      resolve(process.cwd(), "src/components/settings/DisplaySettings.vue"),
      "utf8",
    );

    expect(display).toContain("showTurnNavigationRail: true");
    expect(displayView).toContain("display.showTurnNavigationRail");
    expect(chatView).toContain('v-if="displaySettings.showTurnNavigationRail"');
    expect(chatView).toContain(":scroll-element=\"transcriptScrollElement\"");
    expect(rail).toContain("visibleItems.value.length > 0");
    expect(rail).toContain("const MIN_LEFT_GUTTER = 48");
    expect(rail).toContain("target.scrollIntoView");
    expect(rail).toContain("PREVIEW_OPEN_DELAY_MS = 150");
    expect(rail).toContain("duration: 1400");
    expect(rail).toContain("0.2308 + 0.7692 * var(--marker-progress)");
    expect(rail).toContain("data-scrub-target");
    expect(rail).toMatch(/\.chat-turn-navigation-button\s*\{[\s\S]*cursor:\s*default;/);
  });
});
