import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const cwd = process.cwd();

function read(relPath: string) {
  return readFileSync(resolve(cwd, relPath), "utf8");
}

describe("display settings transcript alignment", () => {
  it("keeps main and Unity embed color styles separately configurable", () => {
    const theme = read("src/composables/useTheme.ts");
    const displayPanel = read("src/components/settings/DisplaySettings.vue");
    const settingsState = read("src/composables/useSettingsState.ts");
    const app = read("src/App.vue");
    const html = read("index.html");
    const zh = read("src/language/zh.json");
    const en = read("src/language/en.json");

    expect(theme).toContain('export type ThemeScope = "main" | "unityEmbed";');
    expect(theme).toContain('unityEmbed: "locus-unity-embed-theme-preference"');
    expect(theme).toContain('main: "dark"');
    expect(theme).toContain('unityEmbed: "dark"');
    expect(theme).toContain("unityEmbedPreference");
    expect(theme).toContain("setThemePreference(scope: ThemeScope, pref: ThemePreference)");

    expect(app).toContain('initTheme(isUnityEmbedWindow ? "unityEmbed" : "main")');
    expect(html).toContain("locus-unity-embed-theme-preference");
    expect(html).toContain("var fallback='dark';");

    expect(displayPanel).toContain("mainPreference");
    expect(displayPanel).toContain("unityEmbedPreference");
    expect(displayPanel).toContain("settings.display.themeMainWindow");
    expect(displayPanel).toContain("settings.display.themeUnityEmbedWindow");
    expect(displayPanel).toContain("setThemePreference('main', $event as ThemePreference)");
    expect(displayPanel).toContain("setThemePreference('unityEmbed', $event as ThemePreference)");
    expect(settingsState).toContain('setThemePreference("main", "dark");');
    expect(settingsState).toContain('setThemePreference("unityEmbed", "dark");');

    expect(zh).toContain('"settings.display.themeMainWindow": "主窗口"');
    expect(zh).toContain('"settings.display.themeUnityEmbedWindow": "Unity 嵌入窗口"');
    expect(en).toContain('"settings.display.themeMainWindow": "Main Window"');
    expect(en).toContain('"settings.display.themeUnityEmbedWindow": "Unity Embedded Window"');
  });

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

  it("adds a Git tree status icon merge toggle", () => {
    const displaySettings = read("src/composables/useDisplaySettings.ts");
    const displayPanel = read("src/components/settings/DisplaySettings.vue");
    const stagingArea = read("src/components/collab/StagingArea.vue");
    const commitDetail = read("src/components/collab/CommitDetail.vue");
    const collabStyles = read("src/components/collab/collabPreview.css");
    const zh = read("src/language/zh.json");
    const en = read("src/language/en.json");

    expect(displaySettings).toContain("mergeGitTreeStatusIcon: boolean;");
    expect(displaySettings).toContain("mergeGitTreeStatusIcon: true,");

    expect(displayPanel).toContain("settings.display.gitViewTitle");
    expect(displayPanel).toContain(":model-value=\"display.mergeGitTreeStatusIcon\"");
    expect(displayPanel).toContain(":aria-label=\"t('settings.display.mergeGitTreeStatusIcon')\"");
    expect(displayPanel).toContain("@update:model-value=\"setDisplay('mergeGitTreeStatusIcon', $event)\"");

    for (const component of [stagingArea, commitDetail]) {
      expect(component).toContain("const { state: displaySettings } = useDisplaySettings();");
      expect(component).toContain("displaySettings.mergeGitTreeStatusIcon");
      expect(component).toContain("fileTreeIconClasses(row.file)");
      expect(component).toContain("staging-tree-status-spacer");
    }

    expect(collabStyles).toContain(".staging-tree-file-icon.is-git-status-icon.status-modified");
    expect(collabStyles).toContain("color: var(--git-status-modified);");
    expect(collabStyles).toContain("color: var(--git-status-added);");
    expect(collabStyles).toContain("color: var(--git-status-deleted);");

    expect(zh).toContain('"settings.display.mergeGitTreeStatusIcon": "层级视图用彩色图标显示修改状态"');
    expect(en).toContain('"settings.display.mergeGitTreeStatusIcon": "Use colored icons for Git tree status"');
  });
});
