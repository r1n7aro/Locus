import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const cwd = process.cwd();

function read(relPath: string) {
  return readFileSync(resolve(cwd, relPath), "utf8");
}

describe("chat input mode layout", () => {
  it("adds a send behavior dropdown to shortcut settings", () => {
    const source = read("src/components/settings/ShortcutSettings.vue");

    expect(source).toContain('import BaseDropdown from "../ui/BaseDropdown.vue"');
    expect(source).toContain('const { state: chatInputSettings, setSubmitMode, setRunningSendMode } = useChatInputSettings();');
    expect(source).toContain('<BaseDropdown');
    expect(source).toContain(':model-value="chatInputSettings.submitMode"');
    expect(source).toContain('@update:model-value="setSubmitMode($event as ChatSubmitMode)"');
    expect(source).toContain(':model-value="chatInputSettings.runningSendMode"');
    expect(source).toContain('@update:model-value="setRunningSendMode($event as RunningSendMode)"');
  });

  it("threads submit mode through the chat input stack", () => {
    const richInput = read("src/components/chat/RichChatInput.vue");
    const composer = read("src/components/chat/ChatComposer.vue");
    const chatView = read("src/components/ChatView.vue");

    expect(richInput).toContain("shouldSelectPopupOnEnter");
    expect(richInput).toContain("shouldSubmitOnEnter");
    expect(richInput).toContain(':submit-mode="chatInputSettings.submitMode"');
    expect(composer).toContain("submitMode?: ChatSubmitMode;");
    expect(composer).toContain("shouldSubmitOnEnter(event, props.submitMode)");
    expect(chatView).toContain("const chatInputPlaceholder = computed(() => {");
    expect(chatView).toContain('t("chat.input.placeholderModifierSend", getChatSubmitModifierLabel())');
  });

  it("defines localized send behavior labels", () => {
    const zh = read("src/language/zh.json");
    const en = read("src/language/en.json");

    expect(zh).toContain('"settings.shortcuts.sendModeTitle": "发送方式"');
    expect(zh).toContain('"settings.shortcuts.runningSendAfterRun": "排为下一条"');
    expect(zh).toContain('"settings.shortcuts.sendModeModifierSendHint": "Enter 换行"');
    expect(zh).toContain('"chat.input.placeholderModifierSend": "输入消息... (@ 引用资产、文件夹或知识, / 查看命令, {0}+Enter 发送)"');
    expect(en).toContain('"settings.shortcuts.sendModeTitle": "Send behavior"');
    expect(en).toContain('"settings.shortcuts.runningSendAfterRun": "Queue next"');
    expect(en).toContain('"settings.shortcuts.sendModeModifierSendHint": "Enter inserts newline"');
    expect(en).toContain('"chat.input.placeholderModifierSend": "Type message... (@ ref asset, folder, or knowledge, / commands, {0}+Enter to send)"');
  });
});
