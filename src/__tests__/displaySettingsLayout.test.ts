import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const cwd = process.cwd();

function read(relPath: string) {
  return readFileSync(resolve(cwd, relPath), "utf8");
}

describe("display settings transcript alignment", () => {
  it("adds a session user message right-align toggle that defaults to on", () => {
    const displaySettings = read("src/composables/useDisplaySettings.ts");
    const displayPanel = read("src/components/settings/DisplaySettings.vue");
    const transcript = read("src/components/chat/ChatTranscript.vue");
    const zh = read("src/language/zh.json");
    const en = read("src/language/en.json");

    expect(displaySettings).toContain("rightAlignUserMessages: boolean;");
    expect(displaySettings).toContain("rightAlignUserMessages: true,");

    expect(displayPanel).toContain(":model-value=\"display.rightAlignUserMessages\"");
    expect(displayPanel).toContain(":aria-label=\"t('settings.display.rightAlignUserMessages')\"");
    expect(displayPanel).toContain("@update:model-value=\"setDisplay('rightAlignUserMessages', $event)\"");
    expect(displayPanel).toContain("{{ t(\"settings.display.rightAlignUserMessages\") }}");

    expect(transcript).toContain("const { state: displaySettings } = useDisplaySettings();");
    expect(transcript).toContain("function shouldRightAlignUserMessageGroup(group: Pick<MessageGroup, \"role\">) {");
    expect(transcript).toContain("'user-align-right': shouldRightAlignUserMessageGroup(group),");
    expect(transcript).toContain(".chat-transcript-message.is-session.user.user-align-right .chat-transcript-message-role.is-session {");
    expect(transcript).toContain(".chat-transcript-message.is-session.user.user-align-right .chat-transcript-message-content.is-session {");
    expect(transcript).toContain(".chat-transcript-message.is-session.user.user-align-right .chat-transcript-item-stack.is-session {");
    expect(transcript).toContain(".chat-transcript-message.is-session.user.user-align-right .chat-transcript-plain-text {");

    expect(zh).toContain('"settings.display.rightAlignUserMessages": "会话窗口中将用户消息右对齐"');
    expect(en).toContain('"settings.display.rightAlignUserMessages": "Right-align user messages in the session view"');
  });
});
